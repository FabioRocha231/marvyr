//! Vida a bordo e no mar aberto (MV-061): tripulação, reparo em mar,
//! abordagem, mapas do tesouro, ilhas ocultas e os eventos de mundo.
//!
//! As regras são puras em `domain-ships`/`domain-combat`/`domain-world`;
//! aqui o servidor valida intents, aplica e avisa. Pilar 1: nada aqui cria
//! item útil — tesouro e kraken rendem recurso BRUTO, que ainda precisa
//! voltar ao porto e ser fabricado.

use bevy::prelude::*;
use lightyear::prelude::server::*;
use lightyear::prelude::*;
use marvyr_domain_combat::{rudder_points, HitZone};
use marvyr_domain_items::map_mod::{dig_secs, scaled_by, treasure_bonus_pct};
use marvyr_domain_items::{roll_map, ItemInstance, MapMod};
use marvyr_domain_ships::{
    casualties, crew_capacity, repair_step, Officer, Officers, VesselPresence, CREW_WAGE,
    CREW_WAGE_ITEM, REPAIR_COMBAT_LOCK_SECS, RUDDER_HP_MAX,
};
use marvyr_domain_world::treasure::{finds_map, island_for_map, DIG_MAX_SPEED, DIG_SECS};
use marvyr_domain_world::{
    DirectorChange, ResourceNode, SeaEvent, SeaEventDirector, SeaEventKind, WorldMap,
};
use marvyr_protocol::{
    ActionKind, ActionResult, BoardShip, DigTreasure, HireCrew, IslandState, IslandsInSight,
    NodesSnapshot, SeaEventState, SeaEventsUpdate, SetRepair, TreasureHint, TreasureHints,
    WorldEventKind,
};
use marvyr_shared::ids::{ItemInstanceId, ResourceNodeId};
use tracing::{info, warn};

use crate::net::{
    DevItems, Metrics, ReliableChannel, ServerRiskPolicy, ServerShip, ServerWorldMap,
    UnreliableChannel,
};
use crate::npc::{NpcRole, NpcShip};
use crate::sets::SimulationSet;

/// Estado de bordo que não é stat de equipamento.
#[derive(Debug, Clone, PartialEq)]
pub struct SeaCondition {
    pub rudder_hp: f32,
    pub crew: u16,
    /// A tripulação está no reparo (liga/desliga pela tecla K).
    pub repairing: bool,
    repair_clock: f32,
    /// Último golpe recebido (s de simulação): trava o reparo.
    pub last_hit_at: f32,
    pub dig: Option<Dig>,
    /// Segundos até a próxima tentativa de abordagem.
    pub board_cooldown: f32,
    /// Fração de casco acumulada pela Tormenta (aplicada em inteiros).
    pub tempest_wear: f32,
    /// v47: moral da tripulação (0..100). Não persiste: atracar enche.
    pub morale: f32,
    /// Segundos até a tripulação poder comer de novo.
    pub ration_cooldown: f32,
    /// v59: oficiais a bordo (persistem com a tripulação).
    pub officers: Officers,
}

/// Escavação em curso.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Dig {
    pub map: ItemInstanceId,
    pub island: u32,
    pub elapsed: f32,
    /// v26: duração (Rocha Dura dobra) e bônus do baú pelos modificadores.
    pub secs: f32,
    pub bonus_pct: u32,
}

impl SeaCondition {
    pub fn fresh(crew: u16) -> Self {
        Self {
            rudder_hp: RUDDER_HP_MAX,
            crew,
            repairing: false,
            repair_clock: 0.0,
            last_hit_at: f32::MIN,
            dig: None,
            board_cooldown: 0.0,
            tempest_wear: 0.0,
            morale: crate::morale::FULL,
            ration_cooldown: 0.0,
            officers: Officers::default(),
        }
    }

    /// Progresso da escavação para o HUD (0 = parado).
    pub fn dig_progress(&self) -> f32 {
        self.dig
            .map(|dig| (dig.elapsed / dig.secs).clamp(0.0, 1.0))
            .unwrap_or(0.0)
    }
}

/// Efeitos de bordo de um golpe: leme (se na popa), baixas e trava do
/// reparo. Tiro interrompe escavação.
pub fn take_hit(sea: &mut SeaCondition, damage: u32, max_hp: u32, zone: HitZone, now: f32) {
    sea.rudder_hp = (sea.rudder_hp - rudder_points(damage, max_hp, zone)).max(0.0);
    // v59: o cirurgião salva metade.
    sea.crew -= sea.officers.treat(casualties(damage, max_hp, sea.crew));
    sea.last_hit_at = now;
    sea.dig = None;
}

/// Alcance de abordagem (m) — costado com costado.
pub const BOARD_RANGE: f32 = 45.0;
/// Velocidade máxima (m/s) do atacante para lançar os ganchos.
const BOARD_MAX_SPEED: f32 = 6.0;
/// Alvo abordável: casco a até 35% ou parado.
const BOARD_HULL_RATIO: f32 = 0.35;
const BOARD_TARGET_STILL: f32 = 1.0;
const BOARD_COOLDOWN_SECS: f32 = 15.0;
/// Velocidade máxima (m/s) para a tripulação trabalhar no reparo.
const REPAIR_MAX_SPEED: f32 = 1.0;

/// NPCs tomados por abordagem, para o `simulate_npcs` pagar e retirar:
/// (id do NPC, navio atacante).
#[derive(Resource, Default)]
pub struct NpcBoardings(pub Vec<(u32, u32)>);

/// Recompensa do tesouro: recurso bruto de alto risco (Pilar 1).
const TREASURE_PEARLS: u32 = 3;
const TREASURE_AMBER: u32 = 2;
const TREASURE_CORAL: u32 = 6;

pub fn install(app: &mut App) {
    app.init_resource::<NpcBoardings>();
    crate::melee::install(app);
    app.init_resource::<MapPerils>();
    // O mapa já está no app (ServerNetPlugin o insere antes de instalar).
    let events = ServerSeaEvents::from_env(&app.world().resource::<ServerWorldMap>().0);
    app.insert_resource(events);
    app.add_systems(
        FixedUpdate,
        (
            handle_set_repair,
            handle_hire_crew,
            handle_hire_officer,
            handle_board,
            crate::melee::run_melees.after(handle_board),
            handle_dig,
        )
            .in_set(SimulationSet::Input),
    );
    app.add_systems(
        FixedUpdate,
        (tick_repair, tick_dig, run_sea_events, unleash_perils)
            .in_set(SimulationSet::EconomyConsequences),
    );
    app.add_systems(
        FixedUpdate,
        broadcast_sea_state.in_set(SimulationSet::Snapshot),
    );
}

pub(crate) fn send_action(
    connection_manager: &mut ConnectionManager,
    client_id: ClientId,
    action: ActionKind,
    success: bool,
    reason: impl Into<String>,
) {
    let _ = connection_manager.send_message::<ReliableChannel, _>(
        client_id,
        &ActionResult {
            action,
            success,
            reason: reason.into(),
        },
    );
}

/// Número aleatório do servidor (sorteios de abordagem e achados).
pub(crate) fn roll() -> u128 {
    uuid::Uuid::new_v4().as_u128()
}

fn handle_set_repair(
    mut events: EventReader<ServerReceiveMessage<SetRepair>>,
    mut connection_manager: ResMut<ConnectionManager>,
    mut ships: Query<&mut ServerShip>,
) {
    for event in events.read() {
        let client_id = event.from();
        let active = event.message().active;
        let Some(mut ship) = ships.iter_mut().find(|s| s.client_id == Some(client_id)) else {
            continue;
        };
        if active && matches!(ship.presence, VesselPresence::Docked(_)) {
            send_action(
                &mut connection_manager,
                client_id,
                ActionKind::Repair,
                false,
                "No porto o estaleiro já cuida do casco.",
            );
            continue;
        }
        ship.sea.repairing = active;
        ship.sea.repair_clock = 0.0;
        let reason = if active {
            "Tripulação ao reparo: arrie o pano, fique longe dos canhões e tenha Madeira no porão."
        } else {
            "Reparo suspenso."
        };
        send_action(
            &mut connection_manager,
            client_id,
            ActionKind::Repair,
            true,
            reason,
        );
    }
}

/// Um ciclo de reparo por segundo: consome Madeira do porão (material de
/// jogador — o mar não conserta ninguém de graça).
fn tick_repair(
    time: Res<Time>,
    dev: Res<DevItems>,
    mut connection_manager: ResMut<ConnectionManager>,
    mut ships: Query<&mut ServerShip>,
) {
    let dt = time.delta_secs();
    let now = time.elapsed_secs();
    for mut ship in &mut ships {
        if !ship.sea.repairing {
            continue;
        }
        ship.sea.repair_clock += dt;
        if ship.sea.repair_clock < 1.0 {
            continue;
        }
        ship.sea.repair_clock -= 1.0;
        if ship.motion.speed > REPAIR_MAX_SPEED
            || now - ship.sea.last_hit_at < REPAIR_COMBAT_LOCK_SECS
        {
            continue; // em movimento ou sob fogo: a tripulação espera
        }
        let timber: u32 = ship
            .hold
            .items()
            .iter()
            .filter(|custody| custody.instance.definition == dev.timber)
            .map(|custody| custody.instance.quantity)
            .sum();
        let Some(step) = repair_step(
            ship.hp,
            ship.stats.max_hp,
            ship.sea.rudder_hp,
            ship.sea.crew,
            timber,
        ) else {
            ship.sea.repairing = false;
            let reason = if ship.hp >= ship.stats.max_hp && ship.sea.rudder_hp >= RUDDER_HP_MAX {
                "Reparo concluído: casco e leme inteiros."
            } else if timber == 0 {
                "Reparo parado: sem Madeira no porão."
            } else {
                "Reparo parado: sem tripulação."
            };
            if let Some(client_id) = ship.client_id {
                send_action(
                    &mut connection_manager,
                    client_id,
                    ActionKind::Repair,
                    false,
                    reason,
                );
            }
            continue;
        };
        if ship.hold.remove(dev.timber, step.timber_used).is_err() {
            continue;
        }
        ship.hp = (ship.hp + step.hull_gain).min(ship.stats.max_hp);
        ship.sea.rudder_hp = (ship.sea.rudder_hp + step.rudder_gain).min(RUDDER_HP_MAX);
    }
}

fn handle_hire_crew(
    mut events: EventReader<ServerReceiveMessage<HireCrew>>,
    mut connection_manager: ResMut<ConnectionManager>,
    mut market: ResMut<crate::market::ServerMarket>,
    store: Res<crate::persist::StoreHandle>,
    dev: Res<DevItems>,
    mut ships: Query<&mut ServerShip>,
) {
    for event in events.read() {
        let client_id = event.from();
        let Some(mut ship) = ships.iter_mut().find(|s| s.client_id == Some(client_id)) else {
            continue;
        };
        let VesselPresence::Docked(region) = ship.presence else {
            send_action(
                &mut connection_manager,
                client_id,
                ActionKind::HireCrew,
                false,
                "Marujo se contrata no porto: atraque primeiro.",
            );
            continue;
        };
        let room = crew_capacity(ship.kind).saturating_sub(ship.sea.crew);
        let count = event.message().count.min(room);
        if count == 0 {
            send_action(
                &mut connection_manager,
                client_id,
                ActionKind::HireCrew,
                false,
                "Tripulação completa para este casco.",
            );
            continue;
        }
        // Soldo em madeira do armazém do porto (sink).
        let cost = CREW_WAGE * u32::from(count);
        if market
            .consume_from_storage(ship.character, region, dev.timber, cost)
            .is_err()
        {
            send_action(
                &mut connection_manager,
                client_id,
                ActionKind::HireCrew,
                false,
                format!("Falta {CREW_WAGE_ITEM} no armazém: {count} marujos custam {cost}."),
            );
            continue;
        }
        ship.sea.crew += count;
        // A madeira já saiu no banco: a tripulação paga vai junto, não no
        // próximo checkpoint.
        if let Some(store) = store.0.as_ref() {
            if let Err(error) = store.save_ship(&crate::net::ship_record(&ship)) {
                warn!(%error, "falha ao persistir tripulação contratada");
            }
        }
        let crew = ship.sea.crew;
        send_action(
            &mut connection_manager,
            client_id,
            ActionKind::HireCrew,
            true,
            format!("{count} marujos a bordo (-{cost} {CREW_WAGE_ITEM}). Tripulação: {crew}."),
        );
        info!(ship_id = ship.ship_id, count, crew, "tripulação contratada");
    }
}

/// v59: contrata um oficial no porto, pago com recurso do armazém
/// (artilheiro: minério; contramestre: madeira; cirurgião: coral).
fn handle_hire_officer(
    mut events: EventReader<ServerReceiveMessage<marvyr_protocol::HireOfficer>>,
    mut connection_manager: ResMut<ConnectionManager>,
    mut market: ResMut<crate::market::ServerMarket>,
    store: Res<crate::persist::StoreHandle>,
    dev: Res<DevItems>,
    mut ships: Query<&mut ServerShip>,
) {
    for event in events.read() {
        let client_id = event.from();
        let Some(mut ship) = ships.iter_mut().find(|s| s.client_id == Some(client_id)) else {
            continue;
        };
        let mut refuse = |text: String| {
            send_action(
                &mut connection_manager,
                client_id,
                ActionKind::HireCrew,
                false,
                text,
            );
        };
        let Some(officer) = Officer::from_code(event.message().officer) else {
            continue;
        };
        let VesselPresence::Docked(region) = ship.presence else {
            refuse(String::from(
                "Oficial se contrata no porto: atraque primeiro.",
            ));
            continue;
        };
        if ship.sea.officers.has(officer) {
            refuse(format!("Já há um {} a bordo.", officer.name()));
            continue;
        }
        let item = match officer {
            Officer::Gunner => dev.ore,
            Officer::Boatswain => dev.timber,
            Officer::Surgeon => dev.coral,
        };
        let item_name = dev
            .catalog
            .get(item)
            .map(|definition| definition.display_name.clone())
            .unwrap_or_default();
        let cost = officer.hire_cost();
        if market
            .consume_from_storage(ship.character, region, item, cost)
            .is_err()
        {
            refuse(format!(
                "Falta {item_name} no armazém: o {} custa {cost}.",
                officer.name()
            ));
            continue;
        }
        ship.sea.officers.add(officer);
        // O recurso já saiu no banco: o oficial vai junto (como o marujo).
        if let Some(store) = store.0.as_ref() {
            if let Err(error) = store.save_ship(&crate::net::ship_record(&ship)) {
                warn!(%error, "falha ao persistir oficial contratado");
            }
        }
        send_action(
            &mut connection_manager,
            client_id,
            ActionKind::HireCrew,
            true,
            format!("{} a bordo (-{cost} {item_name}).", officer.name()),
        );
        info!(
            ship_id = ship.ship_id,
            officer = officer.name(),
            "oficial contratado"
        );
    }
}

/// Alvo de abordagem visto do atacante.
#[derive(Debug, Clone, Copy)]
pub(crate) struct BoardTarget {
    pub(crate) ship_id: u32,
    pub(crate) x: f32,
    pub(crate) y: f32,
    speed: f32,
    pub(crate) hull_ratio: f32,
    pub(crate) crew: u16,
    pub(crate) officers: Officers,
    pub(crate) player: Option<ClientId>,
    pub(crate) npc: bool,
    monster: bool,
    docked: bool,
}

#[allow(clippy::too_many_arguments)]
fn handle_board(
    mut events: EventReader<ServerReceiveMessage<BoardShip>>,
    mut connection_manager: ResMut<ConnectionManager>,
    map: Res<ServerWorldMap>,
    risk: Res<ServerRiskPolicy>,
    mut ships: Query<&mut ServerShip>,
    npcs: Query<&NpcShip>,
    mut melees: ResMut<crate::melee::Melees>,
    time: Res<Time>,
    parties: Res<crate::party::Parties>,
) {
    let dt = time.delta_secs();
    for mut ship in &mut ships {
        ship.sea.board_cooldown = (ship.sea.board_cooldown - dt).max(0.0);
    }
    for event in events.read() {
        let client_id = event.from();
        let target_id = event.message().target_ship_id;
        let Some(attacker) = ships.iter().find(|s| s.client_id == Some(client_id)) else {
            continue;
        };
        let (attacker_id, ax, ay, a_speed, a_crew, a_cooldown, a_docked) = (
            attacker.ship_id,
            attacker.motion.x,
            attacker.motion.y,
            attacker.motion.speed,
            attacker.sea.crew,
            attacker.sea.board_cooldown,
            matches!(attacker.presence, VesselPresence::Docked(_)),
        );
        let target = ships
            .iter()
            .find(|s| s.ship_id == target_id && s.ship_id != attacker_id)
            .map(|s| BoardTarget {
                ship_id: s.ship_id,
                x: s.motion.x,
                y: s.motion.y,
                speed: s.motion.speed,
                hull_ratio: s.hp as f32 / s.stats.max_hp.max(1) as f32,
                crew: s.sea.crew,
                officers: s.sea.officers,
                player: s.client_id,
                npc: false,
                monster: false,
                docked: matches!(s.presence, VesselPresence::Docked(_)),
            })
            .or_else(|| {
                npcs.iter()
                    .find(|n| n.ship_id == target_id)
                    .map(|n| BoardTarget {
                        ship_id: n.ship_id,
                        x: n.motion.x,
                        y: n.motion.y,
                        speed: n.motion.speed,
                        hull_ratio: n.hp as f32 / n.max_hp.max(1) as f32,
                        crew: crew_capacity(n.kind),
                        // NPC não dá nada útil (pilar): sem oficial a capturar.
                        officers: Officers::default(),
                        player: None,
                        npc: true,
                        // Fortaleza também não: é muralha, não convés.
                        monster: n.role.is_monster() || n.role == NpcRole::Fort,
                        docked: false,
                    })
            });
        let refuse = |connection_manager: &mut ConnectionManager, reason: &str| {
            send_action(
                connection_manager,
                client_id,
                ActionKind::Board,
                false,
                reason,
            );
        };
        let Some(target) = target else {
            refuse(&mut connection_manager, "Nenhum navio para abordar.");
            continue;
        };
        // v55: companheiro de party não se aborda.
        let mates = ships
            .iter()
            .find(|s| s.ship_id == target.ship_id)
            .is_some_and(|mate| {
                ships
                    .iter()
                    .find(|s| s.ship_id == attacker_id)
                    .is_some_and(|me| parties.same_party(me.character, mate.character))
            });
        if mates {
            refuse(
                &mut connection_manager,
                "Aliado (party ou companhia) não se aborda.",
            );
            continue;
        }
        if a_docked || target.docked {
            refuse(&mut connection_manager, "Abordagem é coisa de mar aberto.");
            continue;
        }
        if target.monster {
            refuse(
                &mut connection_manager,
                "Monstro e fortaleza não se abordam: afunde-os.",
            );
            continue;
        }
        if a_cooldown > 0.0 {
            refuse(&mut connection_manager, "A tripulação ainda se reagrupa.");
            continue;
        }
        let pvp_here = map
            .0
            .zone_at(target.x, target.y)
            .is_ok_and(|zone| risk.0.pvp_allowed(zone.tier));
        if !pvp_here {
            refuse(
                &mut connection_manager,
                "Águas protegidas: sem abordagem aqui.",
            );
            continue;
        }
        let (dx, dy) = (target.x - ax, target.y - ay);
        if dx * dx + dy * dy > BOARD_RANGE * BOARD_RANGE {
            refuse(
                &mut connection_manager,
                "Encoste o costado no alvo para abordar.",
            );
            continue;
        }
        if a_speed > BOARD_MAX_SPEED {
            refuse(
                &mut connection_manager,
                "Rápido demais para lançar os ganchos.",
            );
            continue;
        }
        if target.hull_ratio > BOARD_HULL_RATIO && target.speed > BOARD_TARGET_STILL {
            refuse(
                &mut connection_manager,
                "O alvo ainda manobra: avarie o casco ou pare o navio dele.",
            );
            continue;
        }
        if a_crew == 0 {
            refuse(&mut connection_manager, "Sem tripulação para abordar.");
            continue;
        }
        if melees.busy(attacker_id) || melees.busy(target.ship_id) {
            refuse(&mut connection_manager, "Já há luta nesse convés.");
            continue;
        }
        if let Some(mut attacker) = ships.iter_mut().find(|s| s.ship_id == attacker_id) {
            attacker.sea.board_cooldown = BOARD_COOLDOWN_SECS;
        }
        // v61: ganchos presos — o resto é o duelo de táticas (`melee`).
        melees.start(attacker_id, client_id, a_crew, target);
        info!(
            attacker = attacker_id,
            target = target.ship_id,
            "abordagem: duelo começou"
        );
        send_action(
            &mut connection_manager,
            client_id,
            ActionKind::Board,
            true,
            "Ganchos presos! 1 Assalto · 2 Mosquete · 3 Muralha",
        );
        if let Some(victim) = target.player {
            crate::reputation::send_event(
                &mut connection_manager,
                &[victim],
                String::from("Abordagem! Defenda o convés: 1 Assalto · 2 Mosquete · 3 Muralha"),
                WorldEventKind::Alert,
            );
        }
    }
}

fn handle_dig(
    time: Res<Time>,
    mut perils: ResMut<MapPerils>,
    mut events: EventReader<ServerReceiveMessage<DigTreasure>>,
    mut connection_manager: ResMut<ConnectionManager>,
    dev: Res<DevItems>,
    world: Res<ServerWorldMap>,
    mut ships: Query<&mut ServerShip>,
) {
    for event in events.read() {
        let client_id = event.from();
        let Some(mut ship) = ships.iter_mut().find(|s| s.client_id == Some(client_id)) else {
            continue;
        };
        let refuse = |connection_manager: &mut ConnectionManager, reason: &str| {
            send_action(
                connection_manager,
                client_id,
                ActionKind::Dig,
                false,
                reason,
            );
        };
        if matches!(ship.presence, VesselPresence::Docked(_)) {
            refuse(&mut connection_manager, "Tesouro não se cava no porto.");
            continue;
        }
        let maps: Vec<(ItemInstanceId, Vec<MapMod>)> = ship
            .hold
            .items()
            .iter()
            .filter(|custody| custody.instance.definition == dev.treasure_map)
            .map(|custody| (custody.instance.id, custody.instance.map_mods().to_vec()))
            .collect();
        if maps.is_empty() {
            refuse(&mut connection_manager, "Sem Mapa do Tesouro no porão.");
            continue;
        }
        let (x, y) = (ship.motion.x, ship.motion.y);
        let Some((map, mods, island)) = maps.iter().find_map(|(map, mods)| {
            let island = island_for_map(&world.0.features().hidden_islands, map.0.as_u128())?;
            island
                .at_dig_spot(x, y)
                .then_some((*map, mods.clone(), island))
        }) else {
            refuse(
                &mut connection_manager,
                "Nenhum dos seus mapas marca este lugar. Siga o X no mapa.",
            );
            continue;
        };
        if ship.motion.speed > DIG_MAX_SPEED {
            refuse(
                &mut connection_manager,
                "Pare o navio para lançar o escaler.",
            );
            continue;
        }
        ship.sea.dig = Some(Dig {
            map,
            island: island.id,
            elapsed: 0.0,
            secs: dig_secs(DIG_SECS, &mods),
            bonus_pct: treasure_bonus_pct(&mods),
        });
        // v26: os perigos do mapa acordam na descida do escaler (cancelar
        // e recomeçar enquanto a leva vive não chama outra).
        if !mods.is_empty() && perils.wake(map, &mods, time.elapsed_secs()) {
            perils.pending.push(Peril {
                mods: mods.clone(),
                island: island.id,
                client_id,
            });
        }
        send_action(
            &mut connection_manager,
            client_id,
            ActionKind::Dig,
            true,
            format!("Escaler na praia de {}: cavando…", island.name),
        );
    }
}

fn tick_dig(
    time: Res<Time>,
    dev: Res<DevItems>,
    world: Res<ServerWorldMap>,
    mut connection_manager: ResMut<ConnectionManager>,
    mut metrics: ResMut<Metrics>,
    mut ships: Query<&mut ServerShip>,
    mut discoveries: EventWriter<crate::progress::Discovered>,
) {
    let dt = time.delta_secs();
    for mut ship in &mut ships {
        let Some(mut dig) = ship.sea.dig else {
            continue;
        };
        if ship.motion.speed > DIG_MAX_SPEED {
            ship.sea.dig = None;
            if let Some(client_id) = ship.client_id {
                send_action(
                    &mut connection_manager,
                    client_id,
                    ActionKind::Dig,
                    false,
                    "O navio andou: o escaler voltou sem o tesouro.",
                );
            }
            continue;
        }
        dig.elapsed += dt;
        if dig.elapsed < dig.secs {
            ship.sea.dig = Some(dig);
            continue;
        }
        ship.sea.dig = None;
        if ship.hold.remove_instance(dig.map).is_none() {
            continue; // o mapa saiu do porão no meio do caminho
        }
        let island = world
            .0
            .features()
            .hidden_islands
            .iter()
            .find(|island| island.id == dig.island)
            .map(|island| island.name)
            .unwrap_or("ilha sem nome");
        let mut left_behind = false;
        for (item, quantity) in [
            (dev.abyssal_pearl, TREASURE_PEARLS),
            (dev.abyssal_amber, TREASURE_AMBER),
            (dev.coral, TREASURE_CORAL),
        ] {
            let quantity = scaled_by(quantity, dig.bonus_pct);
            let found = ItemInstance::new_resource(ItemInstanceId::new(), item, quantity);
            left_behind |= ship.hold.insert(&dev.catalog, found).is_err();
        }
        metrics.treasures_dug += 1;
        discoveries.send(crate::progress::Discovered {
            character: ship.character,
            entry: "Tesouro desenterrado",
        });
        info!(
            ship_id = ship.ship_id,
            island, left_behind, "tesouro desenterrado"
        );
        let bonus = if dig.bonus_pct > 0 {
            format!(" O perigo pagou: +{}% no baú.", dig.bonus_pct)
        } else {
            String::new()
        };
        let reason = if left_behind {
            format!("Tesouro de {island} desenterrado!{bonus} Porão cheio: parte ficou na areia.")
        } else {
            format!("Tesouro de {island} desenterrado!{bonus} Leve-o vivo até um porto.")
        };
        if let Some(client_id) = ship.client_id {
            send_action(
                &mut connection_manager,
                client_id,
                ActionKind::Dig,
                true,
                reason,
            );
        }
        // Rumor: o mar inteiro fica sabendo que tem riqueza navegando.
        let _ = connection_manager.send_message_to_target::<ReliableChannel, _>(
            &marvyr_protocol::WorldEvent {
                text: format!("Rumor: um tesouro foi desenterrado em {island}!"),
                kind: WorldEventKind::Alert,
            },
            NetworkTarget::All,
        );
    }
}

/// Chance de coleta trazer mapa do tesouro (chamado pelo `handle_gather`).
/// Devolve a raridade/perigos do mapa que entrou no porão (`Some(None)` =
/// mapa Normal). v26: o mapa já vem sorteado, semente = id do mapa.
pub(crate) fn maybe_find_map(
    ship: &mut ServerShip,
    dev: &DevItems,
    tries: u32,
) -> Option<Option<marvyr_domain_items::Quality>> {
    if !(0..tries).any(|_| finds_map(roll())) {
        return None;
    }
    let id = ItemInstanceId::new();
    let map = ItemInstance {
        quality: roll_map(id.0.as_u64_pair().0),
        ..ItemInstance::new_resource(id, dev.treasure_map, 1)
    };
    let quality = map.quality.clone();
    ship.hold.insert(&dev.catalog, map).ok().map(|()| quality)
}

/// v26: perigos de mapa esperando nascer, os que já acordaram (um por
/// mapa) e os monstros que eles trouxeram, com hora de sumir.
#[derive(Resource, Default)]
pub struct MapPerils {
    pending: Vec<Peril>,
    /// Mapa → (perigos daquela leva, hora em que acordaram). Cavar de novo
    /// depois que os monstros sumiram, ou com o mapa re-sorteado pelo
    /// Cartógrafo, acorda outra leva: o bônus do baú nunca sai de graça.
    triggered: std::collections::HashMap<ItemInstanceId, (Vec<MapMod>, f32)>,
    spawned: Vec<(u32, f32)>,
}

impl MapPerils {
    /// NPC que um mapa trouxe (não é do evento de mundo de mesmo papel).
    pub fn owns(&self, npc_id: u32) -> bool {
        self.spawned.iter().any(|(id, _)| *id == npc_id)
    }

    /// `true` = esta descida acorda os perigos (e marca a hora).
    fn wake(&mut self, map: ItemInstanceId, mods: &[MapMod], now: f32) -> bool {
        self.triggered
            .retain(|_, (_, woke)| now - *woke < PERIL_NPC_SECS);
        let awake = self
            .triggered
            .get(&map)
            .is_some_and(|(woke_mods, _)| woke_mods.as_slice() == mods);
        if awake {
            return false;
        }
        self.triggered.insert(map, (mods.to_vec(), now));
        true
    }
}

struct Peril {
    mods: Vec<MapMod>,
    island: u32,
    client_id: ClientId,
}

/// Onde nascem os monstros do mapa: do lado do mar da ilha (do centro
/// para o ponto de escavação, e além), dois guardiões abertos e o Kraken
/// mais ao largo.
fn peril_npcs(
    map: &WorldMap,
    island: &marvyr_domain_world::treasure::HiddenIsland,
    mods: &[MapMod],
) -> Vec<(NpcRole, Vec2)> {
    let dig = Vec2::new(island.dig_x, island.dig_y);
    let seaward = (dig - Vec2::new(island.x, island.y)).normalize_or(Vec2::X);
    let across = seaward.perp();
    let mut out = Vec::new();
    if mods.contains(&MapMod::Guarded) {
        for side in [-1.0, 1.0] {
            let want = dig + seaward * 170.0 + across * 130.0 * side;
            out.push((NpcRole::Guardian, water_near(map, dig, want)));
        }
    }
    if mods.contains(&MapMod::Kraken) {
        out.push((NpcRole::Kraken, water_near(map, dig, dig + seaward * 260.0)));
    }
    out
}

/// Água perto de `want`: o próprio ponto, depois recuando até a praia
/// (`dig`, que é sempre mar), depois em volta dela. Ilha colada em outra
/// terra não bota monstro em cima do morro.
pub(crate) fn water_near(map: &WorldMap, dig: Vec2, want: Vec2) -> Vec2 {
    let wet = |at: Vec2| {
        map.push_out_of_land(at.x, at.y, crate::net::HULL_CLEARANCE)
            .is_none()
    };
    let reach = want.distance(dig);
    let pulled = (0..8).map(|step| dig.lerp(want, 1.0 - step as f32 / 8.0));
    let around = (0..16).flat_map(|turn| {
        let angle = turn as f32 / 16.0 * std::f32::consts::TAU;
        [1.0, 0.6].map(|scale| dig + Vec2::from_angle(angle) * reach * scale)
    });
    pulled.chain(around).find(|at| wet(*at)).unwrap_or(dig)
}

/// Quanto os guardiões e o Kraken de mapa ficam antes de sumir no mar.
const PERIL_NPC_SECS: f32 = 300.0;

/// Materializa os perigos: guardiões e Kraken nascem do lado do mar da
/// ilha (a partir do ponto de escavação, longe do centro), a tormenta cai
/// sobre a praia e a boca solta avisa o mar inteiro.
#[allow(clippy::too_many_arguments)]
fn unleash_perils(
    mut commands: Commands,
    time: Res<Time>,
    mut perils: ResMut<MapPerils>,
    mut connection_manager: ResMut<ConnectionManager>,
    mut weather: ResMut<crate::weather::ServerWeather>,
    (dev_ships, map, config): (
        Res<crate::crafting::DevShips>,
        Res<ServerWorldMap>,
        Res<crate::npc::NpcSpawnConfig>,
    ),
    mut npc_ids: ResMut<crate::npc::NpcIdCounter>,
    npcs: Query<(Entity, &NpcShip)>,
) {
    let now = time.elapsed_secs();
    // Monstros de mapa que passaram da hora voltam para o fundo.
    perils.spawned.retain(|(npc_id, despawn_at)| {
        if now < *despawn_at {
            return true;
        }
        if let Some((entity, _)) = npcs.iter().find(|(_, npc)| npc.ship_id == *npc_id) {
            commands.entity(entity).despawn();
        }
        false
    });
    for peril in std::mem::take(&mut perils.pending) {
        let Some(island) = map
            .0
            .features()
            .hidden_islands
            .iter()
            .find(|island| island.id == peril.island)
        else {
            continue;
        };
        let dig = Vec2::new(island.dig_x, island.dig_y);
        let mut spawn = |role, at: Vec2, perils: &mut MapPerils| {
            let (id, npc) = crate::npc::build_npc(
                &dev_ships,
                &map.0,
                &config,
                &mut npc_ids,
                role,
                (at.x, at.y),
            );
            commands.spawn((npc,));
            perils.spawned.push((id, now + PERIL_NPC_SECS));
        };
        let mut woke = Vec::new();
        for (role, at) in peril_npcs(&map.0, island, &peril.mods) {
            spawn(role, at, &mut perils);
        }
        for m in &peril.mods {
            match m {
                MapMod::Guarded | MapMod::Kraken | MapMod::Bedrock => {}
                MapMod::Tempest => {
                    weather.0.spawn_tempest_at(dig.x, dig.y, 260.0, 60.0);
                }
                MapMod::Rumored => {
                    let _ = connection_manager.send_message_to_target::<ReliableChannel, _>(
                        &marvyr_protocol::WorldEvent {
                            text: format!(
                                "Boca solta: tem gente cavando tesouro em {}!",
                                island.name
                            ),
                            kind: WorldEventKind::Alert,
                        },
                        NetworkTarget::All,
                    );
                }
            }
            woke.push(m.label());
        }
        info!(island = island.name, mods = ?peril.mods, "perigos do mapa acordaram");
        crate::reputation::send_event(
            &mut connection_manager,
            &[peril.client_id],
            format!("O mapa cobra o preço: {}!", woke.join(", ")),
            WorldEventKind::Alert,
        );
    }
}

/// Eventos de mundo em curso e o que o servidor materializou para eles.
#[derive(Resource)]
pub struct ServerSeaEvents {
    pub director: SeaEventDirector,
    /// Dev: evento forçado no boot (`MARVYR_SEA_EVENT`).
    forced: Option<SeaEventKind>,
    tide_node: Option<(Entity, u32)>,
    broadcast_clock: f32,
}

impl ServerSeaEvents {
    pub fn new(seed: u64, map: &WorldMap) -> Self {
        Self {
            director: SeaEventDirector::new(seed, map),
            forced: None,
            tide_node: None,
            broadcast_clock: 0.0,
        }
    }

    /// Semente pelo relógio; `MARVYR_SEA_EVENT=kraken|fleet|tempest|tide`
    /// força um evento no primeiro tick (dev/playtest).
    fn from_env(map: &WorldMap) -> Self {
        let seed = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_nanos() as u64)
            .unwrap_or(0);
        let mut events = Self::new(seed, map);
        events.forced = std::env::var("MARVYR_SEA_EVENT")
            .ok()
            .and_then(|value| parse_event_kind(&value));
        events
    }

    /// Teste/dev: força um evento no próximo tick.
    pub fn force(&mut self, kind: SeaEventKind) {
        self.forced = Some(kind);
    }
}

fn parse_event_kind(value: &str) -> Option<SeaEventKind> {
    match value.trim() {
        "kraken" => Some(SeaEventKind::Kraken),
        "fleet" => Some(SeaEventKind::TreasureFleet),
        "tempest" => Some(SeaEventKind::Tempest),
        "tide" => Some(SeaEventKind::ContestedTide),
        "blood" => Some(SeaEventKind::BloodTide),
        _ => None,
    }
}

fn wire_kind(kind: SeaEventKind) -> marvyr_protocol::SeaEventKind {
    match kind {
        SeaEventKind::Tempest => marvyr_protocol::SeaEventKind::Tempest,
        SeaEventKind::TreasureFleet => marvyr_protocol::SeaEventKind::TreasureFleet,
        SeaEventKind::Kraken => marvyr_protocol::SeaEventKind::Kraken,
        SeaEventKind::ContestedTide => marvyr_protocol::SeaEventKind::ContestedTide,
        SeaEventKind::BloodTide => marvyr_protocol::SeaEventKind::BloodTide,
    }
}

fn announcement(kind: SeaEventKind) -> &'static str {
    match kind {
        SeaEventKind::Tempest => "Uma Tormenta se forma no mar aberto: cascos vão rachar.",
        SeaEventKind::TreasureFleet => {
            "Frota do Tesouro avistada ao sul! Carga pesada sob escolta da coroa."
        }
        SeaEventKind::Kraken => "Kraken avistado nas águas sem lei! A coroa paga pela cabeça.",
        SeaEventKind::ContestedTide => {
            "Maré de Pérolas no mar sem lei: recife rico por pouco tempo."
        }
        SeaEventKind::BloodTide => {
            "A Maré Sangrenta subiu! Saqueadores de elite, cinzas e baús malditos."
        }
    }
}

fn farewell(kind: SeaEventKind) -> &'static str {
    match kind {
        SeaEventKind::Tempest => "A Tormenta se desfez.",
        SeaEventKind::TreasureFleet => "A Frota do Tesouro deixou as águas.",
        SeaEventKind::Kraken => "O Kraken voltou às profundezas.",
        SeaEventKind::ContestedTide => "A Maré de Pérolas baixou.",
        SeaEventKind::BloodTide => "A Maré Sangrenta baixou. Os baús fechados afundaram.",
    }
}

/// Estoque de pérolas da Maré.
const TIDE_STOCK: u32 = 40;

#[allow(clippy::too_many_arguments)]
fn run_sea_events(
    mut commands: Commands,
    time: Res<Time>,
    mut events: ResMut<ServerSeaEvents>,
    mut connection_manager: ResMut<ConnectionManager>,
    mut weather: ResMut<crate::weather::ServerWeather>,
    mut metrics: ResMut<Metrics>,
    (dev, dev_ships, map, config): (
        Res<DevItems>,
        Res<crate::crafting::DevShips>,
        Res<ServerWorldMap>,
        Res<crate::npc::NpcSpawnConfig>,
    ),
    (mut npc_ids, mut node_ids): (
        ResMut<crate::npc::NpcIdCounter>,
        ResMut<crate::nodes::NodeIdCounter>,
    ),
    npcs: Query<(Entity, &NpcShip)>,
    perils: Res<MapPerils>,
) {
    let mut changes = events.director.step(time.delta_secs());
    if let Some(kind) = events.forced.take() {
        if let Some(active) = events.director.active().copied() {
            changes.push(DirectorChange::Ended(active));
        }
        changes.push(DirectorChange::Started(events.director.force(kind)));
    }
    for change in changes {
        match change {
            DirectorChange::Started(event) => {
                metrics.sea_events_started += 1;
                info!(kind = ?event.kind, x = event.x, y = event.y, "evento de mundo começou");
                match event.kind {
                    SeaEventKind::Tempest => {
                        weather.0.spawn_tempest_at(
                            event.x,
                            event.y,
                            event.radius,
                            event.kind.duration(),
                        );
                    }
                    SeaEventKind::TreasureFleet => {
                        crate::npc::spawn_treasure_fleet(
                            &mut commands,
                            &dev_ships,
                            &map.0,
                            &config,
                            &mut npc_ids,
                        );
                    }
                    SeaEventKind::Kraken => {
                        crate::npc::spawn_kraken(
                            &mut commands,
                            &dev_ships,
                            &map.0,
                            &config,
                            &mut npc_ids,
                            (event.x, event.y),
                        );
                    }
                    SeaEventKind::ContestedTide => {
                        let node = spawn_tide(&mut commands, &dev, &map.0, &mut node_ids, &event);
                        if let Some(state) = crate::nodes::node_state(&node.1, node.2, &dev.catalog)
                        {
                            let _ = connection_manager
                                .send_message_to_target::<ReliableChannel, _>(
                                    &NodesSnapshot { nodes: vec![state] },
                                    NetworkTarget::All,
                                );
                        }
                        events.tide_node = Some((node.0, node.2));
                    }
                    // Ondas, cinzas e baús: `blood_tide::run_blood_tide`.
                    SeaEventKind::BloodTide => {}
                }
                announce(&mut connection_manager, announcement(event.kind));
            }
            DirectorChange::Ended(event) => {
                info!(kind = ?event.kind, "evento de mundo terminou");
                match event.kind {
                    SeaEventKind::Tempest | SeaEventKind::BloodTide => {}
                    SeaEventKind::TreasureFleet | SeaEventKind::Kraken => {
                        for (entity, npc) in &npcs {
                            if npc.role.is_event_npc()
                                && event_owns(event.kind, npc.role)
                                && !perils.owns(npc.ship_id)
                            {
                                commands.entity(entity).despawn();
                            }
                        }
                    }
                    SeaEventKind::ContestedTide => {
                        if let Some((entity, node_num)) = events.tide_node.take() {
                            commands.entity(entity).despawn();
                            let _ = connection_manager
                                .send_message_to_target::<ReliableChannel, _>(
                                    &marvyr_protocol::NodeUpdated {
                                        node: marvyr_protocol::NodeState {
                                            node_id: node_num,
                                            x: event.x,
                                            y: event.y,
                                            resource_name: String::from("Maré baixou"),
                                            stock: 0,
                                            max_stock: TIDE_STOCK,
                                            golden: false,
                                        },
                                    },
                                    NetworkTarget::All,
                                );
                        }
                    }
                }
                announce(&mut connection_manager, farewell(event.kind));
            }
        }
    }
}

fn event_owns(kind: SeaEventKind, role: NpcRole) -> bool {
    match kind {
        SeaEventKind::Kraken => role == NpcRole::Kraken,
        SeaEventKind::TreasureFleet => {
            matches!(role, NpcRole::TreasureGalleon | NpcRole::Escort)
        }
        _ => false,
    }
}

fn announce(connection_manager: &mut ConnectionManager, text: &str) {
    let _ = connection_manager.send_message_to_target::<ReliableChannel, _>(
        &marvyr_protocol::WorldEvent {
            text: text.to_owned(),
            kind: WorldEventKind::Alert,
        },
        NetworkTarget::All,
    );
}

fn spawn_tide(
    commands: &mut Commands,
    dev: &DevItems,
    map: &marvyr_domain_world::WorldMap,
    node_ids: &mut crate::nodes::NodeIdCounter,
    event: &SeaEvent,
) -> (Entity, ResourceNode, u32) {
    let region = map
        .region_by_name("Ilha do Coral Negro")
        .map(|region| region.id)
        .expect("mapa do slice declara a ilha");
    let node_num = node_ids.0;
    node_ids.0 += 1;
    let node = ResourceNode {
        id: ResourceNodeId::new(),
        name: "Maré de Pérolas",
        x: event.x,
        y: event.y,
        region,
        resource: dev.abyssal_pearl,
        stock: TIDE_STOCK,
        max_stock: TIDE_STOCK,
    };
    let entity = commands
        .spawn((crate::nodes::ServerNode {
            node_num,
            node: node.clone(),
            // Maré não repovoa: acabou, acabou.
            respawn_at: None,
            golden: false,
        },))
        .id();
    (entity, node, node_num)
}

/// ~1 Hz: evento em curso (todos), ilhas à vista e pistas dos mapas (cada
/// capitão). Canal não-confiável: o próximo pulso substitui o perdido.
#[allow(clippy::too_many_arguments)]
fn broadcast_sea_state(
    time: Res<Time>,
    mut events: ResMut<ServerSeaEvents>,
    mut connection_manager: ResMut<ConnectionManager>,
    dev: Res<DevItems>,
    world: Res<ServerWorldMap>,
    ships: Query<&ServerShip>,
    npcs: Query<&NpcShip>,
    (blood, boss, abyss, perils): (
        Res<crate::blood_tide::BloodTide>,
        Res<crate::world_boss::WorldBoss>,
        Res<crate::abyss::Abyss>,
        Res<MapPerils>,
    ),
    mut discoveries: EventWriter<crate::progress::Discovered>,
) {
    let hidden = &world.0.features().hidden_islands;
    events.broadcast_clock += time.delta_secs();
    if events.broadcast_clock < 1.0 {
        return;
    }
    events.broadcast_clock = 0.0;
    let mut active: Vec<SeaEventState> = events
        .director
        .active()
        .map(|event| {
            // A frota anda: a área anunciada segue o galeão.
            let (x, y) = match event.kind {
                SeaEventKind::TreasureFleet => npcs
                    .iter()
                    .find(|npc| npc.role == NpcRole::TreasureGalleon)
                    .map(|npc| (npc.motion.x, npc.motion.y))
                    .unwrap_or((event.x, event.y)),
                SeaEventKind::Kraken => npcs
                    .iter()
                    .find(|npc| npc.role == NpcRole::Kraken && !perils.owns(npc.ship_id))
                    .map(|npc| (npc.motion.x, npc.motion.y))
                    .unwrap_or((event.x, event.y)),
                _ => (event.x, event.y),
            };
            SeaEventState {
                event_id: event.id,
                kind: wire_kind(event.kind),
                name: event.kind.name().to_owned(),
                x,
                y,
                radius: event.radius,
                remaining_secs: event.remaining.max(0.0),
                chests: if event.kind == SeaEventKind::BloodTide {
                    blood.chests.clone()
                } else {
                    Vec::new()
                },
            }
        })
        .into_iter()
        .collect();
    // v34: o Leviatã corre em paralelo ao diretor.
    active.extend(boss.wire(&npcs));
    active.extend(crate::cursed_cargo::wire(&ships));
    active.extend(abyss.wire(&world.0));
    let _ = connection_manager.send_message_to_target::<UnreliableChannel, _>(
        &SeaEventsUpdate { events: active },
        NetworkTarget::All,
    );
    for ship in &ships {
        let Some(client_id) = ship.client_id else {
            continue;
        };
        let (x, y) = (ship.motion.x, ship.motion.y);
        let islands: Vec<IslandState> = hidden
            .iter()
            .filter(|island| island.in_sight(x, y))
            .map(|island| IslandState {
                island_id: island.id,
                name: island.name.to_owned(),
                x: island.x,
                y: island.y,
                radius: island.radius,
            })
            .collect();
        if !islands.is_empty() {
            discoveries.send(crate::progress::Discovered {
                character: ship.character,
                entry: "Ilha oculta",
            });
        }
        let _ = connection_manager
            .send_message::<UnreliableChannel, _>(client_id, &IslandsInSight { islands });
        let hints = ship
            .hold
            .items()
            .iter()
            .filter(|custody| custody.instance.definition == dev.treasure_map)
            .filter_map(|custody| {
                let island = island_for_map(hidden, custody.instance.id.0.as_u128())?;
                Some(TreasureHint {
                    x: island.dig_x,
                    y: island.dig_y,
                    island: island.name.to_owned(),
                    rarity: custody.instance.rarity(),
                    mods: custody.instance.map_mods().to_vec(),
                    bonus_pct: treasure_bonus_pct(custody.instance.map_mods()),
                })
            })
            .collect();
        let _ = connection_manager
            .send_message::<UnreliableChannel, _>(client_id, &TreasureHints { hints });
    }
}

/// Recompensa do kraken abatido: só recurso bruto de alto risco num wreck
/// (vira carga de jogador, que ainda precisa chegar ao porto). Mapa não —
/// NPC não dá item útil (pilar 1). A cabeça do monstro é paga pela coroa
/// (`NpcBounty`).
pub(crate) fn kraken_spoils(dev: &DevItems) -> Vec<(marvyr_shared::ids::ItemDefinitionId, u32)> {
    vec![(dev.abyssal_pearl, 5), (dev.abyssal_amber, 3)]
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn map_perils_wake_again_after_they_leave_or_the_map_changes() {
        let mut perils = MapPerils::default();
        let map = ItemInstanceId::new();
        let guarded = [MapMod::Guarded];
        assert!(perils.wake(map, &guarded, 0.0), "primeira descida acorda");
        assert!(
            !perils.wake(map, &guarded, 10.0),
            "recomeçar com a leva viva não"
        );
        assert!(
            perils.wake(map, &[MapMod::Kraken], 20.0),
            "mapa re-sorteado acorda a leva nova"
        );
        assert!(
            perils.wake(map, &[MapMod::Kraken], 20.0 + PERIL_NPC_SECS),
            "monstros sumiram: cavar de novo chama outra leva"
        );
    }

    #[test]
    fn map_perils_rise_from_the_water_in_every_world() {
        let mods = [MapMod::Guarded, MapMod::Kraken];
        for seed in [0, crate::net::DEFAULT_WORLD_SEED, 7, 1234] {
            let map = WorldMap::from_seed(seed).with_hidden_islands();
            let islands = &map.features().hidden_islands;
            assert!(!islands.is_empty(), "seed {seed}");
            for island in islands {
                let npcs = peril_npcs(&map, island, &mods);
                assert_eq!(npcs.len(), 3);
                for (role, at) in npcs {
                    assert!(
                        map.push_out_of_land(at.x, at.y, 12.0).is_none(),
                        "seed {seed}: {role:?} de {} nasce em terra",
                        island.name
                    );
                }
            }
        }
        let classic = WorldMap::from_seed(0).with_hidden_islands();
        let first = &classic.features().hidden_islands[0];
        assert!(peril_npcs(&classic, first, &[MapMod::Tempest]).is_empty());
    }

    #[test]
    fn stern_hit_breaks_rudder_kills_crew_and_stops_digging() {
        let mut sea = SeaCondition::fresh(8);
        sea.dig = Some(Dig {
            map: ItemInstanceId::new(),
            island: 1,
            elapsed: 3.0,
            secs: DIG_SECS,
            bonus_pct: 0,
        });
        take_hit(&mut sea, 60, 200, HitZone::Stern, 10.0);
        assert!(sea.rudder_hp < RUDDER_HP_MAX);
        assert_eq!(sea.crew, 6);
        assert_eq!(sea.last_hit_at, 10.0);
        assert!(sea.dig.is_none());
        let mut midship = SeaCondition::fresh(8);
        take_hit(&mut midship, 10, 200, HitZone::Midship, 1.0);
        assert_eq!(midship.rudder_hp, RUDDER_HP_MAX);
    }

    #[test]
    fn dig_progress_reports_fraction() {
        let mut sea = SeaCondition::fresh(4);
        assert_eq!(sea.dig_progress(), 0.0);
        sea.dig = Some(Dig {
            map: ItemInstanceId::new(),
            island: 1,
            elapsed: DIG_SECS / 2.0,
            secs: DIG_SECS,
            bonus_pct: 0,
        });
        assert!((sea.dig_progress() - 0.5).abs() < 1e-6);
    }

    #[test]
    fn dev_event_names_parse() {
        assert_eq!(parse_event_kind("kraken"), Some(SeaEventKind::Kraken));
        assert_eq!(parse_event_kind("tide"), Some(SeaEventKind::ContestedTide));
        assert_eq!(parse_event_kind("nada"), None);
    }

    #[test]
    fn treasure_map_is_a_quest_item_in_the_catalog() {
        let dev = DevItems::new();
        let map = dev.catalog.get(dev.treasure_map).expect("mapa no catálogo");
        assert_eq!(map.kind, marvyr_domain_items::ItemKind::Quest);
        assert_eq!(map.max_stack, 1);
    }
}
