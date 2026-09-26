use marvyr_shared::ids::{ItemDefinitionId, ItemInstanceId};
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ItemInstance {
    pub id: ItemInstanceId,
    pub definition: ItemDefinitionId,
    pub quantity: u32,
    pub durability: Option<u16>, // presente apenas para Equipment
    /// Raridade e afixos (só equipamento fabricado Mágico/Raro).
    #[serde(default)]
    pub quality: Option<crate::affix::Quality>,
}

impl ItemInstance {
    pub fn rarity(&self) -> crate::affix::Rarity {
        self.quality
            .as_ref()
            .map_or(crate::affix::Rarity::Normal, |quality| quality.rarity)
    }

    pub fn affixes(&self) -> &[crate::affix::Affix] {
        self.quality
            .as_ref()
            .map_or(&[], |quality| quality.affixes.as_slice())
    }

    pub fn new_resource(id: ItemInstanceId, def: ItemDefinitionId, quantity: u32) -> Self {
        Self {
            id,
            definition: def,
            quantity,
            durability: None,
            quality: None,
        }
    }

    pub fn new_equipment(id: ItemInstanceId, def: ItemDefinitionId, durability: u16) -> Self {
        Self {
            id,
            definition: def,
            quantity: 1,
            durability: Some(durability),
            quality: None,
        }
    }
}
