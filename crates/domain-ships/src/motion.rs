//! Modelo de movimento naval puro (PRD §17): posição 2D, heading, velocidade
//! escalar, aceleração e turn rate. Sem Bevy, sem física rígida — a "sensação
//! de peso" vem de duas regras: o leme só mordem com água passando (navio
//! parado não vira) e virar a toda velocidade descreve arco largo.

use serde::{Deserialize, Serialize};

use crate::stats::ShipStats;

/// Estado cinemático do navio no mundo. `heading` é em radianos, 0 apontando
/// para +X e crescendo anti-horário; `speed` é escalar em m/s.
#[derive(Debug, Clone, Copy, Default, PartialEq, Serialize, Deserialize)]
pub struct ShipMotion {
    pub x: f32,
    pub y: f32,
    pub heading: f32,
    pub speed: f32,
}

/// Intenção do jogador. `throttle` é clampeado para [0, 1] (não há ré a vela:
/// sair de uma posição ruim é decisão de navegação, não botão de voltar).
/// `turn` é clampeado para [-1, 1].
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub struct MotionInput {
    pub throttle: f32,
    pub turn: f32,
}

/// Parâmetros de tuning do casco (PRD §23: valores de balanceamento vivem em
/// configuração, não espalhados no código).
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub struct MotionTuning {
    /// Aceleração máxima a motor/vela, m/s².
    pub max_accel: f32,
    /// Desaceleração máxima (arrancar velas, atrito da água), m/s².
    pub max_decel: f32,
    /// Fração da velocidade máxima em que o leme atinge 100% de efeito.
    /// Abaixo disso, a eficácia cai proporcionalmente até zero em parada.
    pub rudder_reference_ratio: f32,
}

impl Default for MotionTuning {
    fn default() -> Self {
        Self {
            // Tuning de jogabilidade do slice: peso perceptível (chega na
            // velocidade em ~4s) sem parecer que o navio não responde.
            max_accel: 8.0,
            max_decel: 8.0,
            rudder_reference_ratio: 0.15,
        }
    }
}

/// Avança o movimento do navio um passo de simulação (`dt` em segundos).
/// `dt <= 0` é ignorado (defensivo: o loop de simulação nunca deveria passar).
/// A velocidade-alvo é `throttle * speed`: o vento saiu da navegação (o rumo
/// não freia o navio; a Tempestade continua rasgando o pano).
pub fn step_motion(
    motion: &mut ShipMotion,
    stats: &ShipStats,
    input: MotionInput,
    tuning: &MotionTuning,
    dt: f32,
) {
    if dt <= 0.0 {
        return;
    }

    let throttle = input.throttle.clamp(0.0, 1.0);
    let turn = input.turn.clamp(-1.0, 1.0);

    // Velocidade persegue a posição da manche com taxas finitas — o navio
    // nunca instancia velocidade nem freia na hora.
    let target_speed = throttle * stats.speed;
    if motion.speed < target_speed {
        motion.speed = (motion.speed + tuning.max_accel * dt).min(target_speed);
    } else if motion.speed > target_speed {
        motion.speed = (motion.speed - tuning.max_decel * dt).max(target_speed);
    }

    // Leme proporcional à água passando pelo casco: em parada, 0; a partir
    // da velocidade de referência, 1. A toda velocidade o arco de virada é
    // largo porque o raio é speed / turn_rate.
    let reference_speed = stats.speed * tuning.rudder_reference_ratio;
    let rudder = if reference_speed > 0.0 {
        (motion.speed / reference_speed).min(1.0)
    } else {
        0.0
    };

    motion.heading = normalize_angle(motion.heading + turn * stats.turn_rate * rudder * dt);

    motion.x += motion.heading.cos() * motion.speed * dt;
    motion.y += motion.heading.sin() * motion.speed * dt;
}

fn normalize_angle(angle: f32) -> f32 {
    angle.rem_euclid(std::f32::consts::TAU)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn stats() -> ShipStats {
        ShipStats {
            speed: 6.0,
            turn_rate: 1.0,
            max_hp: 100,
            cargo_capacity: 100,
            weapon_damage: 20,
            weapon_range: 50.0,
        }
    }

    fn tuning() -> MotionTuning {
        // Valores fixados: os testes cobrem as REGRAS do modelo, não o
        // tuning de jogo (que muda por balanceamento).
        MotionTuning {
            max_accel: 1.0,
            max_decel: 2.0,
            rudder_reference_ratio: 0.3,
        }
    }

    fn input(throttle: f32, turn: f32) -> MotionInput {
        MotionInput { throttle, turn }
    }

    fn cruise() -> f32 {
        stats().speed
    }

    fn steps(
        n: usize,
        motion: &mut ShipMotion,
        stats: &ShipStats,
        i: MotionInput,
        t: &MotionTuning,
    ) {
        for _ in 0..n {
            step_motion(motion, stats, i, t, 0.1);
        }
    }

    #[test]
    fn stopped_ship_with_zero_throttle_stays_put() {
        let mut motion = ShipMotion::default();
        steps(10, &mut motion, &stats(), input(0.0, 0.0), &tuning());

        assert_eq!(motion.speed, 0.0);
        assert_eq!(motion.x, 0.0);
        assert_eq!(motion.y, 0.0);
    }

    #[test]
    fn throttle_moves_ship_forward_along_heading() {
        let mut motion = ShipMotion {
            heading: 0.0, // +X
            ..ShipMotion::default()
        };
        steps(10, &mut motion, &stats(), input(1.0, 0.0), &tuning());

        assert!(motion.speed > 0.0);
        assert!(motion.x > 0.0);
        assert_eq!(motion.y, 0.0);
    }

    #[test]
    fn speed_never_exceeds_max_speed() {
        let mut motion = ShipMotion::default();
        steps(600, &mut motion, &stats(), input(1.0, 0.0), &tuning());

        assert!((motion.speed - cruise()).abs() < 1e-4);
    }

    #[test]
    fn ship_decelerates_gradually_when_throttle_drops() {
        let mut motion = ShipMotion::default();
        steps(600, &mut motion, &stats(), input(1.0, 0.0), &tuning());
        let speed_before = motion.speed;

        step_motion(&mut motion, &stats(), input(0.0, 0.0), &tuning(), 0.1);

        let lost = speed_before - motion.speed;
        assert!(lost > 0.0 && lost <= tuning().max_decel * 0.1 + f32::EPSILON);
    }

    #[test]
    fn stopped_ship_cannot_turn() {
        let mut motion = ShipMotion::default();
        steps(10, &mut motion, &stats(), input(0.0, 1.0), &tuning());

        assert_eq!(motion.heading, 0.0);
    }

    #[test]
    fn slow_ship_turns_with_reduced_rudder() {
        let t = tuning();
        // Meio da referência (0.3 * 6.0 = 1.8 m/s) → leme com metade do efeito.
        // Throttle calibrado mantém exatamente 0.9 m/s.
        let mut motion = ShipMotion {
            speed: 0.9,
            ..ShipMotion::default()
        };
        step_motion(&mut motion, &stats(), input(0.9 / cruise(), 1.0), &t, 0.1);

        let expected_delta = 1.0 * stats().turn_rate * (0.9 / 1.8) * 0.1;
        assert!((motion.heading - expected_delta).abs() < 1e-5);
    }

    #[test]
    fn full_speed_turn_rate_is_exactly_turn_rate() {
        let mut motion = ShipMotion::default();
        steps(600, &mut motion, &stats(), input(1.0, 0.0), &tuning());

        let heading_before = motion.heading;
        step_motion(&mut motion, &stats(), input(1.0, 1.0), &tuning(), 0.1);

        let delta = motion.heading - heading_before;
        assert!((delta - stats().turn_rate * 0.1).abs() < 1e-5);
    }

    #[test]
    fn turn_input_is_clamped() {
        let mut motion = ShipMotion {
            speed: stats().speed,
            ..ShipMotion::default()
        };
        step_motion(&mut motion, &stats(), input(1.0, 50.0), &tuning(), 0.1);

        let delta = motion.heading;
        assert!(delta <= stats().turn_rate * 0.1 + 1e-5);
    }

    #[test]
    fn heading_wraps_into_zero_to_tau() {
        let mut motion = ShipMotion {
            heading: std::f32::consts::TAU - 0.05,
            speed: stats().speed,
            ..ShipMotion::default()
        };
        step_motion(&mut motion, &stats(), input(1.0, 1.0), &tuning(), 0.1);

        assert!((0.0..std::f32::consts::TAU).contains(&motion.heading));
    }

    #[test]
    fn negative_or_zero_dt_is_ignored() {
        let mut motion = ShipMotion::default();
        step_motion(&mut motion, &stats(), input(1.0, 1.0), &tuning(), 0.0);
        step_motion(&mut motion, &stats(), input(1.0, 1.0), &tuning(), -1.0);

        assert_eq!(motion, ShipMotion::default());
    }
}
