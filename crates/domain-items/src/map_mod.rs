//! Modificadores de Mapa do Tesouro (estilo mapas do PoE): o mapa sai
//! Normal, Mágico (1-2) ou Raro (3-4). Cada modificador deixa a escavação
//! mais perigosa e o baú mais gordo. Sorteado quando o mapa é achado, com a
//! id do mapa como semente: mesmo mapa, mesmos perigos.

use serde::{Deserialize, Serialize};

use crate::affix::{Quality, Rarity};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum MapMod {
    /// Dois corsários guardam a ilha.
    Guarded,
    /// O Kraken dorme sob a praia.
    Kraken,
    /// Uma tormenta cai sobre a ilha.
    Tempest,
    /// Rocha dura: o escaler cava o dobro do tempo.
    Bedrock,
    /// Boca solta: o mar inteiro fica sabendo onde você cava.
    Rumored,
}

impl MapMod {
    pub const ALL: [MapMod; 5] = [
        MapMod::Guarded,
        MapMod::Kraken,
        MapMod::Tempest,
        MapMod::Bedrock,
        MapMod::Rumored,
    ];

    pub fn label(self) -> &'static str {
        match self {
            MapMod::Guarded => "Guardado",
            MapMod::Kraken => "Covil do Kraken",
            MapMod::Tempest => "Tormenta",
            MapMod::Bedrock => "Rocha Dura",
            MapMod::Rumored => "Boca Solta",
        }
    }

    pub fn description(self) -> &'static str {
        match self {
            MapMod::Guarded => "dois corsários guardam a ilha",
            MapMod::Kraken => "o Kraken emerge quando o escaler desce",
            MapMod::Tempest => "uma tormenta cai sobre a ilha",
            MapMod::Bedrock => "cavar leva o dobro do tempo",
            MapMod::Rumored => "o mar inteiro sabe onde você cava",
        }
    }

    /// Quanto o baú engorda (% sobre o tesouro base).
    pub fn reward_pct(self) -> u32 {
        match self {
            MapMod::Guarded => 40,
            MapMod::Kraken => 80,
            MapMod::Tempest => 30,
            MapMod::Bedrock => 25,
            MapMod::Rumored => 35,
        }
    }
}

/// Bônus total do baú (% somado dos modificadores).
pub fn treasure_bonus_pct(mods: &[MapMod]) -> u32 {
    mods.iter().map(|m| m.reward_pct()).sum()
}

/// Quantidade do tesouro com o bônus (arredonda para baixo; nunca menos
/// que a base).
pub fn scaled_treasure(base: u32, mods: &[MapMod]) -> u32 {
    scaled_by(base, treasure_bonus_pct(mods))
}

/// `base` com `bonus_pct` a mais.
pub fn scaled_by(base: u32, bonus_pct: u32) -> u32 {
    base * (100 + bonus_pct) / 100
}

/// Segundos de escavação com a Rocha Dura.
pub fn dig_secs(base: f32, mods: &[MapMod]) -> f32 {
    if mods.contains(&MapMod::Bedrock) {
        base * 2.0
    } else {
        base
    }
}

fn splitmix64(state: &mut u64) -> u64 {
    *state = state.wrapping_add(0x9E37_79B9_7F4A_7C15);
    let mut z = *state;
    z = (z ^ (z >> 30)).wrapping_mul(0xBF58_476D_1CE4_E5B9);
    z = (z ^ (z >> 27)).wrapping_mul(0x94D0_49BB_1331_11EB);
    z ^ (z >> 31)
}

/// Mapa achado: 60% Normal, 30% Mágico, 10% Raro, modificadores sem
/// repetir. Normal = `None` (o mapa de sempre).
pub fn roll_map(seed: u64) -> Option<Quality> {
    let mut state = seed;
    let rarity = match splitmix64(&mut state) % 100 {
        0..=59 => return None,
        60..=89 => Rarity::Magic,
        _ => Rarity::Rare,
    };
    Some(roll_map_at(rarity, splitmix64(&mut state)))
}

/// Perigos de um mapa da raridade dada (Mágico 1-2, Raro 3-4; Normal
/// sai sem perigo). O Orbe do Cartógrafo usa isto.
pub fn roll_map_at(rarity: Rarity, seed: u64) -> Quality {
    let mut state = seed;
    let count = match rarity {
        Rarity::Normal => 0,
        Rarity::Magic => 1 + (splitmix64(&mut state) % 2) as usize,
        Rarity::Rare => 3 + (splitmix64(&mut state) % 2) as usize,
    };
    let mut pool = MapMod::ALL.to_vec();
    let mut map_mods = Vec::with_capacity(count);
    for _ in 0..count {
        map_mods.push(pool.remove((splitmix64(&mut state) % pool.len() as u64) as usize));
    }
    Quality {
        rarity,
        affixes: Vec::new(),
        gems: Vec::new(),
        map_mods,
        aspect: None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rarity_sets_the_mod_count_and_the_odds_hold() {
        let mut tally = [0u32; 3];
        for seed in 0..2_000u64 {
            match roll_map(seed) {
                None => tally[0] += 1,
                Some(q) => {
                    let (low, high, slot) = match q.rarity {
                        Rarity::Magic => (1, 2, 1),
                        Rarity::Rare => (3, 4, 2),
                        Rarity::Normal => unreachable!("Normal não carrega Quality"),
                    };
                    assert!((low..=high).contains(&q.map_mods.len()));
                    let unique: std::collections::HashSet<_> = q.map_mods.iter().collect();
                    assert_eq!(unique.len(), q.map_mods.len(), "sem repetir");
                    assert!(q.affixes.is_empty() && q.gems.is_empty());
                    tally[slot] += 1;
                }
            }
        }
        assert!((1_050..1_350).contains(&tally[0]), "{tally:?}");
        assert!((450..750).contains(&tally[1]), "{tally:?}");
        assert!((120..290).contains(&tally[2]), "{tally:?}");
        assert_eq!(roll_map(42), roll_map(42));
    }

    #[test]
    fn danger_pays_and_bedrock_doubles_the_dig() {
        use MapMod::*;
        assert_eq!(scaled_treasure(6, &[]), 6);
        assert_eq!(scaled_treasure(6, &[Kraken]), 10);
        assert_eq!(scaled_treasure(3, &[Guarded, Kraken, Tempest, Rumored]), 8);
        assert_eq!(dig_secs(8.0, &[Guarded]), 8.0);
        assert_eq!(dig_secs(8.0, &[Bedrock]), 16.0);
    }
}
