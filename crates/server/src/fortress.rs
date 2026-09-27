//! v60: fortalezas piratas — uma por zona sem lei, pedra parada no mar.
//! Canhão em toda volta, tiro pesado anunciado e, quando o casco cai a 70%
//! e a 40%, uma onda de chalupas sai do cais. Quem tirou pelo menos 3% do
//! casco leva o próprio baú (bruto e moeda de ofício — Pilar 1) e Renome;
//! a fortaleza se reconstrói depois de `REBUILD_SECS`.

use std::collections::HashMap;

use bevy::prelude::*;
use lightyear::prelude::server::*;
use lightyear::prelude::*;
use marvyr_domain_world::{RiskTier, WorldMap};
use marvyr_protocol::WorldEventKind;
use marvyr_shared::ids::{CharacterId, ItemDefinitionId};
use tracing::info;

use crate::net::{DevItems, ReliableChannel, ServerWorldMap};
use crate::npc::{NpcRole, NpcShip};
use crate::sets::SimulationSet;

pub const FORT_HP: u32 = 2_500;
/// Tiro pesado da fortaleza (mais amiúde que o da elite).
pub const HEAVY_EVERY_SECS: f32 = 5.0;
/// Pedra derrubada volta depois disto.
const REBUILD_SECS: f32 = 900.0;
/// Fração do casco em que sai cada onda.
const WAVES_AT: [f32; 2] = [0.7, 0.4];
const WAVE_SIZE: usize = 3;
const WAVE_RADIUS: f32 = 110.0;
/// Dano mínimo para ter parte no butim (3% do casco).
pub const SHARE_MIN_DAMAGE: u32 = FORT_HP * 3 / 100;
const RENOWN_SHARE: u32 = 150;
/// v63: influência de cada participante no porto disputado mais perto (a
/// fortaleza derrubada conta na guerra de território).
pub const FORT_INFLUENCE: u32 = 80;

#[derive(Debug, Default)]
struct Fort {
    site: (f32, f32),
    /// NPC vivo; `None` = em reconstrução.
    npc_id: Option<u32>,
    rebuild_in: f32,
    waves_sent: usize,
    damage: HashMap<CharacterId, u32>,
    slain: bool,
}

#[derive(Resource, Default, Debug)]
pub struct Fortresses {
    forts: Vec<Fort>,
    placed: bool,
}

impl Fortresses {
    fn of(&mut self, npc_id: u32) -> Option<&mut Fort> {
        self.forts
            .iter_mut()
            .find(|fort| fort.npc_id == Some(npc_id))
    }

    /// Golpe de um capitão na fortaleza (chamado pelo `simulate_npcs`).
    pub fn record_hit(&mut self, npc_id: u32, character: CharacterId, damage: u32) {
        if let Some(fort) = self.of(npc_id) {
            *fort.damage.entry(character).or_default() += damage;
        }
    }

    pub fn slain(&mut self, npc_id: u32) {
        if let Some(fort) = self.of(npc_id) {
            fort.slain = true;
        }
    }
}

impl Fort {
    fn sharers(&self) -> Vec<CharacterId> {
        let mut out: Vec<CharacterId> = self
            .damage
            .iter()
            .filter(|(_, damage)| **damage >= SHARE_MIN_DAMAGE)
            .map(|(character, _)| *character)
            .collect();
        out.sort_by_key(|c| c.0);
        out
    }

    /// Quantas ondas o casco atual já deveria ter soltado.
    fn waves_due(hp_ratio: f32) -> usize {
        WAVES_AT.iter().filter(|at| hp_ratio <= **at).count()
    }
}

/// Pedra longe de porto: quem atraca no sem lei não pode viver debaixo do
/// canhão (detecção + alcance, com folga).
const PORT_CLEARANCE: f32 = 700.0;

/// Uma fortaleza por zona sem lei, fora do caminho dos bandos e longe dos
/// portos. O mapa clássico (seed 0) não tem zonas: sem fortaleza.
pub fn fort_sites(map: &WorldMap) -> Vec<(f32, f32)> {
    let features = map.features();
    if features.seed == 0 {
        return Vec::new();
    }
    let ports: Vec<(f32, f32)> = map
        .regions()
        .iter()
        .filter_map(|region| region.port.as_ref().map(|port| (port.x, port.y)))
        .collect();
    let clear = |(x, y): (f32, f32)| {
        !map.is_land(x, y)
            && ports
                .iter()
                .all(|(px, py)| (px - x).hypot(py - y) >= PORT_CLEARANCE)
    };
    features
        .areas
        .iter()
        .enumerate()
        .filter(|(_, area)| area.tier == RiskTier::Lawless)
        .filter_map(|(index, area)| {
            // Os bandos ficam em index*1.3 + k*120° a 55% do raio: a
            // fortaleza tenta entre dois deles e gira até achar água limpa.
            let base = index as f32 * 1.3 + std::f32::consts::FRAC_PI_3;
            (0..8)
                .flat_map(|step| [0.3, 0.45].map(|reach| (step, reach)))
                .map(|(step, reach)| {
                    let angle = base + step as f32 * std::f32::consts::FRAC_PI_4;
                    let (x, y) = (
                        area.x + angle.cos() * area.radius * reach,
                        area.y + angle.sin() * area.radius * reach,
                    );
                    map.push_out_of_land(x, y, 80.0).unwrap_or((x, y))
                })
                .find(|spot| clear(*spot))
        })
        .collect()
}

/// Porto disputado mais perto da fortaleza (onde a derrubada vira
/// influência).
fn nearest_contested_port(map: &WorldMap, at: (f32, f32)) -> Option<&'static str> {
    crate::territory::contested_ports(map)
        .into_iter()
        .min_by(|a, b| {
            a.1.distance(Vec2::from(at))
                .total_cmp(&b.1.distance(Vec2::from(at)))
        })
        .map(|(port, _)| port)
}

/// Baú de cada participante: bruto e moeda de ofício (Pilar 1).
fn share(dev: &DevItems) -> Vec<(ItemDefinitionId, u32)> {
    vec![
        (dev.ore, 15),
        (dev.timber, 15),
        (dev.coral, 8),
        (dev.abyssal_pearl, 2),
        (dev.gem_shard, 3),
        (marvyr_domain_items::OrbKind::Chaos.item_id(), 1),
    ]
}

pub fn install(app: &mut App) {
    app.init_resource::<Fortresses>();
    app.add_systems(
        FixedUpdate,
        run_fortresses.in_set(SimulationSet::EconomyConsequences),
    );
}

fn announce(connection_manager: &mut ConnectionManager, text: String) {
    let _ = connection_manager.send_message_to_target::<ReliableChannel, _>(
        &marvyr_protocol::WorldEvent {
            text,
            kind: WorldEventKind::Alert,
        },
        NetworkTarget::All,
    );
}

#[allow(clippy::too_many_arguments, clippy::type_complexity)]
fn run_fortresses(
    mut commands: Commands,
    time: Res<Time>,
    mut forts: ResMut<Fortresses>,
    mut connection_manager: ResMut<ConnectionManager>,
    (dev, dev_ships, map, config): (
        Res<DevItems>,
        Res<crate::crafting::DevShips>,
        Res<ServerWorldMap>,
        Res<crate::npc::NpcSpawnConfig>,
    ),
    (mut npc_ids, mut wreck_ids, mut live_wrecks, mut renown, mut logbook): (
        ResMut<crate::npc::NpcIdCounter>,
        ResMut<crate::net::WreckIdCounter>,
        ResMut<crate::net::LiveWreckRecords>,
        EventWriter<crate::renown::RenownEarned>,
        ResMut<crate::progress::CaptainLogbook>,
    ),
    npcs: Query<&NpcShip>,
) {
    if !forts.placed {
        forts.placed = true;
        forts.forts = fort_sites(&map.0)
            .into_iter()
            .map(|site| Fort {
                site,
                ..Fort::default()
            })
            .collect();
        info!(
            count = forts.forts.len(),
            "fortalezas piratas no mar sem lei"
        );
    }
    let dt = time.delta_secs();
    let now = time.elapsed_secs();
    for fort in &mut forts.forts {
        // Derrubada: cada participante ganha o próprio baú.
        if fort.slain {
            let sharers = fort.sharers();
            let port = nearest_contested_port(&map.0, fort.site);
            for (i, character) in sharers.iter().enumerate() {
                let angle = i as f32 * 2.399_963;
                let spot = Vec2::new(fort.site.0, fort.site.1)
                    + Vec2::from_angle(angle) * (30.0 + 8.0 * i as f32);
                crate::npc::spawn_spoils_wreck(
                    &mut commands,
                    &mut wreck_ids,
                    &mut live_wrecks,
                    share(&dev),
                    Some(*character),
                    (spot.x, spot.y),
                    now,
                );
                renown.send(crate::renown::RenownEarned {
                    character: *character,
                    amount: RENOWN_SHARE,
                    reason: "fortaleza derrubada",
                });
                if let Some(port) = port {
                    logbook.add_war_influence(*character, port, FORT_INFLUENCE);
                }
            }
            info!(
                sharers = sharers.len(),
                "fortaleza derrubada; butim dividido"
            );
            announce(
                &mut connection_manager,
                format!(
                    "Uma fortaleza pirata caiu! {} capitães dividem o butim.",
                    sharers.len()
                ),
            );
            *fort = Fort {
                site: fort.site,
                rebuild_in: REBUILD_SECS,
                ..Fort::default()
            };
            continue;
        }
        match fort.npc_id {
            None => {
                fort.rebuild_in -= dt;
                if fort.rebuild_in > 0.0 {
                    continue;
                }
                let (npc_id, npc) = crate::npc::build_npc(
                    &dev_ships,
                    &map.0,
                    &config,
                    &mut npc_ids,
                    NpcRole::Fort,
                    fort.site,
                );
                commands.spawn((npc,));
                fort.npc_id = Some(npc_id);
                info!(
                    npc_id,
                    x = fort.site.0,
                    y = fort.site.1,
                    "fortaleza erguida"
                );
            }
            Some(npc_id) => {
                let Some(npc) = npcs.iter().find(|npc| npc.ship_id == npc_id) else {
                    // Sumiu sem `slain` (despawn por fora): reconstrói sem butim.
                    *fort = Fort {
                        site: fort.site,
                        rebuild_in: REBUILD_SECS,
                        ..Fort::default()
                    };
                    continue;
                };
                let due = Fort::waves_due(npc.hp as f32 / npc.max_hp.max(1) as f32);
                while fort.waves_sent < due {
                    fort.waves_sent += 1;
                    for k in 0..WAVE_SIZE {
                        let angle = k as f32 * std::f32::consts::TAU / WAVE_SIZE as f32
                            + fort.waves_sent as f32;
                        let at = (
                            fort.site.0 + angle.cos() * WAVE_RADIUS,
                            fort.site.1 + angle.sin() * WAVE_RADIUS,
                        );
                        let at = map.0.push_out_of_land(at.0, at.1, 40.0).unwrap_or(at);
                        let (_, mut sloop) = crate::npc::build_npc(
                            &dev_ships,
                            &map.0,
                            &config,
                            &mut npc_ids,
                            NpcRole::Sloop,
                            at,
                        );
                        // Onda não volta: afundou, acabou.
                        sloop.ai.respawn_after_secs = f32::INFINITY;
                        commands.spawn((sloop,));
                    }
                    info!(npc_id, wave = fort.waves_sent, "fortaleza soltou uma onda");
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn waves_fire_once_per_threshold() {
        assert_eq!(Fort::waves_due(1.0), 0);
        assert_eq!(Fort::waves_due(0.7), 1);
        assert_eq!(Fort::waves_due(0.5), 1);
        assert_eq!(Fort::waves_due(0.1), 2);
    }

    #[test]
    fn only_real_attackers_share_and_hits_follow_the_fort() {
        let mut forts = Fortresses::default();
        forts.forts.push(Fort {
            npc_id: Some(7),
            ..Fort::default()
        });
        let (a, b) = (CharacterId::new(), CharacterId::new());
        forts.record_hit(7, a, SHARE_MIN_DAMAGE);
        forts.record_hit(7, b, 1);
        forts.record_hit(99, b, SHARE_MIN_DAMAGE);
        assert_eq!(forts.forts[0].sharers(), vec![a]);
        forts.slain(7);
        assert!(forts.forts[0].slain);
    }

    #[test]
    fn generated_worlds_raise_forts_on_water_away_from_ports() {
        let map = WorldMap::from_seed(crate::net::DEFAULT_WORLD_SEED);
        let lawless = map
            .features()
            .areas
            .iter()
            .filter(|area| area.tier == RiskTier::Lawless)
            .count();
        let sites = fort_sites(&map);
        assert!(!sites.is_empty() && sites.len() <= lawless);
        for (x, y) in sites {
            assert!(!map.is_land(x, y), "fortaleza na água");
            for port in map
                .regions()
                .iter()
                .filter_map(|region| region.port.as_ref())
            {
                assert!(
                    (port.x - x).hypot(port.y - y) >= PORT_CLEARANCE,
                    "fortaleza longe de {}",
                    port.name
                );
            }
        }
        assert!(fort_sites(&WorldMap::from_seed(0)).is_empty());
        let site = fort_sites(&map)[0];
        assert!(
            nearest_contested_port(&map, site).is_some(),
            "fortaleza conta para um porto"
        );
    }
}
