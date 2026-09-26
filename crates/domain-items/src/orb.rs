//! Orbes de ofício (a "moeda" do PoE, aqui item de escambo): fabricados na
//! oficina com recurso bruto e gastos numa peça do armazém para mexer na
//! raridade e nos afixos, ou num mapa para mexer nos perigos. NPC nunca dá
//! orbe — só a oficina de jogador faz.

use marvyr_shared::ids::ItemDefinitionId;
use serde::{Deserialize, Serialize};

use crate::affix::{Affix, AffixKind, Quality, Rarity};
use crate::map_mod::MapMod;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum OrbKind {
    /// Normal → Mágico.
    Transmutation,
    /// Sorteia de novo os afixos (mesma raridade).
    Chaos,
    /// Mágico → Raro, guardando os afixos que já tinha.
    Regal,
    /// Mais um afixo (Mágico até 2, Raro até 5).
    Exalted,
    /// Mapa do Tesouro: sorteia de novo os perigos (Normal vira Mágico).
    Cartographer,
}

/// Teto de afixos por raridade depois de exaltar.
pub const MAGIC_MAX_AFFIXES: usize = 2;
pub const RARE_MAX_AFFIXES: usize = 5;

#[derive(Debug, Clone, Copy, PartialEq, Eq, thiserror::Error)]
pub enum OrbError {
    #[error("esse orbe é para equipamento")]
    NeedsEquipment,
    #[error("esse orbe é para Mapa do Tesouro")]
    NeedsMap,
    #[error("a peça precisa ser Normal")]
    NeedsNormal,
    #[error("a peça precisa ser Mágica")]
    NeedsMagic,
    #[error("peça Normal não tem afixo para mexer")]
    NeedsAffixes,
    #[error("a peça já está no máximo de afixos")]
    Full,
}

impl OrbKind {
    pub const ALL: [OrbKind; 5] = [
        OrbKind::Transmutation,
        OrbKind::Chaos,
        OrbKind::Regal,
        OrbKind::Exalted,
        OrbKind::Cartographer,
    ];

    pub fn item_name(self) -> &'static str {
        match self {
            OrbKind::Transmutation => "Orbe de Transmutação",
            OrbKind::Chaos => "Orbe do Caos",
            OrbKind::Regal => "Orbe Régio",
            OrbKind::Exalted => "Orbe Exaltado",
            OrbKind::Cartographer => "Orbe do Cartógrafo",
        }
    }

    pub fn item_id(self) -> ItemDefinitionId {
        ItemDefinitionId::stable(self.item_name())
    }

    pub fn from_item(item: ItemDefinitionId) -> Option<OrbKind> {
        OrbKind::ALL.into_iter().find(|orb| orb.item_id() == item)
    }

    pub fn index(self) -> usize {
        OrbKind::ALL
            .iter()
            .position(|orb| *orb == self)
            .unwrap_or(0)
    }

    /// Gasta o orbe na peça. `is_equipment`/`is_map` vêm do catálogo;
    /// `seed` é a sorte do servidor. Falhou, a peça não muda.
    pub fn apply(
        self,
        quality: &mut Option<Quality>,
        is_equipment: bool,
        is_map: bool,
        seed: u64,
    ) -> Result<(), OrbError> {
        let rarity = quality.as_ref().map_or(Rarity::Normal, |q| q.rarity);
        let gems = quality.as_ref().map(|q| q.gems.clone()).unwrap_or_default();
        let mut state = seed;
        match self {
            OrbKind::Cartographer => {
                if !is_map {
                    return Err(OrbError::NeedsMap);
                }
                let rarity = match rarity {
                    Rarity::Normal => Rarity::Magic,
                    other => other,
                };
                *quality = Some(crate::map_mod::roll_map_at(rarity, seed));
                return Ok(());
            }
            _ if !is_equipment => return Err(OrbError::NeedsEquipment),
            OrbKind::Transmutation => {
                if rarity != Rarity::Normal {
                    return Err(OrbError::NeedsNormal);
                }
                *quality = with_gems(crate::affix::roll_quality(Rarity::Magic, seed), gems);
            }
            OrbKind::Chaos => {
                if rarity == Rarity::Normal {
                    return Err(OrbError::NeedsAffixes);
                }
                *quality = with_gems(crate::affix::roll_quality(rarity, seed), gems);
            }
            OrbKind::Regal => {
                if rarity != Rarity::Magic {
                    return Err(OrbError::NeedsMagic);
                }
                let q = quality.as_mut().expect("Mágica tem Quality");
                q.rarity = Rarity::Rare;
                let target = 3 + (splitmix64(&mut state) % 2) as usize;
                while q.affixes.len() < target {
                    let Some(affix) = roll_new_affix(&q.affixes, &mut state) else {
                        break;
                    };
                    q.affixes.push(affix);
                }
            }
            OrbKind::Exalted => {
                let cap = match rarity {
                    Rarity::Normal => return Err(OrbError::NeedsAffixes),
                    Rarity::Magic => MAGIC_MAX_AFFIXES,
                    Rarity::Rare => RARE_MAX_AFFIXES,
                };
                let q = quality.as_mut().expect("Mágica/Rara tem Quality");
                if q.affixes.len() >= cap {
                    return Err(OrbError::Full);
                }
                let affix = roll_new_affix(&q.affixes, &mut state).ok_or(OrbError::Full)?;
                q.affixes.push(affix);
            }
        }
        Ok(())
    }
}

fn with_gems(quality: Option<Quality>, gems: Vec<crate::gem::GemKind>) -> Option<Quality> {
    quality.map(|q| Quality { gems, ..q })
}

fn splitmix64(state: &mut u64) -> u64 {
    *state = state.wrapping_add(0x9E37_79B9_7F4A_7C15);
    let mut z = *state;
    z = (z ^ (z >> 30)).wrapping_mul(0xBF58_476D_1CE4_E5B9);
    z = (z ^ (z >> 27)).wrapping_mul(0x94D0_49BB_1331_11EB);
    z ^ (z >> 31)
}

/// Afixo de um tipo que a peça ainda não tem.
fn roll_new_affix(existing: &[Affix], state: &mut u64) -> Option<Affix> {
    let pool: Vec<AffixKind> = AffixKind::ALL
        .into_iter()
        .filter(|kind| existing.iter().all(|a| a.kind != *kind))
        .collect();
    if pool.is_empty() {
        return None;
    }
    let kind = pool[(splitmix64(state) % pool.len() as u64) as usize];
    let (low, high) = kind.roll_range();
    let span = (high - low + 1) as u64;
    Some(Affix {
        kind,
        value: low + (splitmix64(state) % span) as i32,
    })
}

/// Perigos do mapa que o orbe deixou (para a festa).
pub fn map_mods_of(quality: &Option<Quality>) -> &[MapMod] {
    quality.as_ref().map_or(&[], |q| q.map_mods.as_slice())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::gem::GemKind;

    #[test]
    fn transmute_regal_exalt_climb_the_ladder_and_keep_gems() {
        let mut quality = None;
        crate::gem::socket(&mut quality, GemKind::Ruby).unwrap();
        OrbKind::Transmutation
            .apply(&mut quality, true, false, 1)
            .unwrap();
        let q = quality.clone().unwrap();
        assert_eq!(q.rarity, Rarity::Magic);
        assert!((1..=2).contains(&q.affixes.len()));
        assert_eq!(q.gems, vec![GemKind::Ruby], "a gema segue encaixada");
        assert_eq!(
            OrbKind::Transmutation.apply(&mut quality, true, false, 2),
            Err(OrbError::NeedsNormal)
        );

        OrbKind::Regal.apply(&mut quality, true, false, 3).unwrap();
        let rare = quality.clone().unwrap();
        assert_eq!(rare.rarity, Rarity::Rare);
        assert!(rare.affixes.starts_with(&q.affixes), "guarda os afixos");
        assert!((3..=4).contains(&rare.affixes.len()));

        for seed in 10..20 {
            let _ = OrbKind::Exalted.apply(&mut quality, true, false, seed);
        }
        let full = quality.clone().unwrap();
        assert_eq!(full.affixes.len(), RARE_MAX_AFFIXES);
        let kinds: std::collections::HashSet<_> = full.affixes.iter().map(|a| a.kind).collect();
        assert_eq!(kinds.len(), full.affixes.len(), "sem tipo repetido");
        assert_eq!(
            OrbKind::Exalted.apply(&mut quality, true, false, 99),
            Err(OrbError::Full)
        );
    }

    #[test]
    fn chaos_rerolls_but_keeps_rarity_and_orbs_respect_their_target() {
        let mut quality = crate::affix::roll_quality(Rarity::Rare, 5);
        let before = quality.clone();
        OrbKind::Chaos.apply(&mut quality, true, false, 77).unwrap();
        assert_eq!(quality.as_ref().unwrap().rarity, Rarity::Rare);
        assert_ne!(quality, before);

        let mut normal = None;
        assert_eq!(
            OrbKind::Chaos.apply(&mut normal, true, false, 1),
            Err(OrbError::NeedsAffixes)
        );
        assert_eq!(
            OrbKind::Chaos.apply(&mut normal, false, true, 1),
            Err(OrbError::NeedsEquipment)
        );
        assert_eq!(
            OrbKind::Cartographer.apply(&mut normal, true, false, 1),
            Err(OrbError::NeedsMap)
        );
        assert_eq!(normal, None, "falhou, não mexe");
    }

    #[test]
    fn cartographer_wakes_a_normal_map_and_rerolls_a_rare_one() {
        let mut map = None;
        OrbKind::Cartographer
            .apply(&mut map, false, true, 4)
            .unwrap();
        let q = map.clone().unwrap();
        assert_eq!(q.rarity, Rarity::Magic);
        assert!((1..=2).contains(&q.map_mods.len()));

        let mut rare = Some(crate::map_mod::roll_map_at(Rarity::Rare, 1));
        OrbKind::Cartographer
            .apply(&mut rare, false, true, 2)
            .unwrap();
        assert_eq!(rare.as_ref().unwrap().rarity, Rarity::Rare);
        assert!((3..=4).contains(&map_mods_of(&rare).len()));
    }
}
