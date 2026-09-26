//! NPC naval (MF-044/045, MF-059): um mar vivo. Piratas rondam a ilha e os
//! corredores, caravanas mercantes fazem a Rota da Costa entre os portos e a
//! marinha patrulha as águas da coroa. Nenhum NPC dá item útil (Pilar 1):
//! pirata e caravana afundados deixam só recurso bruto boiando (madeira e
//! minério), exclusivo de quem afundou.

use std::collections::HashMap;

use bevy::ecs::prelude::*;
use bevy::prelude::*;
use lightyear::prelude::server::*;
use lightyear::prelude::*;
use marvyr_domain_combat::elite::{self, EliteAffix};
use marvyr_domain_combat::{
    apply_damage, BroadsideBattery, BroadsideSide, DamageOutcome, Projectile, WeaponParams,
};
use marvyr_domain_items::{CargoHold, ItemCatalog};
use marvyr_domain_ships::{
    compute_ship_stats, step_motion, EquippedComponents, MotionInput, MotionTuning, ShipKind,
    ShipMotion, ShipStats, VesselPresence,
};
use marvyr_domain_world::{RiskTier, WorldMap};
use marvyr_protocol::{Faction, ShipState, WorldEventKind};
use marvyr_shared::ids::{CharacterId, ShipInstanceId, ZoneId};
use tracing::info;

use crate::crafting::DevShips;
use crate::net::{
    ground_on_land, CombatTuning, DevItems, ProjectileIdCounter, ServerProjectile,
    ServerRiskPolicy, ServerShip, ServerWorldMap,
};
use crate::reputation::{notoriety_gain, Offense, Reputation};

/// MV-061: mordida do Kraken (dano ao casco por golpe) e alcance dos
/// tentáculos (m). O monstro não tem canhão: precisa encostar.
const KRAKEN_BITE: u32 = 22;
const KRAKEN_REACH: f32 = 42.0;

/// Alcance máximo de canhão de NPC: o do casco inicial do jogador.
fn npc_max_range(dev_ships: &DevShips) -> f32 {
    dev_ships.merchant.base_weapon_range
}
/// Segundos entre dois golpes do Kraken.
const KRAKEN_BITE_SECS: f32 = 1.6;
/// v34: Leviatã — casco de chefe, mordida mais funda, alcance maior.
pub const LEVIATHAN_HP: u32 = 4_000;
const LEVIATHAN_BITE: u32 = 30;
const LEVIATHAN_REACH: f32 = 55.0;
/// Distância lateral (m) da escolta ao galeão.
const ESCORT_OFFSET: f32 = 70.0;

/// Raio padrão de patrulha ao redor do ponto de spawn.
const PATROL_RADIUS: f32 = 120.0;
/// Separador de ids: NPCs não compartilham o espaço de `ShipIdCounter` para
/// que `Projectile.owner_ship_id` não seja ambíguo.
const NPC_ID_OFFSET: u32 = 1_000_000;
/// Caravana considera o waypoint alcançado dentro deste raio (m).
const WAYPOINT_RADIUS: f32 = 45.0;
/// Sonda de terra à frente da proa (m) e abertura das sondas laterais.
const LAND_PROBE: f32 = 80.0;
const LAND_PROBE_SPREAD: f32 = 40.0 * std::f32::consts::PI / 180.0;

/// Papel do NPC no mar. A facção e o casco derivam dele.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum NpcRole {
    Pirate,
    Navy,
    /// Caravana mercante; `reverse` = Mina -> Serra.
    Caravan {
        reverse: bool,
    },
    /// MV-061: evento Kraken — monstro que ataca corpo a corpo.
    Kraken,
    /// MV-061: evento Frota do Tesouro — galeão carregado de recurso bruto.
    TreasureGalleon,
    /// MV-061: escolta da coroa colada no galeão.
    Escort,
    /// v26: corsário que guarda a ilha de um mapa Guardado. Não respawna;
    /// some sozinho depois de um tempo (`seafaring::MapPerils`).
    Guardian,
    /// v32: Saqueador da Maré Sangrenta — sempre elite, chega em ondas e
    /// afunda com a maré (`blood_tide`).
    Reaver,
    /// v34: Leviatã, o chefe de mundo agendado (`world_boss`): casco
    /// enorme, morde como o Kraken, paga cada capitão que lutou.
    Leviathan,
}

impl NpcRole {
    pub fn faction(self) -> Faction {
        match self {
            Self::Pirate | Self::Guardian | Self::Reaver => Faction::Pirate,
            Self::Navy | Self::Escort => Faction::Navy,
            Self::Caravan { .. } | Self::TreasureGalleon => Faction::Merchant,
            Self::Kraken | Self::Leviathan => Faction::Monster,
        }
    }

    /// Monstro: morde de perto, não se aborda, caça qualquer casco fora
    /// da coroa.
    pub fn is_monster(self) -> bool {
        self.faction() == Faction::Monster
    }

    /// Mercante NPC: foge quando atacado, suja o nome de quem ataca e deixa
    /// a carga boiando para quem afunda.
    pub fn is_merchant(self) -> bool {
        matches!(self, Self::Caravan { .. } | Self::TreasureGalleon)
    }

    /// NPC de evento de mundo: nunca respawna, some quando o evento acaba.
    pub fn is_event_npc(self) -> bool {
        matches!(
            self,
            Self::Kraken
                | Self::TreasureGalleon
                | Self::Escort
                | Self::Guardian
                | Self::Reaver
                | Self::Leviathan
        )
    }

    pub fn kind(self) -> ShipKind {
        match self {
            Self::Pirate | Self::Kraken | Self::Guardian | Self::Reaver | Self::Leviathan => {
                ShipKind::Corsair
            }
            Self::Navy | Self::Escort | Self::TreasureGalleon => ShipKind::Patrol,
            Self::Caravan { .. } => ShipKind::SmallMerchant,
        }
    }

    /// Renome de quem afunda (MV-067): quanto mais perigoso, mais rende.
    pub fn renown(self) -> u32 {
        match self {
            Self::Caravan { .. } => 20,
            Self::Pirate | Self::Navy | Self::Guardian => 35,
            Self::Escort | Self::Reaver => 50,
            Self::TreasureGalleon => 150,
            Self::Kraken => 250,
            Self::Leviathan => 400,
        }
    }

    fn label(self) -> &'static str {
        match self {
            Self::Pirate => "Corsario",
            Self::Guardian => "Guardiao do Tesouro",
            Self::Reaver => "Saqueador da Mare",
            Self::Leviathan => "Leviata",
            Self::Navy | Self::Escort => "navio da Marinha",
            Self::Caravan { .. } => "Mercador",
            Self::Kraken => "Kraken",
            Self::TreasureGalleon => "Galeao do Tesouro",
        }
    }
}

#[derive(Component)]
pub struct NpcShip {
    pub ship_id: u32,
    pub kind: ShipKind,
    pub role: NpcRole,
    pub hp: u32,
    pub max_hp: u32,
    /// NPCs nunca atracam.
    pub presence: VesselPresence,
    /// NPCs nunca carregam item; mantido para shape de `ShipState`.
    pub hold: CargoHold,
    pub battery: BroadsideBattery,
    pub stats: ShipStats,
    pub motion: ShipMotion,
    pub tuning: MotionTuning,
    pub zone: Option<ZoneId>,
    pub ai: NpcAi,
    /// MF-045: personagem do jogador que deu o golpe final.
    pub last_damage_dealer: Option<CharacterId>,
    /// Alvo atual enquanto Chase/Attack (Attack não guarda o id no estado).
    pub last_target: Option<u32>,
    /// v31: afixos de elite (bitmask de `EliteAffix`; 0 = comum).
    pub elite: u8,
    /// Fração de casco regenerado ainda não inteira (Regenerante).
    pub regen_carry: f32,
    /// Desvio de encalhe em curso (ver `unstick`).
    pub detour: Detour,
}

/// Desvio de encalhe: lado do giro e segundos até voltar a mirar o alvo.
#[derive(Debug, Clone, Copy, Default, PartialEq)]
pub struct Detour {
    /// Rumo para longe da terra, do último encalhe.
    away: f32,
    secs: f32,
}

impl Detour {
    fn start(&mut self, away: f32) {
        self.away = away;
        self.secs = DETOUR_SECS;
    }
}

#[derive(Debug, Clone, PartialEq)]
pub enum NpcState {
    Idle,
    Patrol {
        origin: (f32, f32),
        radius: f32,
    },
    /// Caravana seguindo a rota rumo a `NpcAi::route[next_waypoint]`.
    Travel,
    /// Caravana atacada: pano cheio para longe do agressor, em zigue-zague.
    Flee {
        from: u32,
        secs: f32,
    },
    Chase {
        target: u32,
    },
    Attack,
    /// MV-061: escolta mantendo posição ao lado do líder.
    Escort {
        leader: u32,
    },
    Dead,
}

#[derive(Debug, Clone)]
pub struct NpcAi {
    pub state: NpcState,
    pub detection_radius: f32,
    /// Distância em que a caça é abandonada.
    pub leash_radius: f32,
    pub weapon_range: f32,
    pub respawn_after_secs: f32,
    /// Recurso bruto (unidades) que o casco deixa ao afundar.
    pub spoils: u32,
    pub spawn_position: (f32, f32),
    /// Waypoints da caravana (vazio para os demais).
    pub route: Vec<(f32, f32)>,
    pub next_waypoint: usize,
    /// MV-061: galeão que esta escolta protege (0 = nenhum).
    pub escort_leader: u32,
}

#[derive(Resource, Default)]
pub struct NpcIdCounter(pub u32);

/// Povoamento e tuning do mar.
#[derive(Resource, Debug, Clone)]
pub struct NpcSpawnConfig {
    /// Corsários da ilha.
    pub count: usize,
    pub spawn_positions: Vec<(f32, f32)>,
    pub respawn_after_secs: f32,
    /// Piratas que rondam os corredores de fronteira (a Rota da Costa, onde
    /// fica a primeira coleta, não tem saqueador NPC).
    pub raider_positions: Vec<(f32, f32)>,
    pub navy_positions: Vec<(f32, f32)>,
    pub navy_respawn_secs: f32,
    pub caravan_count: usize,
    /// Rota da Costa, Serra -> Mina.
    pub caravan_route: Vec<(f32, f32)>,
    /// Mina -> Serra, pelos portões do sentido de volta.
    pub caravan_return: Vec<(f32, f32)>,
    pub caravan_respawn_secs: f32,
    /// Carga da caravana (unidades de recurso bruto) que fica boiando.
    pub caravan_spoils: u32,
    pub caravan_flee_secs: f32,
    /// Marinha a esta distância de uma caravana atacada responde.
    pub navy_response_radius: f32,
    /// MV-061: carga do galeão da Frota do Tesouro (recurso bruto).
    pub fleet_spoils: u32,
}

impl Default for NpcSpawnConfig {
    /// Tuning padrão com os pontos do mapa clássico.
    fn default() -> Self {
        Self::for_map(&WorldMap::vertical_slice())
    }
}

impl NpcSpawnConfig {
    /// Pontos de spawn e rota vêm do mapa (MV-065); o resto é tuning.
    pub fn for_map(map: &WorldMap) -> Self {
        let features = map.features();
        Self {
            // MV-067: um pirata por ponto do mapa (zonas sem lei inteiras).
            count: features.pirate_spawns.len(),
            // Águas da Ilha do Coral Negro: lawless e longe dos portos.
            spawn_positions: features.pirate_spawns.clone(),
            respawn_after_secs: 30.0,
            // Nos corredores de fronteira, longe da primeira coleta.
            raider_positions: features.raider_spawns.clone(),
            // Beira das águas protegidas, patrulhando para a fronteira.
            navy_positions: features.navy_spawns.clone(),
            navy_respawn_secs: 90.0,
            caravan_count: 3,
            caravan_route: features.caravan_route.clone(),
            caravan_return: features.caravan_return.clone(),
            caravan_respawn_secs: 25.0,
            caravan_spoils: 16,
            caravan_flee_secs: 12.0,
            navy_response_radius: 900.0,
            fleet_spoils: 60,
        }
    }

    fn route(&self, reverse: bool) -> Vec<(f32, f32)> {
        if reverse {
            self.caravan_return.clone()
        } else {
            self.caravan_route.clone()
        }
    }

    /// Onde um papel (re)nasce: caravana sempre no porto de partida.
    fn home(&self, role: NpcRole, fallback: (f32, f32)) -> (f32, f32) {
        match role {
            NpcRole::Caravan { reverse } => {
                self.route(reverse).first().copied().unwrap_or(fallback)
            }
            _ => fallback,
        }
    }

    fn respawn_secs(&self, role: NpcRole) -> f32 {
        match role {
            NpcRole::Pirate => self.respawn_after_secs,
            NpcRole::Navy => self.navy_respawn_secs,
            NpcRole::Caravan { .. } => self.caravan_respawn_secs,
            // Evento não volta: o próximo evento é do diretor.
            NpcRole::Kraken
            | NpcRole::TreasureGalleon
            | NpcRole::Escort
            | NpcRole::Guardian
            | NpcRole::Reaver
            | NpcRole::Leviathan => f32::INFINITY,
        }
    }
}

/// NPCs mortos (ou caravanas que atracaram) aguardando respawn:
/// (tempo restante, papel, posição de nascimento).
#[derive(Resource, Default)]
pub struct NpcRespawnQueue(pub Vec<(f32, NpcRole, (f32, f32))>);

pub fn setup_npcs(
    mut commands: Commands,
    dev_ships: Res<DevShips>,
    map: Res<ServerWorldMap>,
    config: Res<NpcSpawnConfig>,
    mut ids: ResMut<NpcIdCounter>,
) {
    let mut roster: Vec<(NpcRole, (f32, f32))> = Vec::new();
    for position in config.spawn_positions.iter().take(config.count) {
        roster.push((NpcRole::Pirate, *position));
    }
    for position in &config.raider_positions {
        roster.push((NpcRole::Pirate, *position));
    }
    for position in &config.navy_positions {
        roster.push((NpcRole::Navy, *position));
    }
    for (role, position) in roster {
        let (ship_id, ship) = build_npc(&dev_ships, &map.0, &config, &mut ids, role, position);
        let elite = ship.elite;
        commands.spawn((ship,));
        info!(
            ship_id,
            ?role,
            elite,
            x = position.0,
            y = position.1,
            "NPC no mar"
        );
    }
    // Caravanas escalonadas: metade em cada sentido, a partir de trechos
    // diferentes, para a rota nunca amanhecer vazia.
    for i in 0..config.caravan_count {
        let role = NpcRole::Caravan {
            reverse: i % 2 == 1,
        };
        let (ship_id, mut ship) =
            build_npc(&dev_ships, &map.0, &config, &mut ids, role, (0.0, 0.0));
        let start = ((i / 2) * 2).min(ship.ai.route.len().saturating_sub(2));
        place_on_route(&mut ship, start);
        info!(ship_id, ?role, "caravana na Rota da Costa");
        commands.spawn((ship,));
    }
}

pub fn respawn_npcs(
    mut commands: Commands,
    mut queue: ResMut<NpcRespawnQueue>,
    mut ids: ResMut<NpcIdCounter>,
    dev_ships: Res<DevShips>,
    map: Res<ServerWorldMap>,
    config: Res<NpcSpawnConfig>,
    time: Res<Time>,
) {
    if queue.0.is_empty() {
        return;
    }
    let dt = time.delta_secs();
    for pending in &mut queue.0 {
        pending.0 -= dt;
    }
    let ready: Vec<(NpcRole, (f32, f32))> = queue
        .0
        .iter()
        .filter(|pending| pending.0 <= 0.0)
        .map(|pending| (pending.1, pending.2))
        .collect();
    queue.0.retain(|pending| pending.0 > 0.0);
    for (role, position) in ready {
        let (ship_id, ship) = build_npc(&dev_ships, &map.0, &config, &mut ids, role, position);
        commands.spawn((ship,));
        info!(
            ship_id,
            ?role,
            x = position.0,
            y = position.1,
            "NPC respawnou"
        );
    }
}

/// MV-061: Frota do Tesouro — o galeão na rota do sul e duas escoltas da
/// coroa, uma em cada bordo.
pub(crate) fn spawn_treasure_fleet(
    commands: &mut Commands,
    dev_ships: &DevShips,
    map: &WorldMap,
    config: &NpcSpawnConfig,
    ids: &mut NpcIdCounter,
) {
    let (leader, mut galleon) = build_npc(
        dev_ships,
        map,
        config,
        ids,
        NpcRole::TreasureGalleon,
        map.features().fleet_route[0],
    );
    place_on_route(&mut galleon, 0);
    let (gx, gy) = (galleon.motion.x, galleon.motion.y);
    commands.spawn((galleon,));
    for side in [-1.0_f32, 1.0] {
        let (id, mut escort) = build_npc(
            dev_ships,
            map,
            config,
            ids,
            NpcRole::Escort,
            (gx, gy + side * ESCORT_OFFSET),
        );
        escort.ai.state = NpcState::Escort { leader };
        escort.ai.escort_leader = leader;
        info!(npc_id = id, leader, "escolta na Frota do Tesouro");
        commands.spawn((escort,));
    }
    info!(npc_id = leader, "Frota do Tesouro zarpou");
}

/// MV-061: o Kraken emerge no ponto do evento.
pub(crate) fn spawn_kraken(
    commands: &mut Commands,
    dev_ships: &DevShips,
    map: &WorldMap,
    config: &NpcSpawnConfig,
    ids: &mut NpcIdCounter,
    at: (f32, f32),
) {
    let (id, kraken) = build_npc(dev_ships, map, config, ids, NpcRole::Kraken, at);
    commands.spawn((kraken,));
    info!(npc_id = id, x = at.0, y = at.1, "Kraken emergiu");
}

pub(crate) fn build_npc(
    dev_ships: &DevShips,
    map: &WorldMap,
    config: &NpcSpawnConfig,
    ids: &mut NpcIdCounter,
    role: NpcRole,
    position: (f32, f32),
) -> (u32, NpcShip) {
    let ship_id = next_npc_id(ids);
    let kind = role.kind();
    let definition = dev_ships.definition(kind).clone();
    let mut stats = compute_ship_stats(
        &definition,
        &EquippedComponents::default(),
        &ItemCatalog::default(),
    )
    .expect("stats de navio sem equipamento não podem falhar");
    // MV-067: NPC nunca atira mais longe que o navio inicial do jogador —
    // quem é atingido sempre consegue revidar (o Kraken morde de perto).
    stats.weapon_range = stats.weapon_range.min(npc_max_range(dev_ships));
    let max_hp = stats.max_hp;
    let cargo_capacity = stats.cargo_capacity;
    let position = config.home(role, position);
    let (state, route, detection_radius, leash_radius, spoils) = match role {
        NpcRole::Pirate => (patrol_state(position), Vec::new(), 380.0, 600.0, 10),
        // Afundar a marinha não rende nada da coroa.
        NpcRole::Navy => (patrol_state(position), Vec::new(), 450.0, 1_000.0, 0),
        NpcRole::Caravan { reverse } => (NpcState::Travel, config.route(reverse), 0.0, 0.0, 0),
        NpcRole::Kraken => (patrol_state(position), Vec::new(), 450.0, 900.0, 0),
        // Butim do Leviatã é por participante (`world_boss`), não destroço.
        NpcRole::Leviathan => (patrol_state(position), Vec::new(), 550.0, 900.0, 0),
        NpcRole::TreasureGalleon => (
            NpcState::Travel,
            map.features().fleet_route.clone(),
            0.0,
            0.0,
            0,
        ),
        // A escolta ganha o líder em `spawn_treasure_fleet`.
        NpcRole::Escort => (NpcState::Idle, Vec::new(), 0.0, 700.0, 0),
        // Guarda a praia: vê longe, não larga a ilha. O butim é o baú.
        NpcRole::Guardian => (patrol_state(position), Vec::new(), 450.0, 500.0, 0),
        // Caça dentro da maré; o butim é bruto (e cinza, no `simulate_npcs`).
        NpcRole::Reaver => (patrol_state(position), Vec::new(), 500.0, 700.0, 10),
    };
    // MV-061: cascos de evento são maiores que o navio base do papel.
    let (max_hp, stats) = match role {
        NpcRole::Kraken => (
            900,
            ShipStats {
                speed: stats.speed * 1.1,
                weapon_damage: KRAKEN_BITE,
                weapon_range: KRAKEN_REACH,
                ..stats
            },
        ),
        NpcRole::Leviathan => (
            LEVIATHAN_HP,
            ShipStats {
                speed: stats.speed * 0.9,
                weapon_damage: LEVIATHAN_BITE,
                weapon_range: LEVIATHAN_REACH,
                ..stats
            },
        ),
        NpcRole::TreasureGalleon => (
            max_hp * 3,
            ShipStats {
                speed: stats.speed * 0.7,
                ..stats
            },
        ),
        _ => (max_hp, stats),
    };
    // v31: pirata de elite — mais comum longe das capitais.
    let elite = if role == NpcRole::Pirate {
        let chance = match map.zone_at(position.0, position.1).map(|zone| zone.tier) {
            Ok(RiskTier::Protected) => 0,
            Ok(RiskTier::Frontier) => 15,
            _ => 35,
        };
        elite::roll(u64::from(ship_id) ^ 0xE117_E000, chance)
    } else if role == NpcRole::Reaver {
        // Saqueador é sempre elite.
        elite::roll(u64::from(ship_id) ^ 0xB100_D000, 100)
    } else if role == NpcRole::Leviathan {
        // Não é afixo: é a marca de chefe (placa e barra de vida no client).
        elite::BOSS
    } else {
        0
    };
    let (max_hp, stats, spoils) = with_elite(elite, max_hp, stats, spoils);
    let weapon_range = stats.weapon_range;
    let mut ship = NpcShip {
        ship_id,
        kind,
        role,
        hp: max_hp,
        max_hp,
        presence: VesselPresence::AtSea,
        hold: CargoHold::new(ShipInstanceId::new(), cargo_capacity),
        battery: BroadsideBattery::default(),
        stats,
        motion: ShipMotion {
            x: position.0,
            y: position.1,
            ..ShipMotion::default()
        },
        tuning: MotionTuning::default(),
        zone: map.zone_at(position.0, position.1).ok().map(|zone| zone.id),
        ai: NpcAi {
            state,
            detection_radius,
            leash_radius,
            weapon_range,
            respawn_after_secs: config.respawn_secs(role),
            spoils,
            spawn_position: position,
            route,
            next_waypoint: 0,
            escort_leader: 0,
        },
        last_damage_dealer: None,
        last_target: None,
        elite,
        regen_carry: 0.0,
        detour: Detour::default(),
    };
    place_on_route(&mut ship, 0);
    (ship_id, ship)
}

/// Afixos de elite sobre casco, stats e butim (bruto em dobro).
fn with_elite(elite: u8, max_hp: u32, stats: ShipStats, spoils: u32) -> (u32, ShipStats, u32) {
    if elite == 0 {
        return (max_hp, stats, spoils);
    }
    let on = |affix| elite::has(elite, affix);
    let max_hp = if on(EliteAffix::Armored) {
        (max_hp as f32 * elite::ARMORED_HP) as u32
    } else {
        max_hp
    };
    let (speed, turn_rate) = if on(EliteAffix::Swift) {
        (stats.speed * elite::SWIFT, stats.turn_rate * elite::SWIFT)
    } else {
        (stats.speed, stats.turn_rate)
    };
    let weapon_damage = if on(EliteAffix::Incendiary) {
        (stats.weapon_damage as f32 * elite::INCENDIARY_DAMAGE).round() as u32
    } else {
        stats.weapon_damage
    };
    let reload_factor = if on(EliteAffix::DoubleSalvo) {
        stats.reload_factor * elite::DOUBLE_SALVO_RELOAD
    } else {
        stats.reload_factor
    };
    (
        max_hp,
        ShipStats {
            max_hp,
            speed,
            turn_rate,
            weapon_damage,
            reload_factor,
            ..stats
        },
        spoils * elite::SPOILS_FACTOR,
    )
}

/// Regenerante: 2% do casco por segundo, com fração acumulada.
fn regenerate(npc: &mut NpcShip, dt: f32) {
    if !elite::has(npc.elite, EliteAffix::Regenerating) || npc.hp >= npc.max_hp {
        npc.regen_carry = 0.0;
        return;
    }
    npc.regen_carry += npc.max_hp as f32 * elite::REGEN_PCT_PER_SEC * dt;
    let whole = npc.regen_carry.floor();
    npc.regen_carry -= whole;
    npc.hp = (npc.hp + whole as u32).min(npc.max_hp);
}

fn patrol_state(origin: (f32, f32)) -> NpcState {
    NpcState::Patrol {
        origin,
        radius: PATROL_RADIUS,
    }
}

/// Estado de repouso do papel: caravana volta à rota, os demais patrulham.
fn home_state(npc: &NpcShip) -> NpcState {
    match npc.role {
        NpcRole::Caravan { .. } | NpcRole::TreasureGalleon => NpcState::Travel,
        NpcRole::Escort => NpcState::Escort {
            leader: npc.ai.escort_leader,
        },
        _ => patrol_state(npc.ai.spawn_position),
    }
}

/// Põe a caravana no waypoint `index`, aproada para o seguinte. Sem rota,
/// nada muda.
fn place_on_route(ship: &mut NpcShip, index: usize) {
    let Some(&(x, y)) = ship.ai.route.get(index) else {
        return;
    };
    ship.motion.x = x;
    ship.motion.y = y;
    ship.ai.next_waypoint = index + 1;
    if let Some(&(nx, ny)) = ship.ai.route.get(index + 1) {
        ship.motion.heading = (ny - y).atan2(nx - x);
    }
}

fn next_npc_id(ids: &mut NpcIdCounter) -> u32 {
    let ship_id = NPC_ID_OFFSET + ids.0;
    ids.0 += 1;
    ship_id
}

/// Um navio de jogador visto pela IA.
#[derive(Debug, Clone, Copy)]
pub(crate) struct Contact {
    pub ship_id: u32,
    pub x: f32,
    pub y: f32,
    pub cargo_weight: u32,
    /// Procurado ou atacou caravana há pouco.
    pub hunted_by_navy: bool,
    pub zone: Option<RiskTier>,
}

/// O papel aceita caçar este contato onde ele está?
fn lawful_prey(role: NpcRole, contact: &Contact) -> bool {
    match role {
        // Pirata nunca entra em águas protegidas (regra antiga).
        NpcRole::Pirate | NpcRole::Guardian | NpcRole::Reaver => {
            contact.zone != Some(RiskTier::Protected)
        }
        // Marinha ignora honestos; caça procurados na coroa e na fronteira.
        NpcRole::Navy => {
            contact.hunted_by_navy
                && matches!(contact.zone, Some(RiskTier::Protected | RiskTier::Frontier))
        }
        // MV-061: o monstro ataca qualquer casco fora das águas da coroa.
        NpcRole::Kraken | NpcRole::Leviathan => contact.zone != Some(RiskTier::Protected),
        // A escolta persegue quem mexeu com a frota, em qualquer água.
        NpcRole::Escort => contact.hunted_by_navy,
        NpcRole::Caravan { .. } | NpcRole::TreasureGalleon => false,
    }
}

/// Alvo novo dentro do raio. Pirata prefere o porão mais pesado (desempate:
/// o mais perto); marinha vai no mais perto.
pub(crate) fn pick_target(
    role: NpcRole,
    x: f32,
    y: f32,
    radius: f32,
    contacts: &[Contact],
) -> Option<u32> {
    let dist = |c: &Contact| distance_sq(x, y, c.x, c.y);
    let candidates = contacts
        .iter()
        .filter(|c| dist(c) <= radius * radius && lawful_prey(role, c));
    let best = match role {
        NpcRole::Pirate => candidates.max_by(|a, b| {
            a.cargo_weight
                .cmp(&b.cargo_weight)
                .then(dist(b).total_cmp(&dist(a)))
        }),
        _ => candidates.min_by(|a, b| dist(a).total_cmp(&dist(b))),
    };
    best.map(|c| c.ship_id)
}

/// Avança a IA de todos os NPCs: decisão, rumo, desvio de terra e tiro.
#[allow(clippy::too_many_arguments)]
pub fn drive_npcs(
    mut commands: Commands,
    mut npcs: Query<(Entity, &mut NpcShip)>,
    ships: Query<&ServerShip>,
    map: Res<ServerWorldMap>,
    tuning: Res<CombatTuning>,
    dev: Res<DevItems>,
    reputation: Res<Reputation>,
    mut projectile_ids: ResMut<ProjectileIdCounter>,
    mut npc_respawns: ResMut<NpcRespawnQueue>,
    time: Res<Time>,
    mut deferred: ResMut<crate::net::DeferredImpacts>,
) {
    let dt = time.delta_secs();
    // MV-061: posição dos galeões para as escoltas manterem formação.
    let leaders: HashMap<u32, ShipMotion> = npcs
        .iter()
        .filter(|(_, npc)| npc.role == NpcRole::TreasureGalleon)
        .map(|(_, npc)| (npc.ship_id, npc.motion))
        .collect();
    let contacts: Vec<Contact> = ships
        .iter()
        .filter(|ship| ship.client_id.is_some() && ship.presence == VesselPresence::AtSea)
        .map(|ship| Contact {
            ship_id: ship.ship_id,
            x: ship.motion.x,
            y: ship.motion.y,
            cargo_weight: ship.hold.used_weight(&dev.catalog).unwrap_or(0),
            hunted_by_navy: reputation.hunted_by_navy(ship.character),
            zone: map
                .0
                .zone_at(ship.motion.x, ship.motion.y)
                .ok()
                .map(|zone| zone.tier),
        })
        .collect();
    let find = |id: u32| contacts.iter().find(|c| c.ship_id == id).copied();

    for (entity, mut npc) in &mut npcs {
        // `simulate_npcs` já despawna o morto; aqui só não mexe nele.
        if npc.ai.state == NpcState::Dead {
            continue;
        }
        npc.battery.advance(dt);
        regenerate(&mut npc, dt);
        if matches!(npc.role, NpcRole::Pirate | NpcRole::Reaver)
            && in_protected_area(&map.0, npc.motion.x, npc.motion.y)
        {
            npc.ai.state = home_state(&npc);
            npc.last_target = None;
        }
        let (x, y) = (npc.motion.x, npc.motion.y);

        let input = match npc.ai.state.clone() {
            NpcState::Idle | NpcState::Patrol { .. } => {
                if let Some(target) =
                    pick_target(npc.role, x, y, npc.ai.detection_radius, &contacts)
                {
                    npc.last_target = Some(target);
                    npc.ai.state = NpcState::Chase { target };
                    None
                } else if let NpcState::Patrol { origin, radius } = npc.ai.state {
                    Some(patrol_input(npc.motion, origin, radius))
                } else {
                    None
                }
            }
            NpcState::Travel => match advance_route(&mut npc.ai, x, y) {
                Some((wx, wy)) => Some(steer_input(npc.motion, wx, wy)),
                None => {
                    // Atracou no destino: sai do mar e volta mais tarde, do
                    // mesmo porto, no sentido contrário. O galeão da frota
                    // escapou com a carga: não volta.
                    if let NpcRole::Caravan { reverse } = npc.role {
                        npc_respawns.0.push((
                            npc.ai.respawn_after_secs,
                            NpcRole::Caravan { reverse: !reverse },
                            (x, y),
                        ));
                    }
                    info!(npc_id = npc.ship_id, "caravana atracou no destino");
                    commands.entity(entity).despawn();
                    continue;
                }
            },
            NpcState::Flee { from, secs } => {
                let secs = secs - dt;
                match find(from) {
                    Some(attacker) if secs > 0.0 => {
                        npc.ai.state = NpcState::Flee { from, secs };
                        Some(flee_input(npc.motion, attacker.x, attacker.y, secs))
                    }
                    _ => {
                        npc.ai.state = NpcState::Travel;
                        None
                    }
                }
            }
            NpcState::Escort { leader } => {
                if let Some(target) =
                    pick_target(npc.role, x, y, npc.ai.detection_radius, &contacts)
                {
                    npc.last_target = Some(target);
                    npc.ai.state = NpcState::Chase { target };
                    None
                } else if let Some(lead) = leaders.get(&leader) {
                    Some(escort_input(npc.motion, *lead, npc.ship_id))
                } else {
                    // Galeão afundou ou escapou: a escolta patrulha ali.
                    npc.ai.state = patrol_state((x, y));
                    None
                }
            }
            NpcState::Chase { target } => match find(target) {
                Some(c) if keeps_hunting(&npc, &c) => {
                    npc.last_target = Some(target);
                    if distance(x, y, c.x, c.y) <= npc.ai.weapon_range {
                        npc.ai.state = NpcState::Attack;
                    }
                    Some(steer_input(npc.motion, c.x, c.y))
                }
                _ => {
                    npc.ai.state = home_state(&npc);
                    npc.last_target = None;
                    None
                }
            },
            NpcState::Attack => match npc.last_target.and_then(find) {
                Some(c) if keeps_hunting(&npc, &c) => {
                    if distance(x, y, c.x, c.y) > npc.ai.weapon_range {
                        npc.ai.state = NpcState::Chase { target: c.ship_id };
                        Some(steer_input(npc.motion, c.x, c.y))
                    } else if npc.role.is_monster() {
                        // MV-061: o Kraken abraça o casco e morde.
                        if npc.battery.try_fire(BroadsideSide::Port, KRAKEN_BITE_SECS) {
                            deferred.0.push(crate::net::Impact {
                                projectile: None,
                                target_ship_id: c.ship_id,
                                hull_damage: npc.stats.weapon_damage,
                                attacker_ship_id: npc.ship_id,
                                sail_damage: 6.0,
                                at: (x, y),
                                boarded: false,
                            });
                        }
                        Some(steer_input(npc.motion, c.x, c.y))
                    } else {
                        // v21: mesma regra do jogador — tiro em 360° assim
                        // que recarrega; o NPC segue manobrando de costado.
                        let (side, aim) =
                            marvyr_domain_combat::aim_at(npc.motion.heading, (x, y), (c.x, c.y));
                        let cooldown = tuning.cooldown_secs * npc.stats.reload_factor;
                        if npc.battery.try_fire(side, cooldown) {
                            spawn_projectile(
                                &mut commands,
                                &mut projectile_ids,
                                &npc,
                                side,
                                aim,
                                &tuning,
                            );
                        }
                        Some(broadside_input(npc.motion, c.x, c.y))
                    }
                }
                _ => {
                    npc.ai.state = home_state(&npc);
                    npc.last_target = None;
                    None
                }
            },
            NpcState::Dead => None,
        };

        if let Some(input) = input {
            let input = avoid_land(&map.0, npc.motion, input);
            let NpcShip {
                motion,
                stats,
                tuning,
                detour,
                ..
            } = &mut *npc;
            let input = unstick(motion, detour, input, stats.turn_rate, dt);
            step_motion(motion, stats, input, tuning, dt);
            if let Some(away) = ground_on_land(&map.0, motion) {
                detour.start(away);
            }
        }
    }
}

/// Quanto tempo o NPC desencalhado segue o desvio antes de voltar ao alvo.
const DETOUR_SECS: f32 = 4.0;

/// Encalhe: o leme escala com a velocidade, e o casco empurrado para fora
/// da terra a cada tick fica sem velocidade — o NPC não virava, o alvo do
/// outro lado da ilha puxava a proa de volta e ele raspava a costa para
/// sempre. Encalhou, ele se compromete com o desvio: gira no lugar (a remo)
/// até apontar para longe da terra e sai de pano cheio por alguns
/// segundos; só então volta a mirar o alvo.
pub(crate) fn unstick(
    motion: &mut ShipMotion,
    detour: &mut Detour,
    input: MotionInput,
    turn_rate: f32,
    dt: f32,
) -> MotionInput {
    if detour.secs <= 0.0 {
        return input;
    }
    detour.secs -= dt;
    let off = detour.away - motion.heading;
    let off = off.sin().atan2(off.cos());
    let step = turn_rate * dt;
    let heading = motion.heading + off.clamp(-step, step);
    motion.heading = heading.sin().atan2(heading.cos());
    MotionInput {
        throttle: if off.abs() < 0.6 { 1.0 } else { 0.2 },
        turn: 0.0,
    }
}

/// Caça continua enquanto o alvo segue presa legítima e dentro da coleira.
fn keeps_hunting(npc: &NpcShip, contact: &Contact) -> bool {
    lawful_prey(npc.role, contact)
        && distance(npc.motion.x, npc.motion.y, contact.x, contact.y) <= npc.ai.leash_radius
}

/// Próximo waypoint da rota; `None` = chegou ao fim (atracar).
fn advance_route(ai: &mut NpcAi, x: f32, y: f32) -> Option<(f32, f32)> {
    while let Some(&(wx, wy)) = ai.route.get(ai.next_waypoint) {
        if distance(x, y, wx, wy) > WAYPOINT_RADIUS {
            return Some((wx, wy));
        }
        ai.next_waypoint += 1;
    }
    None
}

/// Pano cheio para longe do agressor, alternando +-0,5 rad a cada 2 s.
fn flee_input(motion: ShipMotion, from_x: f32, from_y: f32, secs_left: f32) -> MotionInput {
    let away = (motion.y - from_y).atan2(motion.x - from_x);
    let zig = if (secs_left * 0.5) as i32 % 2 == 0 {
        0.5
    } else {
        -0.5
    };
    steer_heading(motion, away + zig)
}

/// Desvio de terra: sonda 80 m à proa; bloqueada, vira para o lado cujo
/// través de +-40° está livre (empate segue o leme desejado).
pub(crate) fn avoid_land(map: &WorldMap, motion: ShipMotion, input: MotionInput) -> MotionInput {
    let blocked = |angle: f32| {
        map.is_land(
            motion.x + LAND_PROBE * angle.cos(),
            motion.y + LAND_PROBE * angle.sin(),
        )
    };
    if !blocked(motion.heading) {
        return input;
    }
    let left_clear = !blocked(motion.heading + LAND_PROBE_SPREAD);
    let right_clear = !blocked(motion.heading - LAND_PROBE_SPREAD);
    let turn = match (left_clear, right_clear) {
        (true, false) => 1.0,
        (false, true) => -1.0,
        _ if input.turn < 0.0 => -1.0,
        _ => 1.0,
    };
    let throttle_cap = if left_clear || right_clear { 0.7 } else { 0.3 };
    MotionInput {
        throttle: input.throttle.min(throttle_cap),
        turn,
    }
}

/// Impactos de projéteis em NPCs, recompensas e alarme de caravana.
#[allow(clippy::too_many_arguments, clippy::type_complexity)]
pub fn simulate_npcs(
    mut commands: Commands,
    mut connection_manager: ResMut<ConnectionManager>,
    mut npcs: Query<(Entity, &mut NpcShip)>,
    ships: Query<&ServerShip>,
    projectiles: Query<(Entity, &ServerProjectile)>,
    map: Res<ServerWorldMap>,
    risk_policy: Res<ServerRiskPolicy>,
    tuning: Res<CombatTuning>,
    config: Res<NpcSpawnConfig>,
    mut market: ResMut<crate::market::ServerMarket>,
    mut metrics: ResMut<crate::net::Metrics>,
    mut npc_respawns: ResMut<NpcRespawnQueue>,
    mut reputation: ResMut<Reputation>,
    (
        mut boardings,
        mut wreck_ids,
        mut live_wrecks,
        dev,
        time,
        mut renown,
        mut flask_hits,
        blood,
        mut boss,
        talents,
        mut fury,
    ): (
        ResMut<crate::seafaring::NpcBoardings>,
        ResMut<crate::net::WreckIdCounter>,
        ResMut<crate::net::LiveWreckRecords>,
        Res<DevItems>,
        Res<Time>,
        EventWriter<crate::renown::RenownEarned>,
        ResMut<crate::flasks::FlaskHits>,
        Res<crate::blood_tide::BloodTide>,
        ResMut<crate::world_boss::WorldBoss>,
        Res<crate::talents::CaptainTalents>,
        ResMut<crate::fury::SeaFury>,
    ),
) {
    let player_positions: HashMap<u32, (f32, f32)> = ships
        .iter()
        .filter(|ship| ship.client_id.is_some())
        .map(|ship| (ship.ship_id, (ship.motion.x, ship.motion.y)))
        .collect();
    let player_owners: HashMap<u32, CharacterId> = ships
        .iter()
        .map(|ship| (ship.ship_id, ship.character))
        .collect();
    let viewers: Vec<(Option<ClientId>, CharacterId)> = ships
        .iter()
        .map(|ship| (ship.client_id, ship.character))
        .collect();
    let client_of = |character: CharacterId| {
        viewers
            .iter()
            .find(|(_, owner)| *owner == character)
            .and_then(|(client, _)| *client)
    };

    // Impactos em NPCs: projéteis que NÃO acertaram jogador nesta passada.
    // O dano usa o mesmo `apply_damage`; NPC morto não vira wreck (Pilar 1).
    let npc_positions: HashMap<u32, (f32, f32)> = npcs
        .iter()
        .map(|(_, npc)| (npc.ship_id, (npc.motion.x, npc.motion.y)))
        .collect();
    let mut npc_impacts: Vec<(Option<Entity>, u32, u32, u32)> = Vec::new();
    // MV-061: NPC tomado por abordagem rende como se afundasse.
    for (npc_id, attacker_ship_id) in boardings.0.drain(..) {
        npc_impacts.push((None, npc_id, u32::MAX, attacker_ship_id));
    }
    for (projectile_entity, projectile) in &projectiles {
        if projectile.0.expired() {
            continue;
        }
        let hits_player = player_positions.iter().any(|(ship_id, (x, y))| {
            *ship_id != projectile.0.owner_ship_id
                && projectile.0.hit_ship(*x, *y, tuning.hit_radius)
        });
        if hits_player {
            continue;
        }
        for (npc_id, (x, y)) in &npc_positions {
            if *npc_id == projectile.0.owner_ship_id {
                continue;
            }
            if projectile.0.hit_ship(*x, *y, tuning.hit_radius) {
                npc_impacts.push((
                    Some(projectile_entity),
                    *npc_id,
                    projectile.0.damage,
                    projectile.0.owner_ship_id,
                ));
                break; // um projétil atinge um navio só
            }
        }
    }

    // Caravanas atacadas nesta passada: (navio agressor, x, y).
    let mut caravan_alarms: Vec<(u32, f32, f32)> = Vec::new();
    for (projectile_entity, target_npc_id, damage, killer_ship_id) in npc_impacts {
        if let Some(projectile_entity) = projectile_entity {
            commands.entity(projectile_entity).despawn();
        }

        let killed = {
            let Some((entity, mut npc)) = npcs
                .iter_mut()
                .find(|(_, npc)| npc.ship_id == target_npc_id)
            else {
                continue;
            };
            if npc.ai.state == NpcState::Dead {
                continue;
            }
            let zone_tier = map
                .0
                .zone_at(npc.motion.x, npc.motion.y)
                .ok()
                .map(|zone| zone.tier);
            let pvp_here = zone_tier.is_some_and(|tier| risk_policy.0.pvp_allowed(tier));
            if !pvp_here {
                info!(
                    npc_id = target_npc_id,
                    "impacto em NPC ignorado: águas protegidas ou fora do mapa"
                );
                continue;
            }
            let killer = player_owners.get(&killer_ship_id).copied();
            npc.last_damage_dealer = killer;
            // Caravana atacada por jogador: suja o nome, foge e chama a
            // marinha (um alarme por fuga).
            if let (true, Some(attacker)) = (npc.role.is_merchant(), killer) {
                let gain = notoriety_gain(Offense::Hit, zone_tier, false);
                crate::reputation::raise_notoriety(
                    &mut connection_manager,
                    &mut reputation,
                    client_of(attacker),
                    attacker,
                    gain,
                );
                reputation.mark_crown_aggressor(attacker);
                if !matches!(npc.ai.state, NpcState::Flee { .. }) {
                    caravan_alarms.push((killer_ship_id, npc.motion.x, npc.motion.y));
                }
                npc.ai.state = NpcState::Flee {
                    from: killer_ship_id,
                    secs: config.caravan_flee_secs,
                };
            }
            // v34: Caçador morde mais fundo em NPC.
            let damage = if killer.is_some_and(|k| {
                talents.class(k) == Some(marvyr_domain_ships::talents::CaptainClass::Hunter)
            }) {
                damage * (100 + marvyr_domain_ships::talents::HUNTER_DAMAGE_PCT) / 100
            } else {
                damage
            };
            let outcome = apply_npc_damage(&mut npc, damage);
            // v34: o Leviatã lembra quem lutou (butim por participante).
            if npc.role == NpcRole::Leviathan {
                if let Some(character) = killer {
                    boss.record_hit(character, damage);
                }
                if outcome == DamageOutcome::Destroyed {
                    boss.slain((npc.motion.x, npc.motion.y));
                }
            }
            flask_hits
                .0
                .push((killer_ship_id, outcome == DamageOutcome::Destroyed));
            match outcome {
                DamageOutcome::Survived { remaining_hp } => {
                    info!(
                        npc_id = target_npc_id,
                        damage,
                        hp = remaining_hp,
                        "impacto em NPC"
                    );
                    None
                }
                DamageOutcome::Destroyed => Some((
                    entity,
                    npc.ship_id,
                    npc.role,
                    npc.ai.spawn_position,
                    npc.ai.respawn_after_secs,
                    npc.ai.spoils,
                    npc.elite,
                    killer,
                    zone_tier,
                    (npc.motion.x, npc.motion.y),
                )),
            }
        };

        let Some((
            entity,
            npc_ship_id,
            role,
            spawn_position,
            respawn_after_secs,
            spoils,
            npc_elite,
            killer,
            zone_tier,
            position,
        )) = killed
        else {
            continue;
        };

        info!(
            npc_id = npc_ship_id,
            ?role,
            "NPC DESTROYED; sem wreck (Pilar 1)"
        );
        if let Some(killer) = killer {
            let spoils = match role {
                NpcRole::Caravan { .. } | NpcRole::TreasureGalleon => {
                    let gain = notoriety_gain(Offense::Sink, zone_tier, false);
                    crate::reputation::raise_notoriety(
                        &mut connection_manager,
                        &mut reputation,
                        client_of(killer),
                        killer,
                        gain,
                    );
                    if role == NpcRole::TreasureGalleon {
                        config.fleet_spoils
                    } else {
                        config.caravan_spoils
                    }
                }
                _ => spoils,
            };
            // v36: Fúria do Mar — a sequência engorda o butim e sobe.
            let spoils = fury.spoils(killer, spoils);
            fury.bump(killer);
            // Caçadas contam pirata e Kraken (quem a coroa quer no fundo).
            if matches!(
                role,
                NpcRole::Pirate
                    | NpcRole::Kraken
                    | NpcRole::Guardian
                    | NpcRole::Reaver
                    | NpcRole::Leviathan
            ) {
                let sunk_in = map
                    .0
                    .area_at(position.0, position.1)
                    .map(|index| map.0.features().areas[index].name);
                market.npc_kills.push((killer, sunk_in));
            }
            metrics.npc_spoils_dropped += u64::from(spoils);
            renown.send(crate::renown::RenownEarned {
                character: killer,
                amount: role.renown(),
                // v35: o Diário conta elite à parte.
                reason: if npc_elite != 0 {
                    "elite afundado"
                } else {
                    "navio afundado"
                },
            });
            if let Some(client) = client_of(killer) {
                let text = match role {
                    NpcRole::Caravan { .. } => {
                        String::from("Voce afundou um Mercador: carga boiando")
                    }
                    NpcRole::TreasureGalleon => {
                        String::from("Voce afundou o Galeao do Tesouro: carga boiando")
                    }
                    _ if spoils > 0 => format!("Voce afundou {}: carga boiando", role.label()),
                    _ => format!("Voce afundou um {}", role.label()),
                };
                crate::reputation::send_event(
                    &mut connection_manager,
                    &[client],
                    text,
                    WorldEventKind::Kill,
                );
            }
            // Pilar 1: só recurso bruto, e ainda precisa ser recolhido.
            if spoils > 0 {
                let mut loot = raw_spoils(&dev, spoils);
                // v32: afundou dentro da Maré Sangrenta — cinza no destroço.
                if let Some(ash) = blood.ash_for(role, position) {
                    loot.push((dev.blood_ash, ash));
                }
                spawn_spoils_wreck(
                    &mut commands,
                    &mut wreck_ids,
                    &mut live_wrecks,
                    loot,
                    Some(killer),
                    position,
                    time.elapsed_secs(),
                );
            }
            info!(npc_id = npc_ship_id, killer = ?killer, spoils, "NPC afundado; despojos boiando");
        }
        // MV-061: o Kraken deixa recurso bruto boiando (vira carga de
        // jogador — ainda precisa chegar ao porto e ser fabricado).
        if role == NpcRole::Kraken {
            spawn_spoils_wreck(
                &mut commands,
                &mut wreck_ids,
                &mut live_wrecks,
                crate::seafaring::kraken_spoils(&dev),
                killer,
                position,
                time.elapsed_secs(),
            );
        }
        commands.entity(entity).despawn();
        if !role.is_event_npc() {
            npc_respawns
                .0
                .push((respawn_after_secs, role, config.home(role, spawn_position)));
        }
    }

    // Caravana grita; a marinha por perto larga a patrulha e vem.
    for (attacker_ship_id, x, y) in caravan_alarms {
        let mut responding = false;
        for (_, mut navy) in &mut npcs {
            // Quem já está caçando não larga o alvo pelo alarme.
            let busy = matches!(
                navy.ai.state,
                NpcState::Dead | NpcState::Chase { .. } | NpcState::Attack
            );
            if matches!(navy.role, NpcRole::Navy | NpcRole::Escort)
                && !busy
                && distance(navy.motion.x, navy.motion.y, x, y) <= config.navy_response_radius
            {
                navy.ai.state = NpcState::Chase {
                    target: attacker_ship_id,
                };
                navy.last_target = Some(attacker_ship_id);
                responding = true;
            }
        }
        let text = if responding {
            "Mercador atacado! Marinha a caminho"
        } else {
            "Mercador atacado!"
        };
        let witnesses: Vec<ClientId> = ships
            .iter()
            .filter(|ship| crate::aoi::is_visible((x, y), (ship.motion.x, ship.motion.y)))
            .filter_map(|ship| ship.client_id)
            .collect();
        crate::reputation::send_event(
            &mut connection_manager,
            &witnesses,
            text.to_owned(),
            WorldEventKind::Alert,
        );
    }
}

/// Wreck de despojos (MV-061): NPC de evento abatido ou baú de cerração.
pub(crate) fn spawn_spoils_wreck(
    commands: &mut Commands,
    wreck_ids: &mut crate::net::WreckIdCounter,
    live_wrecks: &mut crate::net::LiveWreckRecords,
    spoils: Vec<(marvyr_shared::ids::ItemDefinitionId, u32)>,
    exclusive_looter: Option<CharacterId>,
    (x, y): (f32, f32),
    now: f32,
) {
    let wreck_num = wreck_ids.0;
    wreck_ids.0 += 1;
    let wreck_id = marvyr_shared::ids::WreckId::new();
    let mut chest = marvyr_domain_combat::WreckChest::new(wreck_id);
    for (definition, quantity) in spoils {
        chest.insert(
            marvyr_domain_combat::SurvivorItem {
                definition,
                quantity,
                durability: None,
                quality: None,
            },
            marvyr_shared::ids::ItemInstanceId::new(),
        );
    }
    commands.spawn((crate::net::ServerWreck {
        wreck_num,
        wreck_id,
        chest,
        exclusive_looter,
        spawned_at_secs: now,
        x,
        y,
    },));
    live_wrecks.0.push(crate::persist::WreckRecord {
        wreck_num,
        wreck_id,
        x,
        y,
        exclusive_looter,
        spawned_at_secs: f64::from(now),
    });
    info!(wreck_num, x, y, "despojos boiando");
}

pub(crate) fn apply_npc_damage(npc: &mut NpcShip, damage: u32) -> DamageOutcome {
    let outcome = apply_damage(npc.hp, damage);
    match outcome {
        DamageOutcome::Survived { remaining_hp } => npc.hp = remaining_hp,
        DamageOutcome::Destroyed => {
            npc.hp = 0;
            npc.ai.state = NpcState::Dead;
        }
    }
    outcome
}

/// Despojos de NPC: metade madeira, metade minério (Pilar 1: só bruto).
pub(crate) fn raw_spoils(
    dev: &crate::net::DevItems,
    units: u32,
) -> Vec<(marvyr_shared::ids::ItemDefinitionId, u32)> {
    [(dev.timber, units - units / 2), (dev.ore, units / 2)]
        .into_iter()
        .filter(|(_, quantity)| *quantity > 0)
        .collect()
}

pub(crate) fn to_npc_ship_state(npc: &NpcShip, catalog: &ItemCatalog) -> ShipState {
    ShipState {
        ship_id: npc.ship_id,
        kind: npc.kind,
        x: npc.motion.x,
        y: npc.motion.y,
        heading: npc.motion.heading,
        speed: npc.motion.speed,
        cargo_weight: npc
            .hold
            .used_weight(catalog)
            .expect("porão NPC só contém definições do catálogo"),
        hp: npc.hp,
        max_hp: npc.max_hp,
        max_speed: npc.stats.speed,
        weapon_damage: npc.stats.weapon_damage,
        weapon_range: npc.stats.weapon_range,
        port_cooldown_secs: npc.battery.port_cooldown,
        starboard_cooldown_secs: npc.battery.starboard_cooldown,
        is_npc: true,
        cargo_capacity: npc.stats.cargo_capacity,
        sail_hp: 100.0,
        ammo: Default::default(),
        faction: npc.role.faction(),
        notoriety_tier: 0,
        rudder_hp: 100.0,
        crew: 0,
        crew_max: 0,
        repairing: false,
        dig_progress: 0.0,
        sail_cosmetic: 0,
        flag_cosmetic: 0,
        black_flag: 0,
        fire_target: None,
        aura: 0,
        flasks: Default::default(),
        elite: npc.elite,
        fury: 0,
    }
}

/// Escolta: um bordo de cada lado do galeão (pela paridade do id), no
/// mesmo pano — encosta e segura a formação.
fn escort_input(motion: ShipMotion, leader: ShipMotion, ship_id: u32) -> MotionInput {
    let side = if ship_id % 2 == 0 { 1.0 } else { -1.0 };
    let normal = leader.heading + side * std::f32::consts::FRAC_PI_2;
    let station = (
        leader.x + normal.cos() * ESCORT_OFFSET + leader.heading.cos() * 20.0,
        leader.y + normal.sin() * ESCORT_OFFSET + leader.heading.sin() * 20.0,
    );
    let mut input = steer_input(motion, station.0, station.1);
    if distance(motion.x, motion.y, station.0, station.1) < 40.0 {
        input = steer_heading(motion, leader.heading);
        input.throttle = 0.7;
    }
    input
}

fn patrol_input(motion: ShipMotion, origin: (f32, f32), radius: f32) -> MotionInput {
    let target = if motion.x < origin.0 + radius * 0.5 {
        (origin.0 + radius, origin.1)
    } else {
        origin
    };
    steer_input(motion, target.0, target.1)
}

fn steer_input(motion: ShipMotion, target_x: f32, target_y: f32) -> MotionInput {
    steer_heading(motion, (target_y - motion.y).atan2(target_x - motion.x))
}

fn steer_heading(motion: ShipMotion, desired: f32) -> MotionInput {
    let delta = angle_delta(desired, motion.heading);
    MotionInput {
        throttle: 1.0,
        turn: (delta / std::f32::consts::PI).clamp(-1.0, 1.0),
    }
}

/// Rumo que põe o alvo a 90° (costado), escolhendo o lado mais perto do
/// rumo atual, a meio pano para manter manobra.
fn broadside_input(motion: ShipMotion, target_x: f32, target_y: f32) -> MotionInput {
    let bearing = (target_y - motion.y).atan2(target_x - motion.x);
    let half_pi = std::f32::consts::FRAC_PI_2;
    let a = angle_delta(bearing + half_pi, motion.heading);
    let b = angle_delta(bearing - half_pi, motion.heading);
    let delta = if a.abs() <= b.abs() { a } else { b };
    MotionInput {
        throttle: 0.6,
        turn: (delta / half_pi).clamp(-1.0, 1.0),
    }
}

fn angle_delta(target: f32, current: f32) -> f32 {
    let mut delta = (target - current).rem_euclid(std::f32::consts::TAU);
    if delta > std::f32::consts::PI {
        delta -= std::f32::consts::TAU;
    }
    delta
}

fn distance_sq(ax: f32, ay: f32, bx: f32, by: f32) -> f32 {
    let dx = ax - bx;
    let dy = ay - by;
    dx * dx + dy * dy
}

fn distance(ax: f32, ay: f32, bx: f32, by: f32) -> f32 {
    distance_sq(ax, ay, bx, by).sqrt()
}

fn in_protected_area(map: &WorldMap, x: f32, y: f32) -> bool {
    if map
        .zone_at(x, y)
        .is_ok_and(|zone| zone.tier == RiskTier::Protected)
    {
        return true;
    }
    map.regions()
        .iter()
        .filter_map(|region| region.port.as_ref())
        .any(|port| port.contains(x, y))
}

fn spawn_projectile(
    commands: &mut Commands,
    projectile_ids: &mut ProjectileIdCounter,
    npc: &NpcShip,
    side: BroadsideSide,
    aim: f32,
    tuning: &CombatTuning,
) {
    let projectile_id = projectile_ids.0;
    projectile_ids.0 += tuning.salvo_balls.max(1);
    let weapon = WeaponParams {
        damage: npc.stats.weapon_damage,
        speed: tuning.projectile_speed,
        range: npc.stats.weapon_range,
        muzzle_offset: tuning.muzzle_offset,
    };
    let salvo = Projectile::broadside_salvo(
        projectile_id,
        npc.ship_id,
        side,
        npc.motion.x,
        npc.motion.y,
        npc.motion.heading,
        npc.motion.speed,
        weapon,
        tuning.salvo_balls,
        tuning.salvo_spacing,
        marvyr_domain_combat::Ammo::Round,
    );
    commands.spawn_batch(salvo.into_iter().map(move |mut p| {
        p.rotate(aim);
        (ServerProjectile(p),)
    }));
    info!(
        npc_id = npc.ship_id,
        projectile_id, "NPC broadside disparada"
    );
}

#[cfg(test)]
mod tests {
    /// Um tick de navegação de NPC como no `move_npcs`: desencalhe, motor
    /// e terra.
    fn sail(map: &WorldMap, npc: &mut NpcShip, input: MotionInput, dt: f32) {
        let input = unstick(
            &mut npc.motion,
            &mut npc.detour,
            input,
            npc.stats.turn_rate,
            dt,
        );
        step_motion(&mut npc.motion, &npc.stats, input, &npc.tuning, dt);
        if let Some(away) = ground_on_land(map, &mut npc.motion) {
            npc.detour.start(away);
        }
    }

    use super::*;

    fn npc_at(position: (f32, f32)) -> NpcShip {
        let mut ids = NpcIdCounter::default();
        build_npc(
            &DevShips::new(),
            &WorldMap::vertical_slice(),
            &NpcSpawnConfig::default(),
            &mut ids,
            NpcRole::Pirate,
            position,
        )
        .1
    }

    #[test]
    fn npc_spawns_at_configured_position_with_hp() {
        let mut ids = NpcIdCounter::default();
        let (ship_id, npc) = build_npc(
            &DevShips::new(),
            &WorldMap::vertical_slice(),
            &NpcSpawnConfig::default(),
            &mut ids,
            NpcRole::Pirate,
            (0.0, 900.0),
        );

        assert_eq!(ship_id, NPC_ID_OFFSET);
        assert_eq!(npc.ship_id, NPC_ID_OFFSET);
        assert!(npc.hp > 0);
        assert_eq!(npc.motion.x, 0.0);
        assert_eq!(npc.motion.y, 900.0);
        assert!(npc.hold.items().is_empty());
    }

    #[test]
    fn elite_affixes_reshape_the_pirate_and_regenerate() {
        let base = npc_at((0.0, 0.0));
        let mask = EliteAffix::Armored.bit() | EliteAffix::DoubleSalvo.bit();
        let (hp, stats, spoils) = with_elite(mask, base.max_hp, base.stats.clone(), 10);
        assert_eq!(hp, (base.max_hp as f32 * elite::ARMORED_HP) as u32);
        assert_eq!(stats.max_hp, hp);
        assert!(stats.reload_factor < base.stats.reload_factor);
        assert_eq!(stats.speed, base.stats.speed, "sem Veloz, mesmo pano");
        assert_eq!(spoils, 20, "elite rende bruto em dobro");
        assert_eq!(
            with_elite(0, base.max_hp, base.stats.clone(), 10),
            (base.max_hp, base.stats, 10)
        );

        let mut npc = npc_at((0.0, 0.0));
        npc.elite = EliteAffix::Regenerating.bit();
        npc.hp = npc.max_hp / 2;
        for _ in 0..30 {
            regenerate(&mut npc, 1.0 / 30.0);
        }
        let healed = npc.hp - npc.max_hp / 2;
        let expected = (npc.max_hp as f32 * elite::REGEN_PCT_PER_SEC) as u32;
        assert!(healed.abs_diff(expected) <= 1, "{healed} vs {expected}");
        for _ in 0..10_000 {
            regenerate(&mut npc, 0.1);
        }
        assert_eq!(npc.hp, npc.max_hp, "não passa do casco cheio");
    }

    #[test]
    fn patrol_input_moves_npc_toward_patrol_target() {
        let mut npc = npc_at((0.0, 0.0));
        for _ in 0..30 {
            let NpcState::Patrol { origin, radius } = npc.ai.state else {
                panic!("NPC deveria estar em Patrol");
            };
            let input = patrol_input(npc.motion, origin, radius);
            step_motion(&mut npc.motion, &npc.stats, input, &npc.tuning, 1.0 / 30.0);
        }

        assert!(
            npc.motion.x > 0.0,
            "patrulha avança do spawn; x={}",
            npc.motion.x
        );
    }

    fn contact(ship_id: u32, x: f32, cargo_weight: u32, hunted: bool) -> Contact {
        Contact {
            ship_id,
            x,
            y: 0.0,
            cargo_weight,
            hunted_by_navy: hunted,
            zone: Some(RiskTier::Frontier),
        }
    }

    #[test]
    fn npc_detects_nearest_player_inside_radius_and_ignores_outside() {
        let contacts = [contact(1, 10.0, 0, true), contact(2, 1_000.0, 0, true)];
        assert_eq!(
            pick_target(NpcRole::Navy, 0.0, 0.0, 100.0, &contacts),
            Some(1)
        );
        assert_eq!(pick_target(NpcRole::Navy, 0.0, 0.0, 5.0, &contacts), None);
    }

    #[test]
    fn navy_ignores_honest_captains_and_hunts_the_wanted() {
        let honest = [contact(1, 10.0, 0, false)];
        assert_eq!(pick_target(NpcRole::Navy, 0.0, 0.0, 400.0, &honest), None);

        let wanted = [contact(1, 10.0, 0, false), contact(2, 50.0, 0, true)];
        assert_eq!(
            pick_target(NpcRole::Navy, 0.0, 0.0, 400.0, &wanted),
            Some(2)
        );

        // Mar sem lei não é jurisdição da coroa.
        let mut lawless = contact(3, 10.0, 0, true);
        lawless.zone = Some(RiskTier::Lawless);
        assert_eq!(
            pick_target(NpcRole::Navy, 0.0, 0.0, 400.0, &[lawless]),
            None
        );
    }

    #[test]
    fn pirate_prefers_the_heaviest_hold_then_the_closest() {
        let contacts = [
            contact(1, 20.0, 5, false),
            contact(2, 300.0, 60, false),
            contact(3, 100.0, 60, false),
        ];
        assert_eq!(
            pick_target(NpcRole::Pirate, 0.0, 0.0, 400.0, &contacts),
            Some(3)
        );
        let mut protected = contact(4, 10.0, 999, false);
        protected.zone = Some(RiskTier::Protected);
        assert_eq!(
            pick_target(NpcRole::Pirate, 0.0, 0.0, 400.0, &[protected]),
            None
        );
        assert_eq!(
            pick_target(
                NpcRole::Caravan { reverse: false },
                0.0,
                0.0,
                400.0,
                &contacts
            ),
            None
        );
    }

    fn caravan(reverse: bool) -> NpcShip {
        let mut ids = NpcIdCounter::default();
        build_npc(
            &DevShips::new(),
            &WorldMap::vertical_slice(),
            &NpcSpawnConfig::default(),
            &mut ids,
            NpcRole::Caravan { reverse },
            (0.0, 0.0),
        )
        .1
    }

    /// MV-066: no mundo gerado a caravana atravessa portões — nos dois
    /// sentidos, cada um pelos seus.
    #[test]
    fn caravans_cross_zone_gates_both_ways_on_a_generated_world() {
        let map = WorldMap::from_seed(crate::net::DEFAULT_WORLD_SEED);
        let config = NpcSpawnConfig::for_map(&map);
        for reverse in [false, true] {
            let mut ids = NpcIdCounter::default();
            let role = NpcRole::Caravan { reverse };
            let (_, mut npc) = build_npc(
                &DevShips::new(),
                &map,
                &config,
                &mut ids,
                role,
                config.home(role, (0.0, 0.0)),
            );
            let goal = *npc.ai.route.last().unwrap();
            let dt = 1.0 / 30.0;
            let mut arrived = false;
            for _ in 0..(30 * 1200) {
                let Some((wx, wy)) = advance_route(&mut npc.ai, npc.motion.x, npc.motion.y) else {
                    arrived = true;
                    break;
                };
                let input = avoid_land(&map, npc.motion, steer_input(npc.motion, wx, wy));
                sail(&map, &mut npc, input, dt);
                crate::portals::cross_npc(&map, &mut npc);
            }
            assert!(
                arrived,
                "reverse={reverse}: parou em {:?}",
                (npc.motion.x, npc.motion.y)
            );
            assert!(distance(npc.motion.x, npc.motion.y, goal.0, goal.1) <= WAYPOINT_RADIUS);
        }
    }

    #[test]
    fn caravan_starts_at_its_port_and_sails_the_route_to_the_other() {
        let config = NpcSpawnConfig::default();
        let map = WorldMap::vertical_slice();
        let mut npc = caravan(false);
        assert_eq!(npc.kind, ShipKind::SmallMerchant);
        assert_eq!(npc.role.faction(), Faction::Merchant);
        assert_eq!((npc.motion.x, npc.motion.y), config.caravan_route[0]);
        assert_eq!(npc.ai.next_waypoint, 1);

        let dt = 1.0 / 30.0;
        let mut arrived = false;
        for _ in 0..(30 * 240) {
            let Some((wx, wy)) = advance_route(&mut npc.ai, npc.motion.x, npc.motion.y) else {
                arrived = true;
                break;
            };
            let input = avoid_land(&map, npc.motion, steer_input(npc.motion, wx, wy));
            sail(&map, &mut npc, input, dt);
        }
        assert!(arrived, "caravana deveria chegar a Mina");
        let port = config.caravan_route.last().copied().unwrap();
        assert!(distance(npc.motion.x, npc.motion.y, port.0, port.1) <= WAYPOINT_RADIUS);

        // Volta nasce no porto de chegada.
        let back = caravan(true);
        assert_eq!((back.motion.x, back.motion.y), port);
    }

    #[test]
    fn npc_nosed_into_an_island_frees_itself() {
        let map = WorldMap::vertical_slice();
        // Parado, proa encostada na Ilha do Coral Negro, alvo do outro lado.
        let mut x = -80.0;
        while !map.is_land(x + 30.0, 990.0) {
            x += 1.0;
        }
        let mut npc = caravan(false);
        npc.motion = ShipMotion {
            x,
            y: 990.0,
            heading: 0.0,
            ..ShipMotion::default()
        };
        let start = (npc.motion.x, npc.motion.y);
        let dt = 1.0 / 30.0;
        for _ in 0..(30 * 60) {
            let input = avoid_land(&map, npc.motion, steer_input(npc.motion, 900.0, 990.0));
            sail(&map, &mut npc, input, dt);
        }
        let moved = distance(start.0, start.1, npc.motion.x, npc.motion.y);
        assert!(moved > 150.0, "ficou encalhado: andou {moved:.0} m");
    }

    #[test]
    fn land_avoidance_turns_toward_the_clear_side() {
        let map = WorldMap::vertical_slice();
        // Rumo leste contra a Ilha do Coral Negro, pelo norte dela: proa
        // bloqueada, bombordo (+40°) livre, boreste (-40°) na ilha.
        let motion = ShipMotion {
            x: -80.0,
            y: 990.0,
            heading: 0.0,
            ..ShipMotion::default()
        };
        let straight = MotionInput {
            throttle: 1.0,
            turn: 0.0,
        };
        let steered = avoid_land(&map, motion, straight);
        assert_eq!(steered.turn, 1.0);
        assert!(steered.throttle < 1.0);

        // Pelo sul da ilha: o lado livre é boreste.
        let below = ShipMotion { y: 900.0, ..motion };
        assert_eq!(avoid_land(&map, below, straight).turn, -1.0);

        // Água livre à frente: o input passa intacto.
        let open = ShipMotion {
            x: 0.0,
            y: -500.0,
            ..motion
        };
        assert_eq!(avoid_land(&map, open, straight), straight);
    }

    #[test]
    fn fleeing_caravan_opens_distance_from_the_attacker() {
        let mut npc = caravan(false);
        npc.motion = ShipMotion {
            x: 0.0,
            y: 0.0,
            heading: 0.0,
            ..ShipMotion::default()
        };
        let attacker = (60.0, 0.0);
        for i in 0..(30 * 6) {
            let secs = 12.0 - i as f32 / 30.0;
            let input = flee_input(npc.motion, attacker.0, attacker.1, secs);
            step_motion(&mut npc.motion, &npc.stats, input, &npc.tuning, 1.0 / 30.0);
        }
        let d = distance(npc.motion.x, npc.motion.y, attacker.0, attacker.1);
        assert!(d > 110.0, "d={d} at {:?}", npc.motion);
    }

    #[test]
    fn npc_spoils_are_only_raw_resources() {
        let dev = crate::net::DevItems::new();
        let spoils = raw_spoils(&dev, 15);
        assert_eq!(spoils, vec![(dev.timber, 8), (dev.ore, 7)]);
        assert_eq!(raw_spoils(&dev, 1), vec![(dev.timber, 1)]);
        assert!(caravan(false).hold.items().is_empty());
    }

    #[test]
    fn npc_at_weapon_range_fires_broadside() {
        let (side, _) = marvyr_domain_combat::aim_at(0.0, (0.0, 0.0), (0.0, 10.0));
        assert_eq!(side, BroadsideSide::Port);

        let mut battery = BroadsideBattery::default();
        assert!(battery.try_fire(side, 4.0));
        assert!(!battery.is_ready(side));
    }

    #[test]
    fn attacking_npc_turns_its_beam_toward_the_target() {
        let mut npc = npc_at((0.0, 0.0));
        npc.motion.speed = npc.stats.speed * 0.6;
        // Alvo à frente (+X): o NPC precisa virar até ficar de costado.
        let target = (150.0, 0.0);
        for _ in 0..(30 * 6) {
            let input = broadside_input(npc.motion, target.0, target.1);
            step_motion(&mut npc.motion, &npc.stats, input, &npc.tuning, 1.0 / 30.0);
        }
        let (_, aim) =
            marvyr_domain_combat::aim_at(npc.motion.heading, (npc.motion.x, npc.motion.y), target);
        assert!(aim.abs() < 0.45, "alvo deveria estar no través");
    }

    #[test]
    fn npc_damage_reduces_hp_and_destroy_marks_dead_without_cargo() {
        let mut npc = npc_at((0.0, 900.0));
        let start_hp = npc.hp;

        assert_eq!(
            apply_npc_damage(&mut npc, 10),
            DamageOutcome::Survived {
                remaining_hp: start_hp - 10
            }
        );
        assert_eq!(npc.hp, start_hp - 10);
        assert_ne!(npc.ai.state, NpcState::Dead);

        let lethal = npc.hp;
        apply_npc_damage(&mut npc, lethal);
        assert_eq!(npc.ai.state, NpcState::Dead);
        assert_eq!(npc.hp, 0);
        assert!(npc.hold.items().is_empty());
    }

    #[test]
    fn npc_ship_state_roundtrips_snapshot_flag() {
        let npc = npc_at((0.0, 900.0));
        let state = to_npc_ship_state(&npc, &ItemCatalog::default());

        assert!(state.is_npc);
        assert_eq!(state.ship_id, npc.ship_id);
        assert_eq!(state.kind, npc.kind);
        assert_eq!(state.hp, npc.hp);
    }
}
