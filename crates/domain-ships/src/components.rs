use marvyr_shared::ids::ItemDefinitionId;
use serde::{Deserialize, Serialize};

use marvyr_domain_items::EquipmentSlot;

/// Componente equipado em um slot. Aponta para a definição rica no catálogo
/// de itens (`domain-items`), fonte única dos modificadores de stats.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct EquippedComponent {
    pub slot: EquipmentSlot,
    pub item_definition: ItemDefinitionId,
    /// Afixos da peça instalada (Mágica/Rara); vazio na Normal.
    #[serde(default)]
    pub affixes: Vec<marvyr_domain_items::Affix>,
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct EquippedComponents {
    pub hull: Vec<EquippedComponent>,
    pub sail: Vec<EquippedComponent>,
    pub weapon: Vec<EquippedComponent>,
    pub aux: Vec<EquippedComponent>,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn default_is_empty() {
        let equipped = EquippedComponents::default();
        assert!(equipped.hull.is_empty());
        assert!(equipped.sail.is_empty());
        assert!(equipped.weapon.is_empty());
        assert!(equipped.aux.is_empty());
    }
}
