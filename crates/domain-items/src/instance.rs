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

    /// Afixos + efeitos das gemas encaixadas: o que os stats somam.
    pub fn stat_mods(&self) -> Vec<crate::affix::Affix> {
        let mut mods = self.affixes().to_vec();
        if let Some(quality) = &self.quality {
            mods.extend(quality.gems.iter().flat_map(|gem| gem.effects()));
            // Gemas ligadas na mesma peça rendem mais (ressonância, pares).
            mods.extend(
                crate::synergy::piece_synergies(&quality.gems)
                    .into_iter()
                    .flat_map(crate::synergy::Synergy::bonus),
            );
        }
        mods
    }

    pub fn map_mods(&self) -> &[crate::map_mod::MapMod] {
        self.quality
            .as_ref()
            .map_or(&[], |quality| quality.map_mods.as_slice())
    }

    pub fn gems(&self) -> &[crate::gem::GemKind] {
        self.quality
            .as_ref()
            .map_or(&[], |quality| quality.gems.as_slice())
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
