use marvyr_domain_items::ItemDefinition;
use std::collections::HashMap;

use crate::recipe::{Recipe, StationKind};

#[derive(Debug, thiserror::Error, PartialEq)]
pub enum CraftError {
    #[error("missing ingredient: item {item:?} needs {needed} but inventory has {available}")]
    MissingIngredient {
        item: marvyr_shared::ids::ItemDefinitionId,
        needed: u32,
        available: u32,
    },
    #[error("wrong station: recipe requires {required:?}, current is {current:?}")]
    WrongStation {
        required: StationKind,
        current: StationKind,
    },
    #[error("output is non-stackable but inventory has no slot")]
    NoRoomForOutput,
    #[error("recipe output {item:?} is not in the item catalog")]
    UnknownOutputItem {
        item: marvyr_shared::ids::ItemDefinitionId,
    },
    #[error("cargo rejected the craft: {0}")]
    Cargo(#[from] marvyr_domain_items::CargoError),
    /// MF-037: a oficina trabalha sobre o storage regional do porto — sem
    /// gaveta de storage para o personagem naquela região, não há oficina.
    #[error("o storage do porto está vazio: deposite materiais primeiro")]
    EmptyStorage,
}

#[derive(Debug)]
pub struct InventoryView<'a> {
    /// Quantidade disponível por definição de item.
    pub quantities: HashMap<marvyr_shared::ids::ItemDefinitionId, u32>,
    /// Número de slots livres (para outputs não-fungíveis).
    pub free_slots: u32,
    /// Definições conhecidas (para checar stackability). Saída sem definição
    /// conhecida é rejeitada com `CraftError::UnknownOutputItem`.
    pub definitions: HashMap<marvyr_shared::ids::ItemDefinitionId, &'a ItemDefinition>,
    /// Station atualmente disponível (None se não há).
    pub station: StationKind,
}

/// Verifica se o inventário satisfaz a receita.
/// Função pura: não muta nada.
pub fn can_craft<'a>(recipe: &Recipe, inventory: &InventoryView<'a>) -> Result<(), CraftError> {
    // Checar estação.
    if recipe.required_station != StationKind::None && recipe.required_station != inventory.station
    {
        return Err(CraftError::WrongStation {
            required: recipe.required_station,
            current: inventory.station,
        });
    }

    // Checar ingredientes.
    for ing in &recipe.ingredients {
        let have = inventory.quantities.get(&ing.item).copied().unwrap_or(0);
        if have < ing.quantity {
            return Err(CraftError::MissingIngredient {
                item: ing.item,
                needed: ing.quantity,
                available: have,
            });
        }
    }

    // Checar slot para output.
    let output_def = inventory.definitions.get(&recipe.output_item);
    let needs_slot = match output_def {
        Some(def) if def.is_fungible() => {
            // Fungível: verifica se há espaço no stack existente.
            let have = inventory
                .quantities
                .get(&recipe.output_item)
                .copied()
                .unwrap_or(0);
            have + recipe.output_quantity > def.max_stack
        }
        Some(_) => recipe.output_quantity > inventory.free_slots,
        None => {
            return Err(CraftError::UnknownOutputItem {
                item: recipe.output_item,
            })
        }
    };

    if needs_slot {
        return Err(CraftError::NoRoomForOutput);
    }

    Ok(())
}

#[cfg(test)]
mod tests {
    use std::collections::HashMap;

    use marvyr_domain_items::{ItemDefinition, ItemKind};
    use marvyr_shared::ids::{ItemDefinitionId, RecipeId};

    use super::{can_craft, InventoryView};
    use crate::recipe::{Recipe, StationKind};

    fn definition(kind: ItemKind, max_stack: u32) -> ItemDefinition {
        ItemDefinition {
            id: ItemDefinitionId::new(),
            kind,
            equipment: None,
            max_stack,
            base_weight: 1,
            tags: Default::default(),
            display_name: String::new(),
        }
    }

    fn recipe(
        output: ItemDefinitionId,
        output_quantity: u32,
        required_station: StationKind,
    ) -> Recipe {
        Recipe {
            id: RecipeId::new(),
            display_name: String::from("test recipe"),
            output_item: output,
            output_quantity,
            ingredients: Vec::new(),
            required_station,
            craft_time_secs: 1,
            output_rarity: Default::default(),
        }
    }

    fn view<'a>(
        quantities: Vec<(ItemDefinitionId, u32)>,
        free_slots: u32,
        definitions: &'a HashMap<ItemDefinitionId, ItemDefinition>,
        station: StationKind,
    ) -> InventoryView<'a> {
        InventoryView {
            quantities: quantities.into_iter().collect(),
            free_slots,
            definitions: definitions
                .iter()
                .map(|(id, definition)| (*id, definition))
                .collect(),
            station,
        }
    }

    #[test]
    fn crafts_when_ingredients_station_and_room_are_available() {
        let output = definition(ItemKind::Resource, 10);
        let output_id = output.id;
        let mut definitions = HashMap::new();
        definitions.insert(output_id, output);

        let inventory = view(
            vec![(output_id, 1)],
            0,
            &definitions,
            StationKind::Workbench,
        );

        assert_eq!(
            can_craft(&recipe(output_id, 1, StationKind::Workbench), &inventory),
            Ok(())
        );
    }

    #[test]
    fn fails_when_ingredient_is_missing() {
        let output = definition(ItemKind::Resource, 10);
        let ingredient = ItemDefinitionId::new();
        let recipe = Recipe {
            ingredients: vec![crate::recipe::Ingredient {
                item: ingredient,
                quantity: 5,
            }],
            ..recipe(output.id, 1, StationKind::None)
        };
        let definitions = HashMap::new();
        let inventory = view(vec![(ingredient, 2)], 0, &definitions, StationKind::None);

        assert_eq!(
            can_craft(&recipe, &inventory),
            Err(super::CraftError::MissingIngredient {
                item: ingredient,
                needed: 5,
                available: 2,
            })
        );
    }

    #[test]
    fn fails_on_wrong_station() {
        let output = definition(ItemKind::Resource, 10);
        let output_id = output.id;
        let mut definitions = HashMap::new();
        definitions.insert(output_id, output);
        let inventory = view(vec![], 0, &definitions, StationKind::Anvil);

        assert_eq!(
            can_craft(&recipe(output_id, 1, StationKind::Workbench), &inventory),
            Err(super::CraftError::WrongStation {
                required: StationKind::Workbench,
                current: StationKind::Anvil,
            })
        );
    }

    #[test]
    fn fails_when_non_fungible_output_has_no_free_slot() {
        let output = definition(ItemKind::Equipment, 1);
        let output_id = output.id;
        let mut definitions = HashMap::new();
        definitions.insert(output_id, output);
        let inventory = view(vec![], 0, &definitions, StationKind::None);

        assert_eq!(
            can_craft(&recipe(output_id, 1, StationKind::None), &inventory),
            Err(super::CraftError::NoRoomForOutput)
        );
    }

    #[test]
    fn fails_when_fungible_output_exceeds_max_stack() {
        let output = definition(ItemKind::Resource, 5);
        let output_id = output.id;
        let mut definitions = HashMap::new();
        definitions.insert(output_id, output);
        let inventory = view(vec![(output_id, 4)], 0, &definitions, StationKind::None);

        assert_eq!(
            can_craft(&recipe(output_id, 2, StationKind::None), &inventory),
            Err(super::CraftError::NoRoomForOutput)
        );
    }

    #[test]
    fn fails_when_output_definition_is_unknown() {
        let output_id = ItemDefinitionId::new();
        let definitions = HashMap::new();
        let inventory = view(vec![], 3, &definitions, StationKind::None);

        assert_eq!(
            can_craft(&recipe(output_id, 1, StationKind::None), &inventory),
            Err(super::CraftError::UnknownOutputItem { item: output_id })
        );
    }

    #[test]
    fn recipe_without_station_crafts_at_any_station() {
        let output = definition(ItemKind::Resource, 10);
        let output_id = output.id;
        let mut definitions = HashMap::new();
        definitions.insert(output_id, output);
        let inventory = view(vec![(output_id, 1)], 0, &definitions, StationKind::Dock);

        assert_eq!(
            can_craft(&recipe(output_id, 1, StationKind::None), &inventory),
            Ok(())
        );
    }
}
