//! Portais no servidor (MF-059): amarra o `PortalDirector` puro ao tick.
//! Travessia é autoritativa — o client só vê o navio reaparecer do outro
//! lado. Arena de cerração que fecha devolve quem ficou dentro à origem.

use bevy::prelude::*;
use lightyear::prelude::server::*;
use lightyear::prelude::*;
use marvyr_domain_ships::VesselPresence;
use marvyr_domain_world::map::{FOG_RADIUS, FOG_SLOTS};
use marvyr_domain_world::{arena_layout, PortalDirector, PortalKind, PortalTuning};
use marvyr_protocol::{PortalKindWire, PortalState, PortalsUpdate};
use tracing::info;

use crate::net::{dev_spawn_point, ReliableChannel, ServerShip, ServerWorldMap};
use crate::sets::SimulationSet;

#[derive(Resource)]
pub struct ServerPortals(pub PortalDirector);

pub struct PortalPlugin;

impl Plugin for PortalPlugin {
    fn build(&self, app: &mut App) {
        // Semente pelo relógio: cada servidor sorteia um mar diferente.
        // Teste pode inserir um `ServerPortals` próprio antes (determinístico).
        if !app.world().contains_resource::<ServerPortals>() {
            let seed = std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .map(|d| d.as_nanos() as u64)
                .unwrap_or(1);
            app.insert_resource(ServerPortals(PortalDirector::new(
                seed,
                PortalTuning::default(),
            )));
        }
        app.add_systems(
            FixedUpdate,
            (advance_portals, sync_arenas, broadcast_portals)
                .chain()
                // Depois do movimento; se rodar após a resolução de zona, o
                // `ZoneChanged` do destino sai no tick seguinte (33 ms).
                .in_set(SimulationSet::Zones),
        );
    }
}

/// Relógio dos portais + travessias.
fn advance_portals(
    time: Res<Time>,
    map: Res<ServerWorldMap>,
    mut portals: ResMut<ServerPortals>,
    mut ships: Query<&mut ServerShip>,
    mut npcs: Query<&mut crate::npc::NpcShip>,
    mut discoveries: EventWriter<crate::progress::Discovered>,
) {
    let now = time.elapsed_secs_f64();
    let closed = portals.0.tick(now, &map.0);
    for arena in closed {
        for mut ship in &mut ships {
            let (dx, dy) = (
                ship.motion.x - arena.center.0,
                ship.motion.y - arena.center.1,
            );
            if dx * dx + dy * dy <= (FOG_RADIUS + 80.0).powi(2) {
                info!(ship_id = ship.ship_id, "cerração dissipou: navio devolvido");
                ship.motion.x = arena.origin.0;
                ship.motion.y = arena.origin.1;
                portals.0.hold(ship.ship_id, now);
            }
        }
    }
    for mut ship in &mut ships {
        // Só capitães atravessam; NPC e casco atracado ficam onde estão.
        if ship.client_id.is_none() || matches!(ship.presence, VesselPresence::Docked(_)) {
            continue;
        }
        let (x, y) = (ship.motion.x, ship.motion.y);
        if portals.0.stranded(x, y) {
            info!(
                ship_id = ship.ship_id,
                "navio preso em cerração fechada: resgatado"
            );
            (ship.motion.x, ship.motion.y) = dev_spawn_point(&map.0);
            continue;
        }
        // MV-066: portão de zona. Estático e sem carência — a chegada fica
        // fora do portão de volta.
        if let Some(exit) = map.0.exit_at(x, y) {
            let to = map.0.features().areas[exit.to].name;
            info!(ship_id = ship.ship_id, to, "navio cruzou para outra zona");
            (ship.motion.x, ship.motion.y) = exit.dest;
            continue;
        }
        if let Some((dest_x, dest_y)) = portals.0.transit(ship.ship_id, x, y, now) {
            let zone = map.0.zone_at(dest_x, dest_y).map(|z| z.name).unwrap_or("?");
            info!(ship_id = ship.ship_id, zone, "navio atravessou um portal");
            // v41: Livro de Bordo — cerração leva a uma arena; o resto é
            // sorvedouro.
            let into_fog = FOG_SLOTS
                .iter()
                .any(|c| (dest_x - c.0).powi(2) + (dest_y - c.1).powi(2) <= FOG_RADIUS.powi(2));
            discoveries.send(crate::progress::Discovered {
                character: ship.character,
                entry: if into_fog { "Cerração" } else { "Sorvedouro" },
            });
            ship.motion.x = dest_x;
            ship.motion.y = dest_y;
        }
    }
    for mut npc in &mut npcs {
        cross_npc(&map.0, &mut npc);
    }
}

/// Miolo das cerrações (MV-066): quando uma abre, os rochedos da sua semente
/// viram terra no mapa autoritativo e os baús boiam; quando fecha, a terra
/// some. Baú não precisa sumir junto: o wreck afunda (300 s) antes de a
/// arena fechar (600 s).
fn sync_arenas(
    mut commands: Commands,
    time: Res<Time>,
    portals: Res<ServerPortals>,
    mut map: ResMut<ServerWorldMap>,
    dev: Res<crate::net::DevItems>,
    (mut wreck_ids, mut live_wrecks): (
        ResMut<crate::net::WreckIdCounter>,
        ResMut<crate::net::LiveWreckRecords>,
    ),
    mut known: Local<[Option<u64>; FOG_SLOTS.len()]>,
) {
    for (slot, arena) in portals.0.arenas().iter().enumerate() {
        let layout = arena.map(|open| open.layout);
        if known[slot] == layout {
            continue;
        }
        known[slot] = layout;
        map.0.set_arena(slot, layout);
        let Some(layout) = layout else {
            continue;
        };
        info!(slot, "cerração aberta: miolo sorteado");
        for chest in arena_layout(slot, layout).chests {
            crate::npc::spawn_spoils_wreck(
                &mut commands,
                &mut wreck_ids,
                &mut live_wrecks,
                arena_chest(&dev),
                None,
                chest,
                time.elapsed_secs(),
            );
        }
    }
}

/// Baú de cerração: recurso bruto (o tier 3 só nasce aqui), nunca item
/// pronto — ainda precisa voltar ao porto e passar pela Forja.
fn arena_chest(dev: &crate::net::DevItems) -> Vec<(marvyr_shared::ids::ItemDefinitionId, u32)> {
    vec![(dev.fog_crystal, 3), (dev.fog_essence, 2)]
}

/// NPC num portão de zona: quem tem o outro lado na rota (caravana)
/// atravessa e segue a rota; o resto (patrulha, perseguição) bate no
/// paredão e volta para dentro da própria zona.
pub(crate) fn cross_npc(map: &marvyr_domain_world::WorldMap, npc: &mut crate::npc::NpcShip) {
    let Some(exit) = map.exit_at(npc.motion.x, npc.motion.y).copied() else {
        return;
    };
    if let Some(index) = npc.ai.route.iter().position(|point| *point == exit.dest) {
        (npc.motion.x, npc.motion.y) = exit.dest;
        npc.ai.next_waypoint = index + 1;
        return;
    }
    let area = &map.features().areas[exit.from];
    let (dx, dy) = (exit.x - area.x, exit.y - area.y);
    let len = dx.hypot(dy).max(f32::EPSILON);
    let inside = area.radius - 150.0;
    (npc.motion.x, npc.motion.y) = (area.x + dx / len * inside, area.y + dy / len * inside);
}

fn wire_kind(kind: PortalKind) -> PortalKindWire {
    match kind {
        PortalKind::FogGate => PortalKindWire::FogGate,
        PortalKind::FogExit => PortalKindWire::FogExit,
        PortalKind::Whirlpool => PortalKindWire::Whirlpool,
    }
}

pub fn portal_states(director: &PortalDirector, now: f64) -> Vec<PortalState> {
    director
        .portals()
        .iter()
        .map(|p| PortalState {
            portal_id: p.id,
            kind: wire_kind(p.kind),
            x: p.x,
            y: p.y,
            radius: p.radius,
            expires_in_secs: (p.expires_at - now).max(0.0) as f32,
            uses_left: p.uses_left,
        })
        .collect()
}

/// Todos os portais para todos, a cada segundo (são poucos e o mapa inteiro
/// precisa saber onde a cerração surgiu — é o que faz gente correr para ela).
fn broadcast_portals(
    time: Res<Time>,
    portals: Res<ServerPortals>,
    mut connection_manager: ResMut<ConnectionManager>,
    mut timer: Local<f32>,
) {
    *timer += time.delta_secs();
    if *timer < 1.0 {
        return;
    }
    *timer = 0.0;
    let update = PortalsUpdate {
        portals: portal_states(&portals.0, time.elapsed_secs_f64()),
        arenas: portals
            .0
            .arenas()
            .iter()
            .enumerate()
            .filter_map(|(slot, arena)| arena.map(|open| (slot as u8, open.layout)))
            .collect(),
    };
    let _ = connection_manager
        .send_message_to_target::<ReliableChannel, _>(&update, NetworkTarget::All);
}

#[cfg(test)]
mod tests {
    use super::*;
    use marvyr_domain_world::WorldMap;

    #[test]
    fn opening_a_fog_arena_raises_its_rocks_and_floats_crystal_chests() {
        use bevy::ecs::system::RunSystemOnce;
        let map = WorldMap::vertical_slice();
        let mut director = PortalDirector::new(3, PortalTuning::default());
        director.tick(0.0, &map);
        let layout = director.arenas()[0].expect("arena aberta").layout;
        let mut world = World::new();
        world.insert_resource(Time::<()>::default());
        world.insert_resource(ServerPortals(director));
        world.insert_resource(ServerWorldMap(map));
        world.insert_resource(crate::net::DevItems::new());
        world.init_resource::<crate::net::WreckIdCounter>();
        world.init_resource::<crate::net::LiveWreckRecords>();
        world.run_system_once(sync_arenas).unwrap();
        let rocks = arena_layout(0, layout).rocks;
        let map = &world.resource::<ServerWorldMap>().0;
        assert!(rocks.iter().all(|rock| map.is_land(rock.x, rock.y)));
        let crystal = world.resource::<crate::net::DevItems>().fog_crystal;
        let mut wrecks = world.query::<&crate::net::ServerWreck>();
        let chests: Vec<_> = wrecks.iter(&world).collect();
        assert_eq!(chests.len(), marvyr_domain_world::map::ARENA_CHESTS);
        for chest in chests {
            assert!(chest.exclusive_looter.is_none());
            assert!(chest
                .chest
                .items()
                .iter()
                .any(|custody| custody.instance.definition == crystal));
        }
        assert_eq!(world.resource::<crate::net::LiveWreckRecords>().0.len(), 2);
    }

    #[test]
    fn wire_states_report_remaining_time_and_uses() {
        let map = WorldMap::vertical_slice();
        let mut director = PortalDirector::new(3, PortalTuning::default());
        director.tick(0.0, &map);
        let states = portal_states(&director, 10.0);
        assert_eq!(states.len(), director.portals().len());
        let gate = states
            .iter()
            .find(|s| s.kind == PortalKindWire::FogGate)
            .expect("cerração aberta");
        let lifetime = PortalTuning::default().fog_gate_lifetime as f32;
        assert!((gate.expires_in_secs - (lifetime - 10.0)).abs() < 1e-3);
        assert_eq!(gate.uses_left, Some(2));
    }
}
