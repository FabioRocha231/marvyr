use marvyr_domain_items::Rarity;
use marvyr_shared::ids::{ItemDefinitionId, RecipeId};
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum StationKind {
    None,
    Workbench,
    Anvil,
    Dock,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Ingredient {
    pub item: ItemDefinitionId,
    pub quantity: u32,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Recipe {
    pub id: RecipeId,
    pub display_name: String,
    pub output_item: ItemDefinitionId,
    pub output_quantity: u32,
    pub ingredients: Vec<Ingredient>,
    pub required_station: StationKind,
    pub craft_time_secs: u32,
    /// Raridade da peça produzida (só vale para equipamento).
    #[serde(default)]
    pub output_rarity: Rarity,
    /// v34: tier da peça (1 oficina, 2 Forja Pirata, 3 cristal). Tier alto
    /// puxa os afixos para o topo da faixa (níveis T1–T5).
    #[serde(default)]
    pub output_tier: u8,
}

/// Recurso raro que a versão Rara consome além dos insumos.
pub const RARE_CATALYST_QUANTITY: u32 = 2;

impl Recipe {
    /// A mesma receita numa raridade acima: Mágica gasta o dobro dos
    /// insumos; Rara o triplo e mais `RARE_CATALYST_QUANTITY` do
    /// `catalyst` (recurso raro). Normal devolve a receita como está.
    pub fn at_rarity(&self, rarity: Rarity, catalyst: ItemDefinitionId) -> Recipe {
        let factor = match rarity {
            Rarity::Normal => return self.clone(),
            Rarity::Magic => 2,
            Rarity::Rare => 3,
        };
        let mut ingredients: Vec<Ingredient> = self
            .ingredients
            .iter()
            .map(|ingredient| Ingredient {
                item: ingredient.item,
                quantity: ingredient.quantity * factor,
            })
            .collect();
        if rarity == Rarity::Rare {
            match ingredients.iter_mut().find(|i| i.item == catalyst) {
                Some(existing) => existing.quantity += RARE_CATALYST_QUANTITY,
                None => ingredients.push(Ingredient {
                    item: catalyst,
                    quantity: RARE_CATALYST_QUANTITY,
                }),
            }
        }
        Recipe {
            ingredients,
            output_rarity: rarity,
            ..self.clone()
        }
    }

    pub fn total_ingredient_slots(&self) -> usize {
        self.ingredients.len()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn higher_rarity_costs_more_and_rare_needs_the_catalyst() {
        let wood = ItemDefinitionId::new();
        let coral = ItemDefinitionId::new();
        let base = Recipe {
            id: RecipeId::new(),
            display_name: String::from("Canhão"),
            output_item: ItemDefinitionId::new(),
            output_quantity: 1,
            ingredients: vec![Ingredient {
                item: wood,
                quantity: 10,
            }],
            required_station: StationKind::Workbench,
            craft_time_secs: 0,
            output_rarity: Rarity::Normal,
            output_tier: 1,
        };
        assert_eq!(base.at_rarity(Rarity::Normal, coral), base);
        let magic = base.at_rarity(Rarity::Magic, coral);
        assert_eq!(magic.ingredients[0].quantity, 20);
        assert_eq!(magic.output_rarity, Rarity::Magic);
        let rare = base.at_rarity(Rarity::Rare, coral);
        assert_eq!(rare.ingredients[0].quantity, 30);
        assert_eq!(
            rare.ingredients[1],
            Ingredient {
                item: coral,
                quantity: RARE_CATALYST_QUANTITY
            }
        );
    }
}
