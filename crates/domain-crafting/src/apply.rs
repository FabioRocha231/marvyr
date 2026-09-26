//! Aplicação de receitas (PRD §37, MF-021/037). `can_craft` julga; `craft`
//! e `craft_in_storage` executam: consomem ingredientes e produzem o output
//! — tudo-ou-nada. No porão, a checagem de peso do output acontece ANTES de
//! consumir nada. No storage (MF-037: a oficina do porto), não há peso — a
//! atomicidade é garantida pela retirada exata e fail-closed.

use std::collections::HashMap;

use marvyr_domain_items::{
    put_stack, roll_quality, take_stacks, CargoHold, Custody, ItemCatalog, ItemInstance,
    ItemLocation,
};
use marvyr_shared::ids::{ItemDefinitionId, ItemInstanceId, RegionId};

use crate::recipe::{Recipe, StationKind};
use crate::validate::{can_craft, CraftError, InventoryView};

/// Durabilidade de saída de equipamento dev (§39: tuning é conteúdo).
const DEV_EQUIPMENT_DURABILITY: u16 = 100;

/// Executa a receita sobre o porão. Puro no sentido de regra: toda
/// invalidade é erro ANTES de qualquer mutação (fail-closed, §37).
pub fn craft(
    recipe: &Recipe,
    hold: &mut CargoHold,
    catalog: &ItemCatalog,
    station: StationKind,
) -> Result<ItemInstance, CraftError> {
    let mut quantities: HashMap<marvyr_shared::ids::ItemDefinitionId, u32> = HashMap::new();
    for custody in hold.items() {
        *quantities.entry(custody.instance.definition).or_insert(0) += custody.instance.quantity;
    }
    // Definições relevantes para a validação: output e ingredientes. Output
    // ausente do catálogo cai no UnknownOutputItem de `can_craft`.
    let mut definitions = HashMap::new();
    let mut reference = |id: marvyr_shared::ids::ItemDefinitionId| {
        if let Some(definition) = catalog.get(id) {
            definitions.insert(id, definition);
        }
    };
    reference(recipe.output_item);
    for ingredient in &recipe.ingredients {
        reference(ingredient.item);
    }

    let inventory = InventoryView {
        quantities,
        free_slots: u32::MAX, // porão não tem limite de slots, só de peso
        definitions,
        station,
    };
    can_craft(recipe, &inventory)?;

    // A saída existe no catálogo (can_craft garante): monta a instância.
    let output_definition = catalog
        .get(recipe.output_item)
        .expect("can_craft garante output conhecido");
    let output = if output_definition.is_equipment() {
        ItemInstance::new_equipment(
            ItemInstanceId::new(),
            recipe.output_item,
            DEV_EQUIPMENT_DURABILITY,
        )
    } else {
        ItemInstance::new_resource(
            ItemInstanceId::new(),
            recipe.output_item,
            recipe.output_quantity,
        )
    };

    // Peso da saída cabe? Confere ANTES de consumir: nada se perde no meio.
    hold.can_accept(catalog, output.definition, output.quantity)?;

    // Consome e produz. As remoções não podem falhar: `can_craft` validou
    // as quantidades totais há duas linhas.
    for ingredient in &recipe.ingredients {
        hold.remove(ingredient.item, ingredient.quantity)
            .expect("can_craft validou os ingredientes");
    }
    hold.insert(catalog, output.clone())
        .expect("can_accept conferiu o peso acima");
    Ok(output)
}

/// Executa a receita sobre o STORAGE regional do porto (MF-037): insumos
/// saem do `PortStorage(region)`, output volta para o mesmo storage. Craft
/// é atividade portuária — o porão carrega o que está viajando, o storage
/// guarda a riqueza da região, e o jogador decide o que embarca.
///
/// Puro no sentido de regra: qualquer invalidade (estação, insumo) é erro
/// ANTES de qualquer mutação (fail-closed, §37/§69).
pub fn craft_in_storage(
    recipe: &Recipe,
    storage: &mut Vec<Custody>,
    catalog: &ItemCatalog,
    station: StationKind,
    region: RegionId,
) -> Result<ItemInstance, CraftError> {
    let mut quantities: HashMap<ItemDefinitionId, u32> = HashMap::new();
    for custody in storage.iter() {
        *quantities.entry(custody.instance.definition).or_insert(0) += custody.instance.quantity;
    }
    let mut definitions = HashMap::new();
    let mut reference = |id: ItemDefinitionId| {
        if let Some(definition) = catalog.get(id) {
            definitions.insert(id, definition);
        }
    };
    reference(recipe.output_item);
    for ingredient in &recipe.ingredients {
        reference(ingredient.item);
    }

    // Storage não tem limite de slots nem de peso: a validação que importa
    // é estação + insumos + output conhecido.
    let inventory = InventoryView {
        quantities,
        free_slots: u32::MAX,
        definitions,
        station,
    };
    can_craft(recipe, &inventory)?;

    // Consome do storage (retirada exata e atômica — `can_craft` validou as
    // quantidades agregadas; o destino é descartado: o item foi CONSUMIDO).
    for ingredient in &recipe.ingredients {
        take_stacks(
            storage,
            ingredient.item,
            ingredient.quantity,
            ItemLocation::PortStorage(region),
        )
        .expect("can_craft validou os ingredientes");
    }

    let output_definition = catalog
        .get(recipe.output_item)
        .expect("can_craft garante output conhecido");
    let output = if output_definition.is_equipment() {
        let id = ItemInstanceId::new();
        ItemInstance {
            // A id da peça é a semente: os afixos ficam atados a ela.
            quality: roll_quality(recipe.output_rarity, id.0.as_u64_pair().0),
            ..ItemInstance::new_equipment(id, recipe.output_item, DEV_EQUIPMENT_DURABILITY)
        }
    } else {
        ItemInstance::new_resource(
            ItemInstanceId::new(),
            recipe.output_item,
            recipe.output_quantity,
        )
    };
    let location = ItemLocation::PortStorage(region);
    put_stack(
        storage,
        Custody {
            instance: output.clone(),
            location,
        },
        output_definition.max_stack,
    );
    Ok(output)
}

#[cfg(test)]
mod tests {
    use smallvec::SmallVec;

    use marvyr_domain_items::{CargoHold, ItemCatalog, ItemDefinition, ItemInstance, ItemKind};
    use marvyr_shared::ids::{ItemDefinitionId, ItemInstanceId, RecipeId, ShipInstanceId};

    use super::craft;
    use crate::recipe::{Ingredient, Recipe, StationKind};

    fn resource(id: ItemDefinitionId, name: &str, weight: u32) -> ItemDefinition {
        ItemDefinition {
            id,
            kind: ItemKind::Resource,
            equipment: None,
            max_stack: 100,
            base_weight: weight,
            tags: SmallVec::new(),
            display_name: String::from(name),
        }
    }

    fn equipment(id: ItemDefinitionId, name: &str, weight: u32) -> ItemDefinition {
        ItemDefinition {
            id,
            kind: ItemKind::Equipment,
            equipment: Some(marvyr_domain_items::EquipmentDefinition {
                slot: marvyr_domain_items::EquipmentSlot::Hull,
                stats: marvyr_domain_items::EquipmentStats::default(),
            }),
            max_stack: 1,
            base_weight: weight,
            tags: SmallVec::new(),
            display_name: String::from(name),
        }
    }

    fn catalog_with(definitions: &[ItemDefinition]) -> ItemCatalog {
        let mut catalog = ItemCatalog::default();
        for definition in definitions {
            catalog.register(definition.clone()).unwrap();
        }
        catalog
    }

    fn recipe(output: ItemDefinitionId, ingredients: Vec<Ingredient>) -> Recipe {
        Recipe {
            id: RecipeId::new(),
            display_name: String::from("receita de teste"),
            output_item: output,
            output_quantity: 1,
            ingredients,
            required_station: StationKind::Workbench,
            craft_time_secs: 0,
            output_rarity: Default::default(),
        }
    }

    #[test]
    fn craft_consumes_ingredients_and_produces_output() {
        let wood = ItemDefinitionId::new();
        let hull = ItemDefinitionId::new();
        let catalog = catalog_with(&[resource(wood, "Madeira", 2), equipment(hull, "Casco", 8)]);
        let mut hold = CargoHold::new(ShipInstanceId::new(), 100);
        hold.insert(
            &catalog,
            ItemInstance::new_resource(ItemInstanceId::new(), wood, 20),
        )
        .unwrap();

        let output = craft(
            &recipe(
                hull,
                vec![Ingredient {
                    item: wood,
                    quantity: 15,
                }],
            ),
            &mut hold,
            &catalog,
            StationKind::Workbench,
        )
        .unwrap();

        assert_eq!(output.definition, hull);
        assert!(output.durability.is_some());
        // 40 de peso de madeira - 30 consumidos + 8 do casco = 18.
        assert_eq!(hold.used_weight(&catalog).unwrap(), 18);
    }

    #[test]
    fn craft_without_station_fails_and_consumes_nothing() {
        let wood = ItemDefinitionId::new();
        let hull = ItemDefinitionId::new();
        let catalog = catalog_with(&[resource(wood, "Madeira", 2), equipment(hull, "Casco", 8)]);
        let mut hold = CargoHold::new(ShipInstanceId::new(), 100);
        hold.insert(
            &catalog,
            ItemInstance::new_resource(ItemInstanceId::new(), wood, 20),
        )
        .unwrap();

        let error = craft(
            &recipe(
                hull,
                vec![Ingredient {
                    item: wood,
                    quantity: 15,
                }],
            ),
            &mut hold,
            &catalog,
            crate::recipe::StationKind::None,
        )
        .unwrap_err();

        assert!(matches!(
            error,
            crate::validate::CraftError::WrongStation { .. }
        ));
        assert_eq!(hold.used_weight(&catalog).unwrap(), 40); // intacto
    }

    #[test]
    fn craft_is_atomic_when_output_would_overflow_weight() {
        let wood = ItemDefinitionId::new();
        let hull = ItemDefinitionId::new();
        // Casco pesa 60: porão de 90 com 40 de madeira deixa 50 livres —
        // consumir 15 (30 de peso) caberia, mas o casco de 60 não.
        let catalog = catalog_with(&[resource(wood, "Madeira", 2), equipment(hull, "Casco", 60)]);
        let mut hold = CargoHold::new(ShipInstanceId::new(), 90);
        hold.insert(
            &catalog,
            ItemInstance::new_resource(ItemInstanceId::new(), wood, 20),
        )
        .unwrap();

        let error = craft(
            &recipe(
                hull,
                vec![Ingredient {
                    item: wood,
                    quantity: 5,
                }],
            ),
            &mut hold,
            &catalog,
            StationKind::Workbench,
        )
        .unwrap_err();

        assert!(matches!(
            error,
            crate::validate::CraftError::Cargo(
                marvyr_domain_items::CargoError::CargoCapacityExceeded { .. }
            )
        ));
        // Nada foi consumido: o porão segue com as 20 madeiras inteiras.
        assert_eq!(hold.used_weight(&catalog).unwrap(), 40);
        assert_eq!(hold.items().len(), 1);
    }
}

/// Testes do fluxo de oficina (MF-037): insumos e output vivem no storage.
#[cfg(test)]
mod storage_tests {
    use smallvec::SmallVec;

    use marvyr_domain_items::{
        quantity_of, Custody, ItemCatalog, ItemDefinition, ItemInstance, ItemKind, ItemLocation,
    };
    use marvyr_shared::ids::{ItemDefinitionId, ItemInstanceId, RecipeId, RegionId};

    use super::craft_in_storage;
    use crate::recipe::{Ingredient, Recipe, StationKind};

    fn resource(id: ItemDefinitionId, name: &str, weight: u32) -> ItemDefinition {
        ItemDefinition {
            id,
            kind: ItemKind::Resource,
            equipment: None,
            max_stack: 100,
            base_weight: weight,
            tags: SmallVec::new(),
            display_name: String::from(name),
        }
    }

    fn equipment(id: ItemDefinitionId, name: &str, weight: u32) -> ItemDefinition {
        ItemDefinition {
            id,
            kind: ItemKind::Equipment,
            equipment: Some(marvyr_domain_items::EquipmentDefinition {
                slot: marvyr_domain_items::EquipmentSlot::Hull,
                stats: marvyr_domain_items::EquipmentStats::default(),
            }),
            max_stack: 1,
            base_weight: weight,
            tags: SmallVec::new(),
            display_name: String::from(name),
        }
    }

    fn catalog_with(definitions: &[ItemDefinition]) -> ItemCatalog {
        let mut catalog = ItemCatalog::default();
        for definition in definitions {
            catalog.register(definition.clone()).unwrap();
        }
        catalog
    }

    fn recipe(output: ItemDefinitionId, ingredients: Vec<Ingredient>) -> Recipe {
        Recipe {
            id: RecipeId::new(),
            display_name: String::from("receita de oficina"),
            output_item: output,
            output_quantity: 1,
            ingredients,
            required_station: StationKind::Workbench,
            craft_time_secs: 0,
            output_rarity: Default::default(),
        }
    }

    fn stored(item: ItemDefinitionId, quantity: u32, region: RegionId) -> Custody {
        Custody {
            instance: ItemInstance::new_resource(ItemInstanceId::new(), item, quantity),
            location: ItemLocation::PortStorage(region),
        }
    }

    #[test]
    fn rare_craft_rolls_affixes_onto_the_stored_piece() {
        let wood = ItemDefinitionId::new();
        let coral = ItemDefinitionId::new();
        let hull = ItemDefinitionId::new();
        let catalog = catalog_with(&[
            resource(wood, "Madeira", 1),
            resource(coral, "Coral", 1),
            equipment(hull, "Casco", 8),
        ]);
        let region = RegionId::new();
        let mut storage = vec![stored(wood, 30, region), stored(coral, 2, region)];
        let base = recipe(
            hull,
            vec![Ingredient {
                item: wood,
                quantity: 10,
            }],
        );

        let output = craft_in_storage(
            &base.at_rarity(marvyr_domain_items::Rarity::Rare, coral),
            &mut storage,
            &catalog,
            StationKind::Workbench,
            region,
        )
        .unwrap();

        assert_eq!(output.rarity(), marvyr_domain_items::Rarity::Rare);
        assert!((3..=4).contains(&output.affixes().len()));
        let stored = storage
            .iter()
            .find(|c| c.instance.id == output.id)
            .expect("peça no storage");
        assert_eq!(stored.instance.quality, output.quality, "afixos persistem");
        assert_eq!(quantity_of(&storage, wood), 0);
        assert_eq!(quantity_of(&storage, coral), 0);
    }

    #[test]
    fn storage_craft_consumes_from_and_returns_to_storage() {
        let wood = ItemDefinitionId::new();
        let hull = ItemDefinitionId::new();
        let catalog = catalog_with(&[resource(wood, "Madeira", 2), equipment(hull, "Casco", 8)]);
        let region = RegionId::new();
        let mut storage = vec![stored(wood, 20, region)];

        let output = craft_in_storage(
            &recipe(
                hull,
                vec![Ingredient {
                    item: wood,
                    quantity: 15,
                }],
            ),
            &mut storage,
            &catalog,
            StationKind::Workbench,
            region,
        )
        .unwrap();

        assert_eq!(output.definition, hull);
        // 20 madeira - 15 consumidas = 5, e o casco entrou no storage.
        assert_eq!(quantity_of(&storage, wood), 5);
        assert_eq!(quantity_of(&storage, hull), 1);
        assert!(storage
            .iter()
            .all(|custody| custody.location == ItemLocation::PortStorage(region)));
    }

    #[test]
    fn storage_craft_refuses_ingredients_that_are_only_in_the_hold() {
        // MF-037: o porão NÃO é insumo automático. Sem madeira no storage,
        // o craft falha mesmo que o porão estivesse cheio — quem embarca
        // decide o que embarca.
        let wood = ItemDefinitionId::new();
        let hull = ItemDefinitionId::new();
        let catalog = catalog_with(&[resource(wood, "Madeira", 2), equipment(hull, "Casco", 8)]);
        let region = RegionId::new();
        let mut storage: Vec<Custody> = Vec::new();

        let error = craft_in_storage(
            &recipe(
                hull,
                vec![Ingredient {
                    item: wood,
                    quantity: 15,
                }],
            ),
            &mut storage,
            &catalog,
            StationKind::Workbench,
            region,
        )
        .unwrap_err();

        assert!(matches!(
            error,
            crate::validate::CraftError::MissingIngredient {
                needed: 15,
                available: 0,
                ..
            }
        ));
        assert!(storage.is_empty());
    }

    #[test]
    fn storage_craft_is_atomic_when_station_is_wrong() {
        let wood = ItemDefinitionId::new();
        let hull = ItemDefinitionId::new();
        let catalog = catalog_with(&[resource(wood, "Madeira", 2), equipment(hull, "Casco", 8)]);
        let region = RegionId::new();
        let mut storage = vec![stored(wood, 20, region)];

        let error = craft_in_storage(
            &recipe(
                hull,
                vec![Ingredient {
                    item: wood,
                    quantity: 15,
                }],
            ),
            &mut storage,
            &catalog,
            StationKind::Dock,
            region,
        )
        .unwrap_err();

        assert!(matches!(
            error,
            crate::validate::CraftError::WrongStation { .. }
        ));
        assert_eq!(quantity_of(&storage, wood), 20, "storage intacto");
    }
}
