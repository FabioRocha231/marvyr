//! Loadout no servidor (MF-039, PRD §40). Equipar é serviço de porto: só
//! ATRACADO, com o item vindo do STORAGE regional da doca. Slot ocupado é
//! swap — a instância antiga volta inteira ao storage (nunca é destruída) —
//! e os stats do navio são recalculados no ato pelo modelo puro
//! (`compute_ship_stats`), com a fração de casco preservada (`rescale_hp`).

use bevy::ecs::prelude::*;
use bevy::prelude::*;
use lightyear::prelude::server::*;
use lightyear::prelude::*;
use marvyr_domain_items::{EquipmentSlot, ItemCatalog};
use marvyr_domain_ships::{can_equip, compute_ship_stats, rescale_hp, ShipDefinition};
use marvyr_protocol::{EquipItem, LoadoutLine, LoadoutResult, LoadoutSnapshot, UnequipItem};
use tracing::info;

use crate::net::{DevItems, ReliableChannel, ServerShip};
use crate::sets::SimulationSet;

/// Envia o snapshot do loadout a partir de uma lista de custódias instaladas
/// — usado no hello (navio recém-restaurado ainda não está no ECS).
pub(crate) fn send_loadout_snapshot(
    connection_manager: &mut ConnectionManager,
    client_id: ClientId,
    ship_definition: &ShipDefinition,
    catalog: &ItemCatalog,
    equipped: &[marvyr_domain_items::Custody],
) {
    let _ = connection_manager.send_message::<ReliableChannel, _>(
        client_id,
        &loadout_snapshot_for(ship_definition, catalog, equipped),
    );
}

pub struct LoadoutPlugin;

impl Plugin for LoadoutPlugin {
    fn build(&self, app: &mut App) {
        app.add_systems(
            FixedUpdate,
            (handle_equip, handle_unequip).in_set(SimulationSet::Input),
        );
    }
}

/// O loadout do PRÓPRIO navio para o client: uma linha por slot DECLARADO
/// na definição do casco, com o nome do item instalado (ou vazio).
pub(crate) fn loadout_snapshot_for(
    ship_definition: &ShipDefinition,
    catalog: &ItemCatalog,
    equipped: &[marvyr_domain_items::Custody],
) -> LoadoutSnapshot {
    LoadoutSnapshot {
        slots: ship_definition
            .slots
            .iter()
            .map(|spec| {
                let installed = equipped.iter().find(|custody| {
                    matches!(
                        custody.location,
                        marvyr_domain_items::ItemLocation::Equipped { slot, .. } if slot == spec.kind
                    )
                });
                match installed {
                    Some(custody) => LoadoutLine {
                        slot: spec.kind,
                        item_name: catalog
                            .get(custody.instance.definition)
                            .map(|definition| definition.display_name.clone())
                            .unwrap_or_default(),
                        equipped: true,
                        quality: custody.instance.quality.clone(),
                    },
                    None => LoadoutLine {
                        slot: spec.kind,
                        item_name: String::new(),
                        equipped: false,
                        quality: None,
                    },
                }
            })
            .collect(),
    }
}

fn loadout_result(
    connection_manager: &mut ConnectionManager,
    client_id: ClientId,
    success: bool,
    reason: &str,
) {
    let _ = connection_manager.send_message::<ReliableChannel, _>(
        client_id,
        &LoadoutResult {
            success,
            reason: String::from(reason),
        },
    );
}

/// Recalcula os stats com o loadout vigente mantendo a fração de casco:
/// navio inteiro que instala casco reforçado ganha os pontos dele; avariado
/// segue avariado na mesma proporção.
fn recalc(ship: &mut ServerShip, dev_ships: &crate::crafting::DevShips, dev: &DevItems) {
    let stats = compute_ship_stats(
        dev_ships.definition(ship.kind),
        &ship.loadout.components(),
        &dev.catalog,
    )
    .expect("loadout só contém definições do catálogo (can_equip validou)");
    ship.hold.set_capacity(stats.cargo_capacity);
    ship.hp = rescale_hp(ship.hp, ship.stats.max_hp, stats.max_hp);
    ship.stats = stats;
}

fn send_loadout(
    connection_manager: &mut ConnectionManager,
    client_id: ClientId,
    ship: &ServerShip,
    dev_ships: &crate::crafting::DevShips,
    dev: &DevItems,
) {
    let equipped: Vec<_> = ship.loadout.items().cloned().collect();
    let _ = connection_manager.send_message::<ReliableChannel, _>(
        client_id,
        &loadout_snapshot_for(dev_ships.definition(ship.kind), &dev.catalog, &equipped),
    );
}

/// Equipar (MF-039): PortStorage → Equipped(ship, slot), atomicamente.
/// Validação pura (`can_equip`) antes de qualquer mutação; a única etapa
/// falível é a retirada do storage — tudo depois dela é bookkeeping.
// System Bevy: params são injeção de dependência, não assinatura.
#[allow(clippy::too_many_arguments)]
pub fn handle_equip(
    mut equip_events: EventReader<ServerReceiveMessage<EquipItem>>,
    mut connection_manager: ResMut<ConnectionManager>,
    dev: Res<DevItems>,
    dev_ships: Res<crate::crafting::DevShips>,
    mut market: ResMut<crate::market::ServerMarket>,
    mut ships: Query<&mut ServerShip>,
) {
    for event in equip_events.read() {
        let client_id = event.from();
        let item = event.message().item;
        let Some(mut ship) = ships
            .iter_mut()
            .find(|ship| ship.client_id == Some(client_id))
        else {
            continue;
        };
        let marvyr_domain_ships::VesselPresence::Docked(region) = ship.presence else {
            loadout_result(
                &mut connection_manager,
                client_id,
                false,
                "atraca primeiro (E) — equipar é serviço de porto",
            );
            continue;
        };
        let character = ship.character;

        // 1. Fail-closed puro: item conhecido, é equipamento, o casco tem o slot.
        let Some(definition) = dev.catalog.get(item) else {
            loadout_result(
                &mut connection_manager,
                client_id,
                false,
                "item desconhecido",
            );
            continue;
        };
        let slot = match can_equip(dev_ships.definition(ship.kind), definition) {
            Ok(slot) => slot,
            Err(error) => {
                info!(ship_id = ship.ship_id, error = %error, "equipar recusado");
                loadout_result(
                    &mut connection_manager,
                    client_id,
                    false,
                    &error.to_string(),
                );
                continue;
            }
        };

        // 2. Retira a instância do storage (a única etapa falível).
        match market.take_one_from_storage(character, region, item, event.message().instance) {
            Ok(custody) => {
                // 3. Swap: instala o novo; o antigo sai VIVO e volta ao storage.
                let ship_instance = ship.ship_instance;
                let displaced = ship.loadout.equip(ship_instance, custody, slot);
                if let Some(old) = displaced {
                    market.return_to_storage(character, region, old, &dev.catalog);
                }
                // 4. Stats autoritativos, já no próximo snapshot do dono.
                recalc(&mut ship, &dev_ships, &dev);
                info!(
                    ship_id = ship.ship_id,
                    slot = ?slot,
                    hp = ship.hp,
                    max_hp = ship.stats.max_hp,
                    "item instalado; stats recalculados"
                );
                loadout_result(&mut connection_manager, client_id, true, "item instalado");
                send_loadout(&mut connection_manager, client_id, &ship, &dev_ships, &dev);
            }
            Err(_) => {
                info!(
                    ship_id = ship.ship_id,
                    "equipar recusado: item não está no storage desta região"
                );
                loadout_result(
                    &mut connection_manager,
                    client_id,
                    false,
                    "o item está no porão ou em outro porto: deposite no storage (Z)",
                );
            }
        }
    }
}

/// Desequipar (MF-039): Equipped(ship, slot) → PortStorage, com recálculo.
pub fn handle_unequip(
    mut unequip_events: EventReader<ServerReceiveMessage<UnequipItem>>,
    mut connection_manager: ResMut<ConnectionManager>,
    dev: Res<DevItems>,
    dev_ships: Res<crate::crafting::DevShips>,
    mut market: ResMut<crate::market::ServerMarket>,
    mut ships: Query<&mut ServerShip>,
) {
    for event in unequip_events.read() {
        let client_id = event.from();
        let slot: EquipmentSlot = event.message().slot;
        let Some(mut ship) = ships
            .iter_mut()
            .find(|ship| ship.client_id == Some(client_id))
        else {
            continue;
        };
        let marvyr_domain_ships::VesselPresence::Docked(region) = ship.presence else {
            loadout_result(
                &mut connection_manager,
                client_id,
                false,
                "atraca primeiro (E) — desequipar é serviço de porto",
            );
            continue;
        };
        let character = ship.character;

        let Some(custody) = ship.loadout.unequip(slot) else {
            loadout_result(
                &mut connection_manager,
                client_id,
                false,
                "slot já está vazio",
            );
            continue;
        };
        market.return_to_storage(character, region, custody, &dev.catalog);
        recalc(&mut ship, &dev_ships, &dev);
        info!(
            ship_id = ship.ship_id,
            slot = ?slot,
            hp = ship.hp,
            max_hp = ship.stats.max_hp,
            "item devolvido ao storage; stats recalculados"
        );
        loadout_result(&mut connection_manager, client_id, true, "item no storage");
        send_loadout(&mut connection_manager, client_id, &ship, &dev_ships, &dev);
    }
}
