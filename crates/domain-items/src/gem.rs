//! Gemas de suporte (estilo PoE): encaixadas numa peça equipada, trocam um
//! ganho por um custo ("mais dano, recarga mais lenta"). Gema é recurso
//! fabricado por jogador — NPC nunca solta gema. Encaixada, ela vive dentro
//! da peça (`Quality::gems`): sai do armazém e passa a morar num lugar só.

use marvyr_shared::ids::ItemDefinitionId;
use serde::{Deserialize, Serialize};

use crate::affix::{Affix, AffixKind, Quality, Rarity};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum GemKind {
    Ruby,
    Sapphire,
    Emerald,
    Topaz,
    Amethyst,
    Diamond,
}

impl GemKind {
    /// Mesma ordem da linha de gemas do atlas de ícones.
    pub const ALL: [GemKind; 6] = [
        GemKind::Ruby,
        GemKind::Sapphire,
        GemKind::Emerald,
        GemKind::Topaz,
        GemKind::Amethyst,
        GemKind::Diamond,
    ];

    /// Nome do item no catálogo (a gema solta no armazém).
    pub fn item_name(self) -> &'static str {
        match self {
            GemKind::Ruby => "Rubi",
            GemKind::Sapphire => "Safira",
            GemKind::Emerald => "Esmeralda",
            GemKind::Topaz => "Topázio",
            GemKind::Amethyst => "Ametista",
            GemKind::Diamond => "Diamante",
        }
    }

    /// O que ela faz, em uma palavra de capitão.
    pub fn support_name(self) -> &'static str {
        match self {
            GemKind::Ruby => "Pólvora Negra",
            GemKind::Sapphire => "Mira Longa",
            GemKind::Emerald => "Carga Rápida",
            GemKind::Topaz => "Casco Selado",
            GemKind::Amethyst => "Vento Preso",
            GemKind::Diamond => "Leme Fino",
        }
    }

    pub fn item_id(self) -> ItemDefinitionId {
        ItemDefinitionId::stable(self.item_name())
    }

    pub fn from_item(item: ItemDefinitionId) -> Option<GemKind> {
        GemKind::ALL.into_iter().find(|gem| gem.item_id() == item)
    }

    pub fn from_name(name: &str) -> Option<GemKind> {
        GemKind::ALL.into_iter().find(|gem| gem.item_name() == name)
    }

    pub fn index(self) -> usize {
        GemKind::ALL
            .iter()
            .position(|gem| *gem == self)
            .unwrap_or(0)
    }

    /// Ganho e custo. Valores com sinal no formato dos afixos: recarga
    /// negativa = canhão mais lento.
    pub fn effects(self) -> [Affix; 2] {
        let affix = |kind, value| Affix { kind, value };
        match self {
            GemKind::Ruby => [affix(AffixKind::Damage, 8), affix(AffixKind::Reload, -10)],
            GemKind::Sapphire => [affix(AffixKind::Range, 15), affix(AffixKind::Damage, -3)],
            GemKind::Emerald => [affix(AffixKind::Reload, 12), affix(AffixKind::Range, -8)],
            GemKind::Topaz => [affix(AffixKind::Hull, 30), affix(AffixKind::Speed, -5)],
            GemKind::Amethyst => [affix(AffixKind::Speed, 8), affix(AffixKind::Cargo, -15)],
            GemKind::Diamond => [affix(AffixKind::Turn, 15), affix(AffixKind::Hull, -10)],
        }
    }
}

/// Encaixes por raridade: fabricar melhor rende mais gemas.
pub fn socket_count(rarity: Rarity) -> usize {
    match rarity {
        Rarity::Normal => 1,
        Rarity::Magic => 2,
        Rarity::Rare => 3,
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, thiserror::Error)]
pub enum SocketError {
    #[error("sem encaixe livre nesta peça")]
    Full,
    #[error("não há gema nesse encaixe")]
    Empty,
}

/// Encaixa na primeira vaga. Peça Normal ganha uma `Quality` só para
/// guardar a gema.
pub fn socket(quality: &mut Option<Quality>, gem: GemKind) -> Result<(), SocketError> {
    let rarity = quality.as_ref().map_or(Rarity::Normal, |q| q.rarity);
    let current = quality.as_ref().map_or(0, |q| q.gems.len());
    if current >= socket_count(rarity) {
        return Err(SocketError::Full);
    }
    quality
        .get_or_insert_with(|| Quality {
            rarity: Rarity::Normal,
            affixes: Vec::new(),
            gems: Vec::new(),
            map_mods: Vec::new(),
            aspect: None,
        })
        .gems
        .push(gem);
    Ok(())
}

/// Tira a gema do encaixe `index`. Peça Normal vazia volta a `None`.
pub fn unsocket(quality: &mut Option<Quality>, index: usize) -> Result<GemKind, SocketError> {
    let inner = quality.as_mut().ok_or(SocketError::Empty)?;
    if index >= inner.gems.len() {
        return Err(SocketError::Empty);
    }
    let gem = inner.gems.remove(index);
    if inner.rarity == Rarity::Normal && inner.gems.is_empty() {
        *quality = None;
    }
    Ok(gem)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn sockets_follow_rarity_and_normal_goes_back_to_none() {
        let mut normal = None;
        socket(&mut normal, GemKind::Ruby).unwrap();
        assert_eq!(socket(&mut normal, GemKind::Topaz), Err(SocketError::Full));
        assert_eq!(unsocket(&mut normal, 0), Ok(GemKind::Ruby));
        assert_eq!(normal, None, "Normal sem gema não guarda Quality");

        let mut rare = crate::roll_quality(Rarity::Rare, 5);
        let affixes = rare.as_ref().unwrap().affixes.clone();
        for gem in [GemKind::Ruby, GemKind::Sapphire, GemKind::Diamond] {
            socket(&mut rare, gem).unwrap();
        }
        assert_eq!(socket(&mut rare, GemKind::Emerald), Err(SocketError::Full));
        assert_eq!(unsocket(&mut rare, 1), Ok(GemKind::Sapphire));
        assert_eq!(unsocket(&mut rare, 5), Err(SocketError::Empty));
        let rare = rare.unwrap();
        assert_eq!(rare.gems, vec![GemKind::Ruby, GemKind::Diamond]);
        assert_eq!(rare.affixes, affixes, "gema não mexe nos afixos");
    }

    #[test]
    fn every_gem_trades_a_gain_for_a_cost() {
        for gem in GemKind::ALL {
            let [gain, cost] = gem.effects();
            assert!(gain.value > 0 && cost.value < 0, "{gem:?}");
            assert_ne!(gain.kind, cost.kind);
            assert_eq!(GemKind::from_item(gem.item_id()), Some(gem));
            assert_eq!(GemKind::from_name(gem.item_name()), Some(gem));
        }
    }
}
