//! Cosméticos (MV-066): velas e bandeiras. Regra do produto: coisa paga
//! nunca dá poder. Por isso um cosmético não tem campo de stat, os stats do
//! navio são calculados sem olhar para ele, e o catálogo não usa as cores
//! das facções NPC (pirata, marinha, mercador) — disfarce seria vantagem.

use thiserror::Error;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum CosmeticSlot {
    Sail,
    Flag,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Cosmetic {
    /// Id estável (banco e ferramenta de admin).
    pub id: &'static str,
    pub slot: CosmeticSlot,
    pub name: &'static str,
    /// Coluna de cor no atlas (vela ou bandeira).
    pub color: u8,
}

/// Catálogo. Só cresce NO FIM: a posição + 1 é o código de rede.
pub const COSMETICS: [Cosmetic; 5] = [
    Cosmetic {
        id: "sail-emerald",
        slot: CosmeticSlot::Sail,
        name: "Velas Esmeralda",
        color: 2,
    },
    Cosmetic {
        id: "sail-gold",
        slot: CosmeticSlot::Sail,
        name: "Velas de Ouro",
        color: 3,
    },
    Cosmetic {
        id: "sail-azure",
        slot: CosmeticSlot::Sail,
        name: "Velas Azul-Mar",
        color: 4,
    },
    Cosmetic {
        id: "flag-linen",
        slot: CosmeticSlot::Flag,
        name: "Estandarte de Linho",
        color: 0,
    },
    Cosmetic {
        id: "flag-emerald",
        slot: CosmeticSlot::Flag,
        name: "Estandarte Esmeralda",
        color: 1,
    },
];

/// Títulos (v41): conquistados jogando — Livro de Bordo e o que vier —,
/// nunca vendidos. Só aparência: nenhum stat olha para eles. Só cresce NO
/// FIM: a posição + 1 é o código de rede.
pub const TITLES: [&str; 6] = [
    "o Andarilho da Névoa",
    "a Lenda do Porto",
    "o Terror dos Mares",
    // v42: maestria de casco no máximo.
    "Mestre do Mercante",
    "Mestre da Patrulha",
    "Mestre do Corsário",
];

/// Código de rede do título (`0` = nenhum).
pub fn title_code(name: &str) -> Option<u8> {
    TITLES
        .iter()
        .position(|title| *title == name)
        .map(|index| index as u8 + 1)
}

pub fn title_by_code(code: u8) -> Option<&'static str> {
    TITLES.get(usize::from(code).checked_sub(1)?).copied()
}

/// Código de rede do cosmético (`0` = nenhum).
pub fn cosmetic_code(id: &str) -> Option<u8> {
    COSMETICS
        .iter()
        .position(|cosmetic| cosmetic.id == id)
        .map(|index| index as u8 + 1)
}

pub fn cosmetic_by_code(code: u8) -> Option<&'static Cosmetic> {
    COSMETICS.get(usize::from(code).checked_sub(1)?)
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Error)]
pub enum CosmeticError {
    #[error("cosmético desconhecido")]
    Unknown,
    #[error("este cosmético não é deste lugar")]
    WrongSlot,
    #[error("você não tem este cosmético")]
    NotOwned,
}

/// O que o capitão está usando (códigos; `0` = padrão do casco).
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct ShipCosmetics {
    pub sail: u8,
    pub flag: u8,
}

impl ShipCosmetics {
    /// Veste `code` no `slot` (`0` tira). Só o que o capitão possui.
    pub fn wear(
        &mut self,
        owned: &[u8],
        slot: CosmeticSlot,
        code: u8,
    ) -> Result<(), CosmeticError> {
        if code != 0 {
            let cosmetic = cosmetic_by_code(code).ok_or(CosmeticError::Unknown)?;
            if cosmetic.slot != slot {
                return Err(CosmeticError::WrongSlot);
            }
            if !owned.contains(&code) {
                return Err(CosmeticError::NotOwned);
            }
        }
        match slot {
            CosmeticSlot::Sail => self.sail = code,
            CosmeticSlot::Flag => self.flag = code,
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn codes_round_trip_and_zero_is_nothing() {
        for (index, cosmetic) in COSMETICS.iter().enumerate() {
            let code = cosmetic_code(cosmetic.id).unwrap();
            assert_eq!(usize::from(code), index + 1);
            assert_eq!(cosmetic_by_code(code), Some(cosmetic));
        }
        assert_eq!(cosmetic_by_code(0), None);
        assert_eq!(cosmetic_code("nada"), None);
    }

    #[test]
    fn only_owned_cosmetics_of_the_right_slot_can_be_worn() {
        let gold = cosmetic_code("sail-gold").unwrap();
        let linen = cosmetic_code("flag-linen").unwrap();
        let mut look = ShipCosmetics::default();
        assert_eq!(
            look.wear(&[], CosmeticSlot::Sail, gold),
            Err(CosmeticError::NotOwned)
        );
        assert_eq!(
            look.wear(&[linen], CosmeticSlot::Sail, linen),
            Err(CosmeticError::WrongSlot)
        );
        assert_eq!(
            look.wear(&[gold], CosmeticSlot::Sail, 99),
            Err(CosmeticError::Unknown)
        );
        look.wear(&[gold], CosmeticSlot::Sail, gold).unwrap();
        assert_eq!(look.sail, gold);
        look.wear(&[], CosmeticSlot::Sail, 0).unwrap();
        assert_eq!(look, ShipCosmetics::default());
    }

    #[test]
    fn every_hull_master_title_is_in_the_catalog() {
        for kind in crate::ShipKind::ALL {
            let code = title_code(kind.master_title()).expect("título no catálogo");
            assert_eq!(title_by_code(code), Some(kind.master_title()));
        }
        assert_eq!(title_by_code(0), None);
    }

    #[test]
    fn catalog_stays_off_npc_faction_colors() {
        // Velas: vermelho (pirata), branco (marinha) e creme (mercador).
        // Bandeiras: azul (marinha), vermelha (pirata), branca (mercador) e
        // dourada (o próprio navio).
        for cosmetic in COSMETICS {
            let banned: &[u8] = match cosmetic.slot {
                CosmeticSlot::Sail => &[0, 1, 5],
                CosmeticSlot::Flag => &[2, 3, 4, 5],
            };
            assert!(!banned.contains(&cosmetic.color), "{}", cosmetic.id);
        }
    }
}
