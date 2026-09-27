//! Pano (MF-059): velas rasgadas rendem menos. O vento saiu da navegação —
//! o rumo não freia o navio; ele fica só como clima (a Tempestade rasga o
//! pano) e ambiente.

use serde::{Deserialize, Serialize};

/// Vento num ponto do mar. `direction` é para ONDE o vento sopra (radianos,
/// mesma convenção do heading: 0 = +X, anti-horário); `strength` em [0, 1].
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub struct Wind {
    pub direction: f32,
    pub strength: f32,
}

/// Velas inteiras.
pub const SAIL_HP_MAX: f32 = 100.0;
/// Remendo em alto-mar: +1 ponto a cada 3 s.
pub const SAIL_REPAIR_PER_SEC: f32 = 1.0 / 3.0;
/// Tempestade rasga o pano (por segundo, na intensidade máxima).
pub const STORM_SAIL_DAMAGE_PER_SEC: f32 = 2.0;

/// Pano rasgado rende menos: `0.35 + 0.65 * sail_hp / 100`.
pub fn sail_speed_multiplier(sail_hp: f32) -> f32 {
    0.35 + 0.65 * (sail_hp / SAIL_HP_MAX).clamp(0.0, 1.0)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn close(a: f32, b: f32) -> bool {
        (a - b).abs() < 1e-4
    }

    #[test]
    fn sail_multiplier_spans_035_to_1() {
        assert!(close(sail_speed_multiplier(100.0), 1.0));
        assert!(close(sail_speed_multiplier(0.0), 0.35));
        assert!(close(sail_speed_multiplier(50.0), 0.675));
        assert!(close(sail_speed_multiplier(-5.0), 0.35));
        assert!(close(sail_speed_multiplier(250.0), 1.0));
    }
}
