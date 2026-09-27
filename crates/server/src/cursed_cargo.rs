//! v38: Carga Amaldiçoada. Às vezes, no mar sem lei, o que boia é um baú
//! amaldiçoado. Quem o põe no porão aparece no mapa de todo mundo, e os
//! piratas de elite vêm atrás — cada vez mais rápido. Entregue num porto,
//! a maldição se desfaz em recurso bruto (Pilar 1) e Renome. Afundou, a
//! carga vai para o destroço como qualquer outra: quem pegar, carrega.

use std::collections::HashMap;

use bevy::prelude::*;
use lightyear::prelude::server::*;
use marvyr_domain_items::ItemDefinition;
use marvyr_domain_ships::VesselPresence;
use marvyr_domain_world::RiskTier;
use marvyr_protocol::{ActionKind, SeaEventKind, SeaEventState, WorldEventKind};
use marvyr_shared::ids::ItemDefinitionId;
use tracing::info;

use crate::market::ServerMarket;
use crate::net::{DevItems, ServerShip, ServerWorldMap};
use crate::npc::{NpcRole, NpcShip, NpcState};
use crate::sets::SimulationSet;

pub const CURSED_CARGO: &str = "Carga Amaldiçoada";
/// Dentre os achados à deriva no mar sem lei, 1 em N é a carga.
pub const FIND_ONE_IN: u128 = 10;
/// Primeira caçada depois de embarcar e o intervalo, que encurta a cada
/// caçada até o piso (s).
const FIRST_HUNT: f32 = 30.0;
const HUNT_EVERY: f32 = 45.0;
const HUNT_FLOOR: f32 = 15.0;
const HUNT_SHRINK: f32 = 5.0;
/// Caçador que não afundou ninguém volta para o fundo depois disto (s).
const HUNTER_SECS: f32 = 150.0;
/// Renome da entrega (e o Diário conta pelo motivo).
const DELIVERY_RENOWN: u32 = 150;
pub const DELIVERY_REASON: &str = "carga amaldiçoada entregue";

pub fn item_id() -> ItemDefinitionId {
    ItemDefinitionId::stable(CURSED_CARGO)
}

pub fn definition() -> ItemDefinition {
    ItemDefinition {
        id: item_id(),
        kind: marvyr_domain_items::ItemKind::Quest,
        equipment: None,
        max_stack: 1,
        base_weight: 10,
        tags: Default::default(),
        display_name: String::from(CURSED_CARGO),
    }
}

/// O que a maldição vira no armazém, por carga entregue.
fn reward(dev: &DevItems) -> [(ItemDefinitionId, u32); 3] {
    [
        (dev.abyssal_pearl, 4),
        (dev.abyssal_amber, 4),
        (dev.coral, 12),
    ]
}

struct Carrier {
    next_hunt: f32,
    hunts: u32,
}

#[derive(Resource, Default)]
pub struct CursedCargo {
    carriers: HashMap<u32, Carrier>,
    /// Caçadores soltos: (npc, hora de sumir).
    hunters: Vec<(u32, f32)>,
}

pub fn install(app: &mut App) {
    app.init_resource::<CursedCargo>().add_systems(
        FixedUpdate,
        run_cursed_cargo.in_set(SimulationSet::EconomyConsequences),
    );
}

/// Quantas cargas o navio leva.
pub fn aboard(ship: &ServerShip) -> u32 {
    ship.hold
        .items()
        .iter()
        .filter(|custody| custody.instance.definition == item_id())
        .map(|custody| custody.instance.quantity)
        .sum()
}

fn next_interval(hunts: u32) -> f32 {
    (HUNT_EVERY - HUNT_SHRINK * hunts as f32).max(HUNT_FLOOR)
}

/// Linhas do HUD de eventos: cada navio com a carga, visto por todos.
pub fn wire(ships: &Query<&ServerShip>) -> Vec<SeaEventState> {
    ships
        .iter()
        .filter(|ship| aboard(ship) > 0 && ship.presence == VesselPresence::AtSea)
        .map(|ship| SeaEventState {
            event_id: 0x4000_0000 + ship.ship_id,
            kind: SeaEventKind::CursedCargo,
            name: String::from(CURSED_CARGO),
            x: ship.motion.x,
            y: ship.motion.y,
            radius: 90.0,
            remaining_secs: 0.0,
            chests: Vec::new(),
        })
        .collect()
}

#[allow(clippy::too_many_arguments)]
fn run_cursed_cargo(
    mut commands: Commands,
    time: Res<Time>,
    mut cargo: ResMut<CursedCargo>,
    mut connection_manager: ResMut<ConnectionManager>,
    (dev, dev_ships, map, config): (
        Res<DevItems>,
        Res<crate::crafting::DevShips>,
        Res<ServerWorldMap>,
        Res<crate::npc::NpcSpawnConfig>,
    ),
    store: Res<crate::persist::StoreHandle>,
    (mut npc_ids, mut market, mut renown): (
        ResMut<crate::npc::NpcIdCounter>,
        ResMut<ServerMarket>,
        EventWriter<crate::renown::RenownEarned>,
    ),
    npcs: Query<(Entity, &NpcShip)>,
    mut ships: Query<&mut ServerShip>,
) {
    let now = time.elapsed_secs();
    // Caçador que passou da hora some.
    cargo.hunters.retain(|(npc_id, gone_at)| {
        let alive = npcs.iter().find(|(_, npc)| npc.ship_id == *npc_id);
        match alive {
            Some((entity, _)) if now >= *gone_at => {
                commands.entity(entity).despawn();
                false
            }
            Some(_) => true,
            None => false,
        }
    });
    for mut ship in &mut ships {
        let count = aboard(&ship);
        if count == 0 {
            cargo.carriers.remove(&ship.ship_id);
            continue;
        }
        let client = ship.client_id;
        // Porto: a maldição se desfaz.
        if let VesselPresence::Docked(region) = ship.presence {
            if ship.hold.remove(item_id(), count).is_err() {
                continue;
            }
            // Grava o porão sem a carga antes de pagar: crash entre os dois
            // perde a recompensa, nunca a paga em dobro.
            crate::net::save_ship_now(&store, &ship);
            for (item, quantity) in reward(&dev) {
                market.grant_to_storage(
                    ship.character,
                    region,
                    item,
                    quantity * count,
                    &dev.catalog,
                );
            }
            renown.send(crate::renown::RenownEarned {
                character: ship.character,
                amount: DELIVERY_RENOWN * count,
                reason: DELIVERY_REASON,
            });
            cargo.carriers.remove(&ship.ship_id);
            info!(ship_id = ship.ship_id, count, "carga amaldiçoada entregue");
            if let Some(client_id) = client {
                crate::seafaring::send_action(
                    &mut connection_manager,
                    client_id,
                    ActionKind::CursedCargo,
                    true,
                    "Carga entregue! A maldição virou pérolas, âmbar e coral no armazém.",
                );
            }
            continue;
        }
        let carrier = cargo.carriers.entry(ship.ship_id).or_insert_with(|| {
            if let Some(client_id) = client {
                crate::reputation::send_event(
                    &mut connection_manager,
                    &[client_id],
                    String::from("A carga amaldicoada chama os piratas: leve-a a um porto!"),
                    WorldEventKind::Alert,
                );
            }
            Carrier {
                next_hunt: now + FIRST_HUNT,
                hunts: 0,
            }
        });
        if now < carrier.next_hunt {
            continue;
        }
        carrier.hunts += 1;
        carrier.next_hunt = now + next_interval(carrier.hunts);
        // Águas protegidas seguram os caçadores (a carga ainda aparece).
        let protected = map
            .0
            .zone_at(ship.motion.x, ship.motion.y)
            .map_or(true, |zone| zone.tier == RiskTier::Protected);
        if protected {
            continue;
        }
        let at = Vec2::new(ship.motion.x, ship.motion.y);
        let behind = at - Vec2::from_angle(ship.motion.heading) * 380.0;
        let spot = crate::seafaring::water_near(&map.0, at, behind);
        let (id, mut npc) = crate::npc::build_npc(
            &dev_ships,
            &map.0,
            &config,
            &mut npc_ids,
            NpcRole::Reaver,
            (spot.x, spot.y),
        );
        npc.ai.state = NpcState::Chase {
            target: ship.ship_id,
        };
        npc.ai.leash_radius = 3_000.0;
        commands.spawn((npc,));
        cargo.hunters.push((id, now + HUNTER_SECS));
        info!(
            ship_id = ship.ship_id,
            npc_id = id,
            "caçador da carga amaldiçoada"
        );
        if let Some(client_id) = client {
            crate::reputation::send_event(
                &mut connection_manager,
                &[client_id],
                String::from("Um saqueador farejou a carga amaldicoada!"),
                WorldEventKind::Alert,
            );
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn hunts_come_faster_down_to_a_floor() {
        assert_eq!(next_interval(0), HUNT_EVERY);
        assert!(next_interval(2) < next_interval(1));
        assert_eq!(next_interval(50), HUNT_FLOOR);
    }

    #[test]
    fn cargo_is_heavy_single_and_pays_raw_only() {
        let dev = DevItems::new();
        assert_eq!(definition().max_stack, 1);
        for (item, _) in reward(&dev) {
            let def = dev.catalog.get(item).expect("no catálogo");
            assert_eq!(def.kind, marvyr_domain_items::ItemKind::Resource);
        }
    }
}
