//! Piratas de elite (monstros raros do PoE, elites do D4): parte dos
//! piratas nasce com 1 ou 2 afixos que mudam o combate. Vai pelo fio como
//! bitmask (`ShipState.elite`); o servidor aplica, o client só nomeia.
//! Butim continua só recurso bruto (Pilar 1), só que em dobro.

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum EliteAffix {
    /// Casco ×1,8.
    Armored,
    /// Pano e leme ×1,3.
    Swift,
    /// Dano ×1,4.
    Incendiary,
    /// Recarga ×0,6.
    DoubleSalvo,
    /// Recupera 2% do casco por segundo.
    Regenerating,
}

pub const ARMORED_HP: f32 = 1.8;
pub const SWIFT: f32 = 1.3;
pub const INCENDIARY_DAMAGE: f32 = 1.4;
pub const DOUBLE_SALVO_RELOAD: f32 = 0.6;
pub const REGEN_PCT_PER_SEC: f32 = 0.02;
/// Elite rende o dobro de recurso bruto.
pub const SPOILS_FACTOR: u32 = 2;

impl EliteAffix {
    pub const ALL: [EliteAffix; 5] = [
        EliteAffix::Armored,
        EliteAffix::Swift,
        EliteAffix::Incendiary,
        EliteAffix::DoubleSalvo,
        EliteAffix::Regenerating,
    ];

    pub fn bit(self) -> u8 {
        1 << EliteAffix::ALL.iter().position(|a| *a == self).unwrap_or(0)
    }

    pub fn name(self) -> &'static str {
        match self {
            EliteAffix::Armored => "Blindado",
            EliteAffix::Swift => "Veloz",
            EliteAffix::Incendiary => "Incendiário",
            EliteAffix::DoubleSalvo => "Salva Dupla",
            EliteAffix::Regenerating => "Regenerante",
        }
    }
}

/// Afixos presentes na máscara, na ordem de `ALL`.
pub fn affixes(mask: u8) -> Vec<EliteAffix> {
    EliteAffix::ALL
        .into_iter()
        .filter(|a| mask & a.bit() != 0)
        .collect()
}

pub fn has(mask: u8, affix: EliteAffix) -> bool {
    mask & affix.bit() != 0
}

fn splitmix64(state: &mut u64) -> u64 {
    *state = state.wrapping_add(0x9E37_79B9_7F4A_7C15);
    let mut z = *state;
    z = (z ^ (z >> 30)).wrapping_mul(0xBF58_476D_1CE4_E5B9);
    z = (z ^ (z >> 27)).wrapping_mul(0x94D0_49BB_1331_11EB);
    z ^ (z >> 31)
}

/// Sorteia a elite: `chance_pct` de sair elite; elite tem 1 afixo (70%) ou
/// 2 (30%), sem repetir. 0 = pirata comum.
pub fn roll(seed: u64, chance_pct: u32) -> u8 {
    let mut state = seed;
    if (splitmix64(&mut state) % 100) as u32 >= chance_pct {
        return 0;
    }
    let count = if splitmix64(&mut state) % 100 < 70 {
        1
    } else {
        2
    };
    let mut pool = EliteAffix::ALL.to_vec();
    let mut mask = 0;
    for _ in 0..count {
        let pick = pool.remove((splitmix64(&mut state) % pool.len() as u64) as usize);
        mask |= pick.bit();
    }
    mask
}

/// Nome do elite para a placa: "Pirata Blindado Veloz".
pub fn title(base: &str, mask: u8) -> String {
    affixes(mask)
        .iter()
        .fold(String::from(base), |acc, a| format!("{acc} {}", a.name()))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn roll_respects_the_chance_and_the_affix_count() {
        let mut elites = 0;
        for seed in 0..2_000u64 {
            let mask = roll(seed, 30);
            if mask != 0 {
                elites += 1;
                assert!((1..=2).contains(&affixes(mask).len()), "{mask:b}");
            }
        }
        assert!((450..750).contains(&elites), "{elites}");
        assert!((0..500u64).all(|seed| roll(seed, 0) == 0));
        assert_eq!(roll(7, 30), roll(7, 30));
    }

    #[test]
    fn mask_roundtrips_names() {
        let mask = EliteAffix::Armored.bit() | EliteAffix::Swift.bit();
        assert_eq!(affixes(mask), vec![EliteAffix::Armored, EliteAffix::Swift]);
        assert!(has(mask, EliteAffix::Swift));
        assert!(!has(mask, EliteAffix::Regenerating));
        assert_eq!(title("Pirata", mask), "Pirata Blindado Veloz");
    }
}
