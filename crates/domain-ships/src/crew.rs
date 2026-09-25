//! Tripulação, leme e reparo em mar (MV-061). Marujo é capacidade comprada
//! com ouro no porto (sink) e perdida no mar — afundou, a tripulação foi
//! junto (Pilar 2). Nada aqui é XP: é propriedade embarcada.

use crate::definition::ShipKind;

/// Tripulação máxima por casco.
pub fn crew_capacity(kind: ShipKind) -> u16 {
    match kind {
        ShipKind::SmallMerchant => 8,
        ShipKind::Patrol => 16,
        ShipKind::Corsair => 24,
    }
}

/// Marujos de graça que todo casco novo traz (sem tripulação o jogador não
/// sairia do porto — o esqueleto mínimo não é recompensa, é piso).
pub const SKELETON_CREW: u16 = 4;

/// Mantimento por marujo contratado: madeira do armazém do porto (o soldo
/// em ouro saiu com a moeda).
pub const CREW_WAGE_ITEM: &str = "Madeira";
pub const CREW_WAGE: u32 = 3;

/// Multiplicador da recarga de bordo: tripulação cheia 1,0x; sem ninguém
/// para carregar os canhões, 1,8x.
pub fn reload_multiplier(crew: u16, capacity: u16) -> f32 {
    1.8 - 0.8 * crew_ratio(crew, capacity)
}

fn crew_ratio(crew: u16, capacity: u16) -> f32 {
    if capacity == 0 {
        return 0.0;
    }
    (f32::from(crew) / f32::from(capacity)).clamp(0.0, 1.0)
}

/// Marujos mortos por um impacto no casco: cada 12% do casco perdido custa
/// um marujo (arredondado para baixo, mínimo 0).
pub fn casualties(hull_damage: u32, max_hp: u32, crew: u16) -> u16 {
    if max_hp == 0 {
        return 0;
    }
    let lost = (hull_damage as f32 / max_hp as f32 / 0.12).floor() as u16;
    lost.min(crew)
}

pub const RUDDER_HP_MAX: f32 = 100.0;

/// Leme avariado governa menos: 30% do giro com o leme destruído.
pub fn rudder_turn_multiplier(rudder_hp: f32) -> f32 {
    0.3 + 0.7 * (rudder_hp / RUDDER_HP_MAX).clamp(0.0, 1.0)
}

/// Segundos sem levar tiro antes de o reparo poder começar.
pub const REPAIR_COMBAT_LOCK_SECS: f32 = 6.0;

/// Um ciclo de reparo (1 s de trabalho da tripulação).
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct RepairStep {
    pub hull_gain: u32,
    pub rudder_gain: f32,
    /// Unidades de Madeira consumidas do porão.
    pub timber_used: u32,
}

/// Quanto um ciclo de reparo recupera. Consome 1 Madeira por ciclo; sem
/// madeira, sem tripulação ou sem avaria, nada acontece (`None`).
/// Cada marujo recupera 0,5% do casco e 2 pontos de leme por ciclo.
pub fn repair_step(
    hp: u32,
    max_hp: u32,
    rudder_hp: f32,
    crew: u16,
    timber_available: u32,
) -> Option<RepairStep> {
    let hull_missing = max_hp.saturating_sub(hp);
    let rudder_missing = (RUDDER_HP_MAX - rudder_hp).max(0.0);
    if crew == 0 || timber_available == 0 || (hull_missing == 0 && rudder_missing <= 0.0) {
        return None;
    }
    let hull_gain = ((max_hp as f32 * 0.005 * f32::from(crew)).ceil() as u32).min(hull_missing);
    let rudder_gain = (2.0 * f32::from(crew)).min(rudder_missing);
    Some(RepairStep {
        hull_gain,
        rudder_gain,
        timber_used: 1,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn bigger_hulls_carry_more_crew() {
        assert!(crew_capacity(ShipKind::Corsair) > crew_capacity(ShipKind::SmallMerchant));
        assert!(SKELETON_CREW <= crew_capacity(ShipKind::SmallMerchant));
    }

    #[test]
    fn full_crew_reloads_fastest() {
        assert!((reload_multiplier(8, 8) - 1.0).abs() < 1e-6);
        assert!((reload_multiplier(0, 8) - 1.8).abs() < 1e-6);
        assert!(reload_multiplier(4, 8) > 1.0);
    }

    #[test]
    fn heavy_hits_kill_sailors_but_never_more_than_aboard() {
        assert_eq!(casualties(10, 200, 8), 0);
        assert_eq!(casualties(50, 200, 8), 2);
        assert_eq!(casualties(500, 200, 3), 3);
    }

    #[test]
    fn rudder_damage_slows_the_turn() {
        assert_eq!(rudder_turn_multiplier(RUDDER_HP_MAX), 1.0);
        assert!((rudder_turn_multiplier(0.0) - 0.3).abs() < 1e-6);
    }

    #[test]
    fn repair_needs_timber_crew_and_damage() {
        let step = repair_step(100, 200, 50.0, 8, 5).unwrap();
        assert_eq!(step.hull_gain, 8);
        assert_eq!(step.rudder_gain, 16.0);
        assert_eq!(step.timber_used, 1);
        assert_eq!(repair_step(100, 200, 50.0, 8, 0), None);
        assert_eq!(repair_step(100, 200, 50.0, 0, 5), None);
        assert_eq!(repair_step(200, 200, 100.0, 8, 5), None);
        assert_eq!(repair_step(199, 200, 100.0, 8, 5).unwrap().hull_gain, 1);
    }
}
