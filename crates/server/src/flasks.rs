//! Frascos de bordo no servidor (v25). Beber é intent (teclas 1-4 no mar);
//! o efeito, as cargas e o remendo da Estopa correm aqui. Multiplicadores
//! entram onde o jogo calcula: movimento, arma, recarga e dano recebido.

use bevy::prelude::*;
use lightyear::prelude::*;
use marvyr_domain_combat::FlaskKind;
use marvyr_domain_ships::VesselPresence;
use marvyr_protocol::{FlaskWire, UseFlask};
use tracing::info;

use crate::net::ServerShip;
use crate::sets::SimulationSet;

/// Acertos de canhão deste tick: (navio atirador, afundou?). Enche os
/// frascos do atirador — frasco se recarrega lutando, como no PoE.
#[derive(Resource, Default)]
pub struct FlaskHits(pub Vec<(u32, bool)>);

pub struct FlaskPlugin;

impl Plugin for FlaskPlugin {
    fn build(&self, app: &mut App) {
        app.init_resource::<FlaskHits>().add_systems(
            FixedUpdate,
            (
                handle_use_flask.in_set(SimulationSet::Input),
                tick_flasks.in_set(SimulationSet::EconomyConsequences),
            ),
        );
    }
}

/// Frasco no porão: é o item que se bebe.
pub fn aboard(ship: &ServerShip, kind: FlaskKind) -> bool {
    ship.hold
        .items()
        .iter()
        .any(|custody| custody.instance.definition == kind.item_id())
}

fn handle_use_flask(
    mut events: EventReader<ServerReceiveMessage<UseFlask>>,
    mut ships: Query<&mut ServerShip>,
) {
    for event in events.read() {
        let client_id = event.from();
        let kind = event.message().kind;
        let Some(mut ship) = ships.iter_mut().find(|s| s.client_id == Some(client_id)) else {
            continue;
        };
        // Porto enche os frascos; beber atracado só desperdiçaria a dose.
        if matches!(ship.presence, VesselPresence::Docked(_)) || !aboard(&ship, kind) {
            continue;
        }
        match ship.flasks.drink(kind) {
            Ok(()) => info!(ship_id = ship.ship_id, ?kind, "frasco bebido"),
            Err(refusal) => info!(ship_id = ship.ship_id, ?kind, ?refusal, "frasco recusado"),
        }
    }
}

fn tick_flasks(time: Res<Time>, mut hits: ResMut<FlaskHits>, mut ships: Query<&mut ServerShip>) {
    let dt = time.delta_secs();
    let hits = std::mem::take(&mut hits.0);
    for mut ship in &mut ships {
        let max_hp = ship.stats.max_hp;
        let healed = ship.flasks.tick(dt, max_hp);
        // Casco a pique (hp 0) não se remenda: já é naufrágio.
        if healed > 0 && ship.hp > 0 {
            ship.hp = (ship.hp + healed).min(max_hp);
        }
        if matches!(ship.presence, VesselPresence::Docked(_)) {
            ship.flasks.refill();
        }
        for (shooter, sank) in &hits {
            if *shooter == ship.ship_id {
                ship.flasks.fill(if *sank {
                    marvyr_domain_combat::flask::CHARGES_PER_SINK
                } else {
                    marvyr_domain_combat::flask::CHARGES_PER_HIT
                });
            }
        }
    }
}

/// Face protocolar dos frascos de um navio.
pub fn wire(ship: &ServerShip) -> FlaskWire {
    FlaskWire {
        charges: ship.flasks.charges(),
        active: ship.flasks.active_mask(),
        aboard: FlaskKind::ALL
            .iter()
            .filter(|kind| aboard(ship, **kind))
            .fold(0, |mask, kind| mask | 1 << kind.index()),
    }
}

/// Frasco solto no catálogo: consumível de bordo, fabricado na oficina.
pub fn flask_definition(kind: FlaskKind) -> marvyr_domain_items::ItemDefinition {
    marvyr_domain_items::ItemDefinition {
        id: kind.item_id(),
        kind: marvyr_domain_items::ItemKind::Consumable,
        equipment: None,
        max_stack: 3,
        base_weight: 2,
        tags: Default::default(),
        display_name: String::from(kind.item_name()),
    }
}

#[cfg(test)]
mod tests {
    use bevy::ecs::system::RunSystemOnce;
    use marvyr_domain_items::ItemInstance;
    use marvyr_domain_ships::ShipKind;
    use marvyr_domain_world::WorldMap;
    use marvyr_shared::ids::{CharacterId, ItemInstanceId, RegionId};

    use super::*;
    use crate::crafting::DevShips;
    use crate::net::{spawn_ship_for, DevItems, ShipIdCounter, DEFAULT_WORLD_SEED};

    fn ship_with_repair_flask() -> App {
        let mut app = App::new();
        app.insert_resource(DevItems::new())
            .insert_resource(DevShips::new())
            .init_resource::<FlaskHits>()
            .init_resource::<Time>()
            .add_systems(Update, tick_flasks);
        app.world_mut()
            .run_system_once(
                |mut commands: Commands, dev: Res<DevItems>, ships: Res<DevShips>| {
                    let map = WorldMap::from_seed(DEFAULT_WORLD_SEED);
                    let id = spawn_ship_for(
                        &mut commands,
                        &mut ShipIdCounter(7),
                        &dev,
                        &ships,
                        &map,
                        ShipKind::SmallMerchant,
                        None,
                        CharacterId::new(),
                        Vec::new(),
                        None,
                    );
                    assert_eq!(id, 7);
                },
            )
            .expect("spawn");
        let catalog = DevItems::new().catalog;
        let flask =
            ItemInstance::new_resource(ItemInstanceId::new(), FlaskKind::Repair.item_id(), 1);
        ship(&mut app).hold.insert(&catalog, flask).expect("cabe");
        app
    }

    fn ship(app: &mut App) -> Mut<'_, ServerShip> {
        let world = app.world_mut();
        world.query::<&mut ServerShip>().single_mut(world)
    }

    #[test]
    fn wire_shows_what_is_aboard_and_hits_refill_the_shooter() {
        let mut app = ship_with_repair_flask();
        let wire = wire(&ship(&mut app));
        assert_eq!(wire.aboard, 0b0001, "só a Estopa está no porão");
        assert_eq!(wire.active, 0);

        ship(&mut app).flasks.drink(FlaskKind::Repair).unwrap();
        assert_eq!(wire_of(&mut app).active, 0b0001);
        let spent = wire_of(&mut app).charges[0];
        app.world_mut()
            .resource_mut::<FlaskHits>()
            .0
            .extend([(7, false), (99, true)]);
        app.update();
        assert_eq!(
            wire_of(&mut app).charges[0],
            spent + marvyr_domain_combat::flask::CHARGES_PER_HIT,
            "só o acerto deste navio conta"
        );

        ship(&mut app).presence = VesselPresence::Docked(RegionId::new());
        app.update();
        assert_eq!(
            wire_of(&mut app).charges,
            [marvyr_domain_combat::flask::MAX_CHARGES; 4],
            "porto enche"
        );
    }

    fn wire_of(app: &mut App) -> FlaskWire {
        wire(&ship(app))
    }
}
