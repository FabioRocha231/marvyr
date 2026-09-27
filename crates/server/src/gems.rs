//! Gemas de suporte no servidor (v24). Encaixar é serviço de porto, como
//! equipar: a gema sai do armazém da doca e passa a morar DENTRO da peça
//! instalada (`Quality::gems`); tirar devolve ao armazém.
//!
//! Dois lugares persistidos mudam (armazém e navio), então a ordem é a que
//! falha para o lado da perda, nunca da cópia: encaixar grava o armazém sem
//! a gema e depois o navio; tirar grava o navio sem a gema e depois o
//! armazém. Navio que não grava desfaz a operação.

use std::collections::HashMap;

use bevy::prelude::*;
use lightyear::prelude::server::*;
use lightyear::prelude::*;
use marvyr_domain_items::{gem, Custody, GemKind, ItemInstance, ItemLocation};
use marvyr_domain_ships::VesselPresence;
use marvyr_protocol::{SocketGem, UnsocketGem};
use marvyr_shared::ids::ItemInstanceId;
use tracing::{info, warn};

use crate::loadout::{loadout_result, recalc, send_loadout};
use crate::net::{DevItems, ServerShip};
use crate::sets::SimulationSet;

/// Cada troca grava o navio no banco: meio segundo entre elas por capitão.
const GEM_COOLDOWN_SECS: f64 = 0.5;

pub struct GemPlugin;

impl Plugin for GemPlugin {
    fn build(&self, app: &mut App) {
        app.add_systems(
            FixedUpdate,
            (handle_socket, handle_unsocket).in_set(SimulationSet::Input),
        );
    }
}

/// Atracado e descansado; senão o motivo para o jogador.
fn gem_service(
    ship: &ServerShip,
    last: &mut HashMap<ClientId, f64>,
    client_id: ClientId,
    now: f64,
) -> Result<marvyr_shared::ids::RegionId, &'static str> {
    let VesselPresence::Docked(region) = ship.presence else {
        return Err("atraca primeiro (E) — gema é serviço de porto");
    };
    if last
        .get(&client_id)
        .is_some_and(|at| now - at < GEM_COOLDOWN_SECS)
    {
        return Err("devagar com as gemas");
    }
    last.insert(client_id, now);
    Ok(region)
}

const NOT_RECORDED: &str = "o porto não conseguiu registrar; tente de novo";

/// Armazém → peça no `slot`. `save` grava o navio; falhou, desfaz tudo.
pub(crate) fn socket_gem(
    ship: &mut ServerShip,
    market: &mut crate::market::ServerMarket,
    catalog: &marvyr_domain_items::ItemCatalog,
    region: marvyr_shared::ids::RegionId,
    slot: marvyr_domain_items::EquipmentSlot,
    gem: GemKind,
    save: impl Fn(&ServerShip, &crate::market::ServerMarket) -> Result<(), String>,
) -> Result<(), &'static str> {
    let piece = ship.loadout.get(slot).ok_or("não há peça nesse slot")?;
    if piece.instance.gems().len() >= gem::socket_count(piece.instance.rarity()) {
        return Err("sem encaixe livre nesta peça");
    }
    let character = ship.character;
    // Sai do armazém e entra na peça em memória; armazém e navio gravam
    // juntos (uma transação). Não gravou: desfaz os dois, nada foi escrito.
    let taken = market
        .take_one_unsaved(character, region, gem.item_id(), None)
        .map_err(|_| "essa gema não está no armazém deste porto")?;
    let piece = ship.loadout.get_mut(slot).expect("peça vista acima");
    gem::socket(&mut piece.instance.quality, gem).expect("vaga conferida acima");
    if let Err(error) = save(ship, market) {
        warn!(%error, ship_id = ship.ship_id, "encaixe desfeito: não gravou");
        let piece = ship.loadout.get_mut(slot).expect("peça vista acima");
        let last_socket = piece.instance.gems().len() - 1;
        let _ = gem::unsocket(&mut piece.instance.quality, last_socket);
        market.stash(character, region, vec![taken], catalog);
        return Err(NOT_RECORDED);
    }
    Ok(())
}

/// Peça no `slot` → armazém. `save` grava o navio; falhou, a gema fica.
pub(crate) fn unsocket_gem(
    ship: &mut ServerShip,
    market: &mut crate::market::ServerMarket,
    catalog: &marvyr_domain_items::ItemCatalog,
    region: marvyr_shared::ids::RegionId,
    slot: marvyr_domain_items::EquipmentSlot,
    index: usize,
    save: impl Fn(&ServerShip, &crate::market::ServerMarket) -> Result<(), String>,
) -> Result<GemKind, &'static str> {
    // Sai da peça e volta ao armazém em memória; os dois gravam juntos.
    let piece = ship.loadout.get_mut(slot).ok_or("não há peça nesse slot")?;
    let before = piece.instance.quality.clone();
    let gem = gem::unsocket(&mut piece.instance.quality, index)
        .map_err(|_| "não há gema nesse encaixe")?;
    let custody = Custody {
        instance: ItemInstance::new_resource(ItemInstanceId::new(), gem.item_id(), 1),
        location: ItemLocation::PortStorage(region),
    };
    let character = ship.character;
    market.stash(character, region, vec![custody], catalog);
    if let Err(error) = save(ship, market) {
        warn!(%error, ship_id = ship.ship_id, "remoção desfeita: não gravou");
        ship.loadout
            .get_mut(slot)
            .expect("peça vista acima")
            .instance
            .quality = before;
        let _ = market.take_one_unsaved(character, region, gem.item_id(), None);
        return Err(NOT_RECORDED);
    }
    Ok(gem)
}

#[allow(clippy::too_many_arguments)]
fn handle_socket(
    time: Res<Time>,
    mut events: EventReader<ServerReceiveMessage<SocketGem>>,
    mut connection_manager: ResMut<ConnectionManager>,
    dev: Res<DevItems>,
    dev_ships: Res<crate::crafting::DevShips>,
    mut market: ResMut<crate::market::ServerMarket>,
    mut ships: Query<&mut ServerShip>,
    mut last: Local<HashMap<ClientId, f64>>,
) {
    let now = time.elapsed_secs_f64();
    for event in events.read() {
        let client_id = event.from();
        let SocketGem { slot, gem } = *event.message();
        let Some(mut ship) = ships.iter_mut().find(|s| s.client_id == Some(client_id)) else {
            continue;
        };
        let done = gem_service(&ship, &mut last, client_id, now).and_then(|region| {
            socket_gem(
                &mut ship,
                &mut market,
                &dev.catalog,
                region,
                slot,
                gem,
                |ship, market| market.persist_with_ship(Some(&crate::net::ship_record(ship))),
            )
        });
        if let Err(reason) = done {
            loadout_result(&mut connection_manager, client_id, false, reason);
            continue;
        }
        recalc(&mut ship, &dev_ships, &dev);
        info!(ship_id = ship.ship_id, ?slot, ?gem, "gema encaixada");
        loadout_result(&mut connection_manager, client_id, true, "gema encaixada");
        send_loadout(&mut connection_manager, client_id, &ship, &dev_ships, &dev);
    }
}

#[allow(clippy::too_many_arguments)]
fn handle_unsocket(
    time: Res<Time>,
    mut events: EventReader<ServerReceiveMessage<UnsocketGem>>,
    mut connection_manager: ResMut<ConnectionManager>,
    dev: Res<DevItems>,
    dev_ships: Res<crate::crafting::DevShips>,
    mut market: ResMut<crate::market::ServerMarket>,
    mut ships: Query<&mut ServerShip>,
    mut last: Local<HashMap<ClientId, f64>>,
) {
    let now = time.elapsed_secs_f64();
    for event in events.read() {
        let client_id = event.from();
        let UnsocketGem { slot, index } = *event.message();
        let Some(mut ship) = ships.iter_mut().find(|s| s.client_id == Some(client_id)) else {
            continue;
        };
        let done = gem_service(&ship, &mut last, client_id, now).and_then(|region| {
            let index = usize::from(index);
            unsocket_gem(
                &mut ship,
                &mut market,
                &dev.catalog,
                region,
                slot,
                index,
                |ship, market| market.persist_with_ship(Some(&crate::net::ship_record(ship))),
            )
        });
        let gem = match done {
            Ok(gem) => gem,
            Err(reason) => {
                loadout_result(&mut connection_manager, client_id, false, reason);
                continue;
            }
        };
        recalc(&mut ship, &dev_ships, &dev);
        info!(
            ship_id = ship.ship_id,
            ?slot,
            ?gem,
            "gema de volta ao armazém"
        );
        loadout_result(&mut connection_manager, client_id, true, "gema no armazém");
        send_loadout(&mut connection_manager, client_id, &ship, &dev_ships, &dev);
    }
}

/// Gema solta no catálogo: recurso empilhável, fabricado na oficina.
pub fn gem_definition(gem: GemKind) -> marvyr_domain_items::ItemDefinition {
    marvyr_domain_items::ItemDefinition {
        id: gem.item_id(),
        kind: marvyr_domain_items::ItemKind::Resource,
        equipment: None,
        max_stack: 20,
        base_weight: 1,
        tags: Default::default(),
        display_name: String::from(gem.item_name()),
    }
}

#[cfg(test)]
mod tests {
    use bevy::ecs::system::RunSystemOnce;
    use marvyr_domain_items::{EquipmentSlot, Rarity};
    use marvyr_domain_ships::ShipKind;
    use marvyr_domain_world::WorldMap;
    use marvyr_shared::ids::{CharacterId, RegionId};

    use super::*;
    use crate::crafting::DevShips;
    use crate::market::ServerMarket;
    use crate::net::{spawn_ship_for, ShipIdCounter, DEFAULT_WORLD_SEED};

    /// Navio com um canhão Raro (3 encaixes) e 2 rubis no armazém.
    fn setup() -> (App, ServerMarket, RegionId) {
        let mut app = App::new();
        app.insert_resource(DevItems::new())
            .insert_resource(DevShips::new());
        let character = CharacterId::new();
        app.world_mut()
            .run_system_once(
                move |mut commands: Commands, dev: Res<DevItems>, ships: Res<DevShips>| {
                    let map = WorldMap::from_seed(DEFAULT_WORLD_SEED);
                    spawn_ship_for(
                        &mut commands,
                        &mut ShipIdCounter(1),
                        &dev,
                        &ships,
                        &map,
                        ShipKind::SmallMerchant,
                        None,
                        character,
                        Vec::new(),
                        None,
                    );
                },
            )
            .expect("spawn");
        let dev = DevItems::new();
        let region = RegionId::new();
        let world = app.world_mut();
        let mut ship = world.query::<&mut ServerShip>().single_mut(world);
        let piece = ItemInstance {
            quality: marvyr_domain_items::roll_quality(Rarity::Rare, 9),
            ..ItemInstance::new_equipment(ItemInstanceId::new(), dev.bronze_cannon, 100)
        };
        let ship_instance = ship.ship_instance;
        ship.loadout.equip(
            ship_instance,
            Custody {
                instance: piece,
                location: ItemLocation::PortStorage(region),
            },
            EquipmentSlot::Weapon,
        );
        let mut market = ServerMarket::new();
        market.grant_to_storage(character, region, GemKind::Ruby.item_id(), 2, &dev.catalog);
        (app, market, region)
    }

    fn rubies(market: &ServerMarket, ship: &ServerShip, region: RegionId) -> u32 {
        market.storage_quantity(ship.character, region, GemKind::Ruby.item_id())
    }

    fn socketed(ship: &ServerShip) -> Vec<GemKind> {
        ship.loadout
            .get(EquipmentSlot::Weapon)
            .unwrap()
            .instance
            .gems()
            .to_vec()
    }

    #[test]
    fn gem_moves_between_storage_and_piece_and_never_doubles() {
        let (mut app, mut market, region) = setup();
        let catalog = DevItems::new().catalog;
        let world = app.world_mut();
        let mut ship = world.query::<&mut ServerShip>().single_mut(world);
        let ok = |_: &ServerShip, _: &ServerMarket| Ok(());
        let weapon = EquipmentSlot::Weapon;

        socket_gem(
            &mut ship,
            &mut market,
            &catalog,
            region,
            weapon,
            GemKind::Ruby,
            ok,
        )
        .unwrap();
        socket_gem(
            &mut ship,
            &mut market,
            &catalog,
            region,
            weapon,
            GemKind::Ruby,
            ok,
        )
        .unwrap();
        assert_eq!(socketed(&ship), vec![GemKind::Ruby, GemKind::Ruby]);
        assert_eq!(rubies(&market, &ship, region), 0);
        assert_eq!(
            socket_gem(
                &mut ship,
                &mut market,
                &catalog,
                region,
                weapon,
                GemKind::Ruby,
                ok
            ),
            Err("essa gema não está no armazém deste porto")
        );
        assert_eq!(
            socket_gem(
                &mut ship,
                &mut market,
                &catalog,
                region,
                EquipmentSlot::Hull,
                GemKind::Ruby,
                ok
            ),
            Err("não há peça nesse slot")
        );

        let gem = unsocket_gem(&mut ship, &mut market, &catalog, region, weapon, 1, ok).unwrap();
        assert_eq!(gem, GemKind::Ruby);
        assert_eq!(socketed(&ship), vec![GemKind::Ruby]);
        assert_eq!(rubies(&market, &ship, region), 1);
        assert_eq!(
            unsocket_gem(&mut ship, &mut market, &catalog, region, weapon, 4, ok),
            Err("não há gema nesse encaixe")
        );
    }

    #[test]
    fn ship_that_does_not_save_undoes_the_move() {
        let (mut app, mut market, region) = setup();
        let catalog = DevItems::new().catalog;
        let world = app.world_mut();
        let mut ship = world.query::<&mut ServerShip>().single_mut(world);
        let broken = |_: &ServerShip, _: &ServerMarket| Err(String::from("banco fora"));
        let weapon = EquipmentSlot::Weapon;

        assert_eq!(
            socket_gem(
                &mut ship,
                &mut market,
                &catalog,
                region,
                weapon,
                GemKind::Ruby,
                broken
            ),
            Err(NOT_RECORDED)
        );
        assert!(socketed(&ship).is_empty(), "gema não fica na peça");
        assert_eq!(rubies(&market, &ship, region), 2, "volta ao armazém");

        socket_gem(
            &mut ship,
            &mut market,
            &catalog,
            region,
            weapon,
            GemKind::Ruby,
            |_, _| Ok(()),
        )
        .unwrap();
        assert_eq!(
            unsocket_gem(&mut ship, &mut market, &catalog, region, weapon, 0, broken),
            Err(NOT_RECORDED)
        );
        assert_eq!(socketed(&ship), vec![GemKind::Ruby], "segue encaixada");
        assert_eq!(rubies(&market, &ship, region), 1, "sem cópia no armazém");
    }
}
