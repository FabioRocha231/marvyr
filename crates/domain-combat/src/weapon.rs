//! Bateria de bordo (PRD §19): o armamento principal dispara perpendicular ao
//! casco — combate de posicionamento, não de perseguição. Cada bordo tem sua
//! recarga, mas disparar um trava o outro por `CROSS_LOCK` da recarga: no
//! playtest dava para descarregar os dois bordos em sequência (dano dobrado
//! num instante, sobretudo casco com casco).

use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum BroadsideSide {
    /// Borda esquerda: direção = heading + π/2 (anti-horário).
    Port,
    /// Borda direita: direção = heading − π/2.
    Starboard,
}

impl BroadsideSide {
    /// Desvio angular do heading para a direção do tiro.
    pub fn angle_offset(self) -> f32 {
        match self {
            BroadsideSide::Port => std::f32::consts::FRAC_PI_2,
            BroadsideSide::Starboard => -std::f32::consts::FRAC_PI_2,
        }
    }
}

/// Fração da recarga que o bordo oposto fica travado após um disparo.
pub const CROSS_LOCK: f32 = 0.5;

#[derive(Debug, Clone, Copy, Default, PartialEq, Serialize, Deserialize)]
pub struct BroadsideBattery {
    pub port_cooldown: f32,
    pub starboard_cooldown: f32,
}

impl BroadsideBattery {
    pub fn is_ready(&self, side: BroadsideSide) -> bool {
        match side {
            BroadsideSide::Port => self.port_cooldown <= 0.0,
            BroadsideSide::Starboard => self.starboard_cooldown <= 0.0,
        }
    }

    /// Tenta disparar o bordo. `true` = disparou, o cooldown começou e o
    /// bordo oposto ficou travado por pelo menos `CROSS_LOCK` dele.
    pub fn try_fire(&mut self, side: BroadsideSide, cooldown_secs: f32) -> bool {
        if !self.is_ready(side) {
            return false;
        }
        let (fired, other) = match side {
            BroadsideSide::Port => (&mut self.port_cooldown, &mut self.starboard_cooldown),
            BroadsideSide::Starboard => (&mut self.starboard_cooldown, &mut self.port_cooldown),
        };
        *fired = cooldown_secs;
        *other = other.max(cooldown_secs * CROSS_LOCK);
        true
    }

    /// Avança o recarga dos dois bordos; nunca fica negativo.
    pub fn advance(&mut self, dt: f32) {
        self.port_cooldown = (self.port_cooldown - dt).max(0.0);
        self.starboard_cooldown = (self.starboard_cooldown - dt).max(0.0);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn battery_starts_ready_on_both_sides() {
        let battery = BroadsideBattery::default();
        assert!(battery.is_ready(BroadsideSide::Port));
        assert!(battery.is_ready(BroadsideSide::Starboard));
    }

    #[test]
    fn fire_locks_the_other_side_for_part_of_the_reload() {
        let mut battery = BroadsideBattery::default();
        assert!(battery.try_fire(BroadsideSide::Port, 4.0));
        assert!(!battery.try_fire(BroadsideSide::Starboard, 4.0));

        battery.advance(4.0 * CROSS_LOCK);
        assert!(!battery.is_ready(BroadsideSide::Port));
        assert!(battery.try_fire(BroadsideSide::Starboard, 4.0));
    }

    #[test]
    fn cross_lock_never_shortens_a_longer_reload() {
        let mut battery = BroadsideBattery {
            port_cooldown: 0.0,
            starboard_cooldown: 3.5,
        };
        assert!(battery.try_fire(BroadsideSide::Port, 4.0));
        assert!((battery.starboard_cooldown - 3.5).abs() < f32::EPSILON);
    }

    #[test]
    fn advance_ticks_cooldown_down_without_going_negative() {
        let mut battery = BroadsideBattery {
            port_cooldown: 4.0,
            starboard_cooldown: 0.5,
        };
        battery.advance(1.0);
        battery.advance(1.0);
        battery.advance(1.0);
        battery.advance(1.0);
        battery.advance(1.0);

        assert!((battery.port_cooldown - 0.0).abs() < f32::EPSILON);
        assert!(battery.is_ready(BroadsideSide::Port));
        assert!(battery.is_ready(BroadsideSide::Starboard));
    }

    #[test]
    fn port_is_counterclockwise_starboard_is_clockwise() {
        let pi_2 = std::f32::consts::FRAC_PI_2;
        assert!((BroadsideSide::Port.angle_offset() - pi_2).abs() < f32::EPSILON);
        assert!((BroadsideSide::Starboard.angle_offset() + pi_2).abs() < f32::EPSILON);
    }
}
