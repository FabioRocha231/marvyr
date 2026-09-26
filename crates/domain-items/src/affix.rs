//! Afixos de equipamento (estilo PoE): peça fabricada sai Normal, Mágica
//! (1-2 afixos) ou Rara (3-4). Só a fabricação de jogador cria afixo — NPC
//! nunca dá item útil. O sorteio é determinístico pela semente (a id da
//! peça): mesma peça, mesmos afixos, em qualquer máquina.

use serde::{Deserialize, Serialize};

#[derive(
    Debug, Clone, Copy, Default, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize,
)]
pub enum Rarity {
    #[default]
    Normal,
    Magic,
    Rare,
}

impl Rarity {
    pub const ALL: [Rarity; 3] = [Rarity::Normal, Rarity::Magic, Rarity::Rare];
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum AffixKind {
    /// + pontos de dano de arma.
    Damage,
    /// + % de alcance.
    Range,
    /// + pontos de casco.
    Hull,
    /// + % de velocidade.
    Speed,
    /// + pontos de porão.
    Cargo,
    /// + % de giro do leme.
    Turn,
    /// - % do tempo de recarga.
    Reload,
}

impl AffixKind {
    pub const ALL: [AffixKind; 7] = [
        AffixKind::Damage,
        AffixKind::Range,
        AffixKind::Hull,
        AffixKind::Speed,
        AffixKind::Cargo,
        AffixKind::Turn,
        AffixKind::Reload,
    ];

    /// Faixa sorteável (inclusiva). Balanceamento: um Raro bom rende perto
    /// de um tier de equipamento, nunca dois.
    pub fn roll_range(self) -> (i32, i32) {
        match self {
            AffixKind::Damage => (2, 6),
            AffixKind::Range => (3, 10),
            AffixKind::Hull => (8, 25),
            AffixKind::Speed => (2, 6),
            AffixKind::Cargo => (5, 15),
            AffixKind::Turn => (3, 10),
            AffixKind::Reload => (3, 10),
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct Affix {
    pub kind: AffixKind,
    pub value: i32,
}

/// Raridade e afixos de uma peça. Normal não carrega `Quality`.
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct Quality {
    pub rarity: Rarity,
    pub affixes: Vec<Affix>,
    /// Gemas de suporte encaixadas (v24); vazio em peça sem gema.
    #[serde(default)]
    pub gems: Vec<crate::gem::GemKind>,
}

/// Soma dos afixos equipados, já no formato que os stats consomem.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct AffixTotals {
    pub damage: i32,
    pub range_pct: i32,
    pub hull: i32,
    pub speed_pct: i32,
    pub cargo: i32,
    pub turn_pct: i32,
    pub reload_pct: i32,
}

impl AffixTotals {
    pub fn add(&mut self, affix: Affix) {
        let slot = match affix.kind {
            AffixKind::Damage => &mut self.damage,
            AffixKind::Range => &mut self.range_pct,
            AffixKind::Hull => &mut self.hull,
            AffixKind::Speed => &mut self.speed_pct,
            AffixKind::Cargo => &mut self.cargo,
            AffixKind::Turn => &mut self.turn_pct,
            AffixKind::Reload => &mut self.reload_pct,
        };
        *slot += affix.value;
    }
}

/// Nível da aura de poder (0-3) pela raridade do que está instalado:
/// Mágico conta 1, Raro 2. Equipamento bom brilha — e vira alvo de longe.
pub fn aura_level(rarities: impl IntoIterator<Item = Rarity>) -> u8 {
    let score: u32 = rarities
        .into_iter()
        .map(|rarity| match rarity {
            Rarity::Normal => 0,
            Rarity::Magic => 1,
            Rarity::Rare => 2,
        })
        .sum();
    match score {
        0 => 0,
        1..=2 => 1,
        3..=4 => 2,
        _ => 3,
    }
}

fn splitmix64(state: &mut u64) -> u64 {
    *state = state.wrapping_add(0x9E37_79B9_7F4A_7C15);
    let mut z = *state;
    z = (z ^ (z >> 30)).wrapping_mul(0xBF58_476D_1CE4_E5B9);
    z = (z ^ (z >> 27)).wrapping_mul(0x94D0_49BB_1331_11EB);
    z ^ (z >> 31)
}

/// Sorteia os afixos de uma peça da raridade dada; Normal = `None`. Tipos
/// não se repetem na mesma peça.
pub fn roll_quality(rarity: Rarity, seed: u64) -> Option<Quality> {
    let mut state = seed;
    let count = match rarity {
        Rarity::Normal => return None,
        Rarity::Magic => 1 + (splitmix64(&mut state) % 2) as usize,
        Rarity::Rare => 3 + (splitmix64(&mut state) % 2) as usize,
    };
    let mut pool = AffixKind::ALL.to_vec();
    let mut affixes = Vec::with_capacity(count);
    for _ in 0..count {
        let kind = pool.remove((splitmix64(&mut state) % pool.len() as u64) as usize);
        let (low, high) = kind.roll_range();
        let span = (high - low + 1) as u64;
        affixes.push(Affix {
            kind,
            value: low + (splitmix64(&mut state) % span) as i32,
        });
    }
    Some(Quality {
        rarity,
        affixes,
        gems: Vec::new(),
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rarity_sets_the_affix_count_and_values_stay_in_range() {
        assert_eq!(roll_quality(Rarity::Normal, 7), None);
        for seed in 0..500u64 {
            let magic = roll_quality(Rarity::Magic, seed).unwrap();
            assert!((1..=2).contains(&magic.affixes.len()));
            let rare = roll_quality(Rarity::Rare, seed).unwrap();
            assert!((3..=4).contains(&rare.affixes.len()));
            for affix in magic.affixes.iter().chain(&rare.affixes) {
                let (low, high) = affix.kind.roll_range();
                assert!((low..=high).contains(&affix.value), "{affix:?}");
            }
            let mut kinds: Vec<_> = rare.affixes.iter().map(|a| a.kind).collect();
            kinds.dedup();
            let unique: std::collections::HashSet<_> = kinds.iter().collect();
            assert_eq!(unique.len(), rare.affixes.len(), "sem tipo repetido");
        }
    }

    #[test]
    fn aura_grows_with_the_rarity_installed() {
        use Rarity::*;
        assert_eq!(aura_level([Normal, Normal]), 0);
        assert_eq!(aura_level([Magic]), 1);
        assert_eq!(aura_level([Rare, Magic]), 2);
        assert_eq!(aura_level([Rare, Rare, Rare]), 3);
    }

    #[test]
    fn same_seed_same_piece() {
        assert_eq!(
            roll_quality(Rarity::Rare, 42),
            roll_quality(Rarity::Rare, 42)
        );
        assert_ne!(
            roll_quality(Rarity::Rare, 42),
            roll_quality(Rarity::Rare, 43)
        );
    }

    #[test]
    fn totals_sum_by_kind() {
        let mut totals = AffixTotals::default();
        totals.add(Affix {
            kind: AffixKind::Damage,
            value: 3,
        });
        totals.add(Affix {
            kind: AffixKind::Damage,
            value: 2,
        });
        totals.add(Affix {
            kind: AffixKind::Reload,
            value: 5,
        });
        assert_eq!(totals.damage, 5);
        assert_eq!(totals.reload_pct, 5);
    }
}
