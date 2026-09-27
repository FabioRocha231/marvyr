//! v40: o Abismo — fim de jogo sem teto (o Pit do D4). Uma Boca fixa no mar
//! sem lei; quem entra no anel começa a descer. Cada camada é uma onda de
//! saqueadores de elite mais fortes que a anterior, com 90 s para limpar.
//! Limpou: destroço só do mergulhador com bruto que cresce com a
//! profundidade (Pilar 1) e a próxima camada. Estourou o tempo, afundou,
//! atracou ou fugiu do anel: o Abismo cospe o capitão de volta.
//!
//! Sem instância: outros capitães veem a descida e podem ajudar — ou
//! esperar o mergulhador sair carregado.

use std::collections::HashMap;

use bevy::prelude::*;
use lightyear::prelude::server::*;
use marvyr_domain_ships::VesselPresence;
use marvyr_domain_world::WorldMap;
use marvyr_protocol::{ActionKind, SeaEventKind, SeaEventState};
use marvyr_shared::ids::{CharacterId, ItemDefinitionId};
use tracing::info;

use crate::net::{DevItems, ServerShip, ServerWorldMap};
use crate::npc::{NpcRole, NpcShip, NpcState};
use crate::sets::SimulationSet;

/// Raio do anel da Boca (m): entrar começa a descida.
pub const MOUTH_RADIUS: f32 = 120.0;
/// Longe disto da Boca, a descida acaba (fugiu).
const ARENA_RADIUS: f32 = 900.0;
/// Tempo por camada (s) e a pausa até a próxima.
const LAYER_SECS: f32 = 90.0;
const BREATH_SECS: f32 = 6.0;
/// Quem acabou de sair espera antes de mergulhar de novo (s).
const COOLDOWN_SECS: f32 = 30.0;
/// Renome por camada vencida, vezes a profundidade.
const RENOWN_PER_DEPTH: u32 = 40;
pub const REASON: &str = "camada do Abismo";

/// Boca do Abismo: além do sítio fundo mais longe do spawn, mais para o
/// sem-lei. Determinística pelo mapa (seed).
pub fn mouth(map: &WorldMap) -> Vec2 {
    let spawn = Vec2::from(map.features().spawn);
    let far = map
        .features()
        .kraken_sites
        .iter()
        .map(|site| Vec2::from(*site))
        .max_by(|a, b| a.distance(spawn).total_cmp(&b.distance(spawn)))
        .unwrap_or(spawn);
    let beyond = far + (far - spawn).normalize_or_zero() * 350.0;
    crate::seafaring::water_near(map, far, beyond)
}

/// Onda da camada: quantos saqueadores e o quanto cada um cresce.
pub fn wave(depth: u32) -> (usize, f32, f32) {
    let count = (2 + depth as usize / 2).min(6);
    let hp = 1.0 + 0.25 * (depth - 1) as f32;
    let damage = 1.0 + 0.12 * (depth - 1) as f32;
    (count, hp, damage)
}

/// Butim da camada vencida: bruto que engorda e sobe de raridade.
fn layer_spoils(dev: &DevItems, depth: u32) -> Vec<(ItemDefinitionId, u32)> {
    let mut spoils = crate::npc::raw_spoils(dev, 10 + 6 * depth);
    spoils.push((dev.coral, 2 + depth));
    if depth >= 3 {
        spoils.push((dev.abyssal_pearl, depth / 2));
        spoils.push((dev.abyssal_amber, depth / 3));
    }
    if depth >= 6 {
        spoils.push((dev.fog_crystal, depth / 6));
    }
    spoils.retain(|(_, quantity)| *quantity > 0);
    spoils
}

/// v43: no Abismo Faminto cada camada paga 50% mais.
fn season_spoils(dev: &DevItems, depth: u32) -> Vec<(ItemDefinitionId, u32)> {
    let hungry =
        crate::progress::theme() == marvyr_domain_economy::logbook::SeasonTheme::HungryAbyss;
    layer_spoils(dev, depth)
        .into_iter()
        .map(|(item, quantity)| (item, if hungry { quantity * 3 / 2 } else { quantity }))
        .collect()
}

#[derive(Debug, Clone)]
struct Dive {
    ship_id: u32,
    depth: u32,
    /// Segundos até a camada acabar (ou, em `breath`, até a próxima).
    clock: f32,
    breath: bool,
    wave: Vec<u32>,
}

#[derive(Resource, Default)]
pub struct Abyss {
    dives: HashMap<CharacterId, Dive>,
    cooldown: HashMap<CharacterId, f32>,
}

impl Abyss {
    /// Linhas do HUD de eventos: a Boca sempre, e cada descida em curso.
    pub fn wire(&self, map: &WorldMap) -> Vec<SeaEventState> {
        let at = mouth(map);
        let mut lines = vec![SeaEventState {
            event_id: 0x5000_0000,
            kind: SeaEventKind::Abyss,
            name: String::from("Boca do Abismo"),
            x: at.x,
            y: at.y,
            radius: MOUTH_RADIUS,
            remaining_secs: 0.0,
            chests: Vec::new(),
        }];
        lines.extend(self.dives.values().map(|dive| SeaEventState {
            event_id: 0x5000_0001 + dive.ship_id,
            kind: SeaEventKind::Abyss,
            name: format!("Abismo: camada {}", dive.depth),
            x: at.x,
            y: at.y,
            radius: ARENA_RADIUS,
            remaining_secs: dive.clock.max(0.0),
            chests: Vec::new(),
        }));
        lines
    }
}

pub fn install(app: &mut App) {
    app.init_resource::<Abyss>().add_systems(
        FixedUpdate,
        run_abyss.in_set(SimulationSet::EconomyConsequences),
    );
}

#[allow(clippy::too_many_arguments)]
fn run_abyss(
    mut commands: Commands,
    time: Res<Time>,
    mut abyss: ResMut<Abyss>,
    mut connection_manager: ResMut<ConnectionManager>,
    (dev, dev_ships, map, config): (
        Res<DevItems>,
        Res<crate::crafting::DevShips>,
        Res<ServerWorldMap>,
        Res<crate::npc::NpcSpawnConfig>,
    ),
    (mut npc_ids, mut wreck_ids, mut live_wrecks, mut renown): (
        ResMut<crate::npc::NpcIdCounter>,
        ResMut<crate::net::WreckIdCounter>,
        ResMut<crate::net::LiveWreckRecords>,
        EventWriter<crate::renown::RenownEarned>,
    ),
    npcs: Query<(Entity, &NpcShip)>,
    ships: Query<&ServerShip>,
) {
    let dt = time.delta_secs();
    let now = time.elapsed_secs();
    let at = mouth(&map.0);
    abyss.cooldown.retain(|_, secs| {
        *secs -= dt;
        *secs > 0.0
    });
    // Mergulho novo: entrou no anel, sem descida e sem espera.
    for ship in &ships {
        let Some(client_id) = ship.client_id else {
            continue;
        };
        let here = Vec2::new(ship.motion.x, ship.motion.y);
        if ship.presence != VesselPresence::AtSea
            || here.distance(at) > MOUTH_RADIUS
            || abyss.dives.contains_key(&ship.character)
            || abyss.cooldown.contains_key(&ship.character)
        {
            continue;
        }
        info!(ship_id = ship.ship_id, "mergulho no Abismo");
        abyss.dives.insert(
            ship.character,
            Dive {
                ship_id: ship.ship_id,
                depth: 1,
                clock: 2.0,
                breath: true,
                wave: Vec::new(),
            },
        );
        crate::seafaring::send_action(
            &mut connection_manager,
            client_id,
            ActionKind::Abyss,
            true,
            "O Abismo te puxa: camada 1!",
        );
    }

    let characters: Vec<CharacterId> = abyss.dives.keys().copied().collect();
    for character in characters {
        let Some(mut dive) = abyss.dives.remove(&character) else {
            continue;
        };
        let diver = ships
            .iter()
            .find(|ship| ship.ship_id == dive.ship_id && ship.character == character);
        let alive: Vec<u32> = dive
            .wave
            .iter()
            .copied()
            .filter(|id| npcs.iter().any(|(_, npc)| npc.ship_id == *id))
            .collect();
        dive.wave = alive;
        dive.clock -= dt;
        let fled = diver.map_or(true, |ship| {
            ship.presence != VesselPresence::AtSea
                || Vec2::new(ship.motion.x, ship.motion.y).distance(at) > ARENA_RADIUS
        });
        let timed_out = !dive.breath && dive.clock <= 0.0 && !dive.wave.is_empty();
        if fled || timed_out {
            for id in &dive.wave {
                if let Some((entity, _)) = npcs.iter().find(|(_, npc)| npc.ship_id == *id) {
                    commands.entity(entity).despawn();
                }
            }
            abyss.cooldown.insert(character, COOLDOWN_SECS);
            let reached = dive.depth.saturating_sub(1);
            info!(
                ?character,
                depth = dive.depth,
                fled,
                "o Abismo cuspiu o capitão"
            );
            if let Some(client_id) = diver.and_then(|ship| ship.client_id) {
                crate::seafaring::send_action(
                    &mut connection_manager,
                    client_id,
                    ActionKind::Abyss,
                    false,
                    format!("O Abismo te cuspiu. Camadas vencidas: {reached}."),
                );
            }
            continue;
        }
        let Some(diver) = diver else {
            continue;
        };
        if dive.breath {
            if dive.clock <= 0.0 {
                // Desce: a onda da camada sobe do fundo em volta da Boca.
                let (count, hp, damage) = wave(dive.depth);
                for i in 0..count {
                    let angle = i as f32 / count as f32 * std::f32::consts::TAU + dive.depth as f32;
                    let want = at + Vec2::from_angle(angle) * 320.0;
                    let spot = crate::seafaring::water_near(&map.0, at, want);
                    let (id, mut npc) = crate::npc::build_npc(
                        &dev_ships,
                        &map.0,
                        &config,
                        &mut npc_ids,
                        NpcRole::Reaver,
                        (spot.x, spot.y),
                    );
                    npc.max_hp = (npc.max_hp as f32 * hp) as u32;
                    npc.hp = npc.max_hp;
                    npc.stats.weapon_damage =
                        (npc.stats.weapon_damage as f32 * damage).round() as u32;
                    npc.ai.state = NpcState::Chase {
                        target: diver.ship_id,
                    };
                    npc.ai.leash_radius = ARENA_RADIUS * 2.0;
                    commands.spawn((npc,));
                    dive.wave.push(id);
                }
                dive.breath = false;
                dive.clock = LAYER_SECS;
            }
        } else if dive.wave.is_empty() {
            // Camada vencida: butim do mergulhador e fôlego até a próxima.
            crate::npc::spawn_spoils_wreck(
                &mut commands,
                &mut wreck_ids,
                &mut live_wrecks,
                season_spoils(&dev, dive.depth),
                Some(character),
                (diver.motion.x, diver.motion.y),
                now,
            );
            renown.send(crate::renown::RenownEarned {
                character,
                amount: RENOWN_PER_DEPTH * dive.depth,
                reason: REASON,
            });
            info!(?character, depth = dive.depth, "camada do Abismo vencida");
            if let Some(client_id) = diver.client_id {
                crate::seafaring::send_action(
                    &mut connection_manager,
                    client_id,
                    ActionKind::Abyss,
                    true,
                    format!("Camada {} vencida! Mais fundo...", dive.depth),
                );
            }
            dive.depth += 1;
            dive.breath = true;
            dive.clock = BREATH_SECS;
        }
        abyss.dives.insert(character, dive);
    }
}

/// Profundidade da camada que o motivo de Renome acabou de pagar.
pub fn depth_of(renown: u32) -> u32 {
    renown / RENOWN_PER_DEPTH
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn waves_grow_and_cap() {
        let (one, hp1, dmg1) = wave(1);
        assert_eq!((one, hp1, dmg1), (2, 1.0, 1.0));
        let (ten, hp10, dmg10) = wave(10);
        assert_eq!(ten, 6, "no máximo seis por camada");
        assert!(hp10 > 3.0 && dmg10 > 2.0);
        assert_eq!(wave(40).0, 6);
    }

    #[test]
    fn deeper_layers_pay_rarer_raw() {
        let dev = DevItems::new();
        let shallow = layer_spoils(&dev, 1);
        assert!(!shallow.iter().any(|(item, _)| *item == dev.abyssal_pearl));
        let deep = layer_spoils(&dev, 6);
        assert!(deep.iter().any(|(item, _)| *item == dev.fog_crystal));
        for (item, _) in deep {
            let def = dev.catalog.get(item).unwrap();
            assert_eq!(def.kind, marvyr_domain_items::ItemKind::Resource);
        }
        assert_eq!(depth_of(RENOWN_PER_DEPTH * 7), 7);
    }

    #[test]
    fn mouth_is_lawless_open_water_in_every_world() {
        for seed in [0, 1, 7, 42, 1234] {
            let map = WorldMap::from_seed(seed);
            let at = mouth(&map);
            let zone = map.zone_at(at.x, at.y).expect("dentro de uma zona");
            assert_eq!(
                zone.tier,
                marvyr_domain_world::RiskTier::Lawless,
                "seed {seed}"
            );
            assert!(
                map.push_out_of_land(at.x, at.y, 40.0).is_none(),
                "seed {seed}"
            );
        }
    }
}
