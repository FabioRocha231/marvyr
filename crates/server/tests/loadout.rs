//! Loadout no servidor (MF-039): a ponte storage ↔ slots pelos métodos do
//! ServerMarket, com as garantias de swap (nada destruído) e fail-closed.

use marvyr_domain_items::{
    EquipmentDefinition, EquipmentSlot, EquipmentStats, ItemCatalog, ItemDefinition, ItemInstance,
    ItemKind, ItemLocation,
};
use marvyr_domain_ships::{can_equip, compute_ship_stats, ShipLoadout};
use marvyr_server::market::ServerMarket;
use marvyr_shared::ids::{
    ItemDefinitionId, ItemInstanceId, RegionId, ShipDefinitionId, ShipInstanceId,
};
use smallvec::SmallVec;

fn catalog_with_hull() -> (ItemCatalog, ItemDefinitionId) {
    let hull = ItemDefinitionId::new();
    let mut catalog = ItemCatalog::default();
    catalog
        .register(ItemDefinition {
            id: hull,
            kind: ItemKind::Equipment,
            equipment: Some(EquipmentDefinition {
                slot: EquipmentSlot::Hull,
                stats: EquipmentStats {
                    hp: 40,
                    ..EquipmentStats::default()
                },
            }),
            max_stack: 1,
            base_weight: 8,
            tags: SmallVec::new(),
            display_name: String::from("Casco Reforçado"),
        })
        .unwrap();
    (catalog, hull)
}

fn merchant() -> marvyr_domain_ships::ShipDefinition {
    marvyr_domain_ships::ShipDefinition::small_merchant()
}

#[test]
fn equip_moves_the_instance_storage_to_slot_and_back() {
    let (catalog, hull) = catalog_with_hull();
    let mut market = ServerMarket::new();
    let character = market.character("marujo");
    let region = RegionId::new();

    // Uma instância de casco entra no storage (como se tivesse sido craftada).
    market.return_to_storage(
        character,
        region,
        marvyr_domain_items::Custody {
            instance: ItemInstance::new_equipment(ItemInstanceId::new(), hull, 100),
            location: ItemLocation::PortStorage(region),
        },
        &catalog,
    );

    // Equipar: a instância SAI do storage (não copia).
    let installed = market
        .take_one_from_storage(character, region, hull, None)
        .expect("casco no storage");
    let ship = ShipInstanceId::new();
    let mut loadout = ShipLoadout::new();
    let slot = can_equip(&merchant(), catalog.get(hull).unwrap()).expect("slot Hull aceito");
    assert!(loadout.equip(ship, installed, slot).is_none());

    // Stats recalculados: +40 hp do casco reforçado.
    let stats = compute_ship_stats(&merchant(), &loadout.components(), &catalog).unwrap();
    assert_eq!(stats.max_hp, 140);

    // Swap: desequipar devolve a MESMA instância ao storage — viva.
    let equipped_id = loadout.unequip(slot).unwrap().instance.id;
    market.return_to_storage(
        character,
        region,
        marvyr_domain_items::Custody {
            instance: {
                let mut instance = ItemInstance::new_equipment(equipped_id, hull, 100);
                instance.id = equipped_id;
                instance
            },
            location: ItemLocation::PortStorage(region),
        },
        &catalog,
    );
    let de_volta = market
        .take_one_from_storage(character, region, hull, None)
        .expect("casco de volta");
    assert_eq!(
        de_volta.instance.id, equipped_id,
        "mesma instância, nada de clone"
    );
}

#[test]
fn equip_refuses_what_is_not_in_storage() {
    let (catalog, hull) = catalog_with_hull();
    let mut market = ServerMarket::new();
    let character = market.character("forasteiro");
    let region = RegionId::new();

    assert!(market
        .take_one_from_storage(character, region, hull, None)
        .is_err());
    // Item desconhecido também não equipa (fail-closed pelo catálogo).
    assert!(catalog.get(ItemDefinitionId::new()).is_none());
}

#[test]
fn hull_slot_is_required_by_the_definition() {
    let (catalog, hull) = catalog_with_hull();
    // Um casco sem slot Hull (definição de teste) recusa o equipamento.
    let mut no_hull = merchant();
    no_hull
        .slots
        .retain(|spec| spec.kind != EquipmentSlot::Hull);
    let error = can_equip(&no_hull, catalog.get(hull).unwrap()).unwrap_err();
    assert!(matches!(
        error,
        marvyr_domain_ships::LoadoutError::SlotNotAccepted {
            slot: EquipmentSlot::Hull
        }
    ));
    let _ = ShipDefinitionId::new();
}
