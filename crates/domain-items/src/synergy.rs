//! Sinergia de gemas (ligações do PoE, conjuntos do D4). Três jeitos de uma
//! gema render mais do que sozinha:
//!
//! - **Ressonância**: três gemas iguais na mesma peça — o ganho da gema
//!   entra mais uma vez.
//! - **Par ligado**: duas gemas certas na mesma peça destravam um bônus com
//!   nome (a lista é [`PAIRS`]).
//! - **Conjunto**: a mesma gema em três peças instaladas — o ganho dela
//!   entra mais uma vez no navio.
//!
//! Tudo puro: o servidor calcula, os stats somam, o client só mostra.

use serde::{Deserialize, Serialize};

use crate::affix::{Affix, AffixKind};
use crate::gem::GemKind;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum Synergy {
    Resonance(GemKind),
    /// Par na ordem de `GemKind::ALL` (o menor primeiro).
    Pair(GemKind, GemKind),
    Set(GemKind),
}

/// Par ligado: (gema, gema, nome, bônus).
pub type PairEntry = (GemKind, GemKind, &'static str, &'static [(AffixKind, i32)]);

/// Pares ligados.
pub const PAIRS: [PairEntry; 6] = [
    (
        GemKind::Ruby,
        GemKind::Sapphire,
        "Tiro Certeiro",
        &[(AffixKind::Damage, 5)],
    ),
    (
        GemKind::Ruby,
        GemKind::Emerald,
        "Salva Contínua",
        &[(AffixKind::Reload, 8)],
    ),
    (
        GemKind::Sapphire,
        GemKind::Diamond,
        "Olho de Gávea",
        &[(AffixKind::Range, 10), (AffixKind::Turn, 5)],
    ),
    (
        GemKind::Emerald,
        GemKind::Amethyst,
        "Vento de Pólvora",
        &[(AffixKind::Speed, 4), (AffixKind::Reload, 4)],
    ),
    (
        GemKind::Topaz,
        GemKind::Amethyst,
        "Porão Selado",
        &[(AffixKind::Cargo, 20)],
    ),
    (
        GemKind::Topaz,
        GemKind::Diamond,
        "Quilha Firme",
        &[(AffixKind::Hull, 25)],
    ),
];

fn pair_entry(a: GemKind, b: GemKind) -> Option<&'static PairEntry> {
    PAIRS
        .iter()
        .find(|(x, y, _, _)| (*x, *y) == (a, b) || (*x, *y) == (b, a))
}

impl Synergy {
    pub fn name(self) -> String {
        match self {
            Synergy::Resonance(gem) => format!("Ressonância de {}", gem.item_name()),
            Synergy::Pair(a, b) => pair_entry(a, b)
                .map(|(_, _, name, _)| String::from(*name))
                .unwrap_or_default(),
            Synergy::Set(gem) => format!("Conjunto de {}", gem.item_name()),
        }
    }

    /// Gemas que acendem juntas (a tela liga os encaixes delas).
    pub fn gems(self) -> Vec<GemKind> {
        match self {
            Synergy::Resonance(gem) | Synergy::Set(gem) => vec![gem],
            Synergy::Pair(a, b) => vec![a, b],
        }
    }

    pub fn bonus(self) -> Vec<Affix> {
        match self {
            Synergy::Resonance(gem) | Synergy::Set(gem) => vec![gem.effects()[0]],
            Synergy::Pair(a, b) => pair_entry(a, b)
                .map(|(_, _, _, mods)| {
                    mods.iter()
                        .map(|(kind, value)| Affix {
                            kind: *kind,
                            value: *value,
                        })
                        .collect()
                })
                .unwrap_or_default(),
        }
    }
}

/// Sinergias dentro de uma peça: ressonância e cada par ligado presente.
pub fn piece_synergies(gems: &[GemKind]) -> Vec<Synergy> {
    let mut out = Vec::new();
    for gem in GemKind::ALL {
        if gems.iter().filter(|g| **g == gem).count() >= 3 {
            out.push(Synergy::Resonance(gem));
        }
    }
    for (a, b, _, _) in PAIRS {
        if gems.contains(&a) && gems.contains(&b) {
            out.push(Synergy::Pair(a, b));
        }
    }
    out
}

/// Conjuntos do navio: a mesma gema em três peças diferentes.
pub fn set_synergies(pieces: &[&[GemKind]]) -> Vec<Synergy> {
    GemKind::ALL
        .into_iter()
        .filter(|gem| pieces.iter().filter(|gems| gems.contains(gem)).count() >= 3)
        .map(Synergy::Set)
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use GemKind::*;

    #[test]
    fn pairs_and_resonance_light_up_inside_a_piece() {
        assert!(piece_synergies(&[Ruby]).is_empty());
        assert_eq!(
            piece_synergies(&[Emerald, Ruby]),
            vec![Synergy::Pair(Ruby, Emerald)]
        );
        assert_eq!(
            piece_synergies(&[Ruby, Ruby, Ruby]),
            vec![Synergy::Resonance(Ruby)]
        );
        assert_eq!(
            piece_synergies(&[Topaz, Amethyst, Diamond]),
            vec![
                Synergy::Pair(Topaz, Amethyst),
                Synergy::Pair(Topaz, Diamond)
            ]
        );
        assert_eq!(Synergy::Pair(Emerald, Ruby).name(), "Salva Contínua");
        assert_eq!(Synergy::Resonance(Ruby).bonus(), vec![Ruby.effects()[0]]);
    }

    #[test]
    fn set_needs_the_same_gem_in_three_pieces() {
        let hull: &[GemKind] = &[Topaz];
        let sail: &[GemKind] = &[Topaz, Amethyst];
        let cannon: &[GemKind] = &[Ruby];
        assert!(set_synergies(&[hull, sail, cannon]).is_empty());
        let cannon: &[GemKind] = &[Topaz];
        assert_eq!(
            set_synergies(&[hull, sail, cannon]),
            vec![Synergy::Set(Topaz)]
        );
    }

    #[test]
    fn every_pair_has_a_name_and_a_gain() {
        for (a, b, name, mods) in PAIRS {
            assert_ne!(a, b);
            assert!(a.index() < b.index(), "{name}: menor primeiro");
            assert!(!name.is_empty());
            assert!(mods.iter().all(|(_, value)| *value > 0), "{name}");
        }
    }
}
