//! Combate naval profundo (MV-061): pontaria em 360°, avaria por zona do casco
//! e abordagem. Regras puras — o servidor decide quando chamar.

use serde::{Deserialize, Serialize};

use crate::weapon::BroadsideSide;

/// Pontaria em 360° (tiro automático): o bordo que mais encara o alvo e a
/// correção (radianos, somada ao través desse bordo) até a marcação dele.
/// Sem arco — o jogador não precisa apresentar o costado.
///
/// ponytail: alvo pela proa ou popa gira a salva ~90°, e as balas (dispostas
/// ao longo do casco) saem em fila em vez de leque; leque de verdade pede
/// uma salva montada na marcação, se a fila incomodar no playtest.
pub fn aim_at(heading: f32, shooter: (f32, f32), target: (f32, f32)) -> (BroadsideSide, f32) {
    let bearing = (target.1 - shooter.1).atan2(target.0 - shooter.0);
    let port = angle_delta(bearing, heading + BroadsideSide::Port.angle_offset());
    let starboard = angle_delta(bearing, heading + BroadsideSide::Starboard.angle_offset());
    if port.abs() <= starboard.abs() {
        (BroadsideSide::Port, port)
    } else {
        (BroadsideSide::Starboard, starboard)
    }
}

/// Mais próximo dentro do alcance entre `candidates` (id, posição).
pub fn nearest_in_range(
    shooter: (f32, f32),
    range: f32,
    candidates: impl IntoIterator<Item = (u32, (f32, f32))>,
) -> Option<(u32, (f32, f32))> {
    candidates
        .into_iter()
        .map(|(id, at)| {
            let (dx, dy) = (at.0 - shooter.0, at.1 - shooter.1);
            (id, at, dx * dx + dy * dy)
        })
        .filter(|(_, _, d2)| *d2 <= range * range)
        .min_by(|a, b| a.2.total_cmp(&b.2))
        .map(|(id, at, _)| (id, at))
}

/// Parte do casco atingida, pela posição do impacto relativa à proa.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum HitZone {
    Bow,
    Midship,
    /// Popa: onde fica o leme.
    Stern,
}

/// Cone de popa/proa: impacto a menos de ~35° do eixo do navio.
const END_CONE_COS: f32 = 0.82;

pub fn hit_zone(ship: (f32, f32), heading: f32, impact: (f32, f32)) -> HitZone {
    let (dx, dy) = (impact.0 - ship.0, impact.1 - ship.1);
    let dist = (dx * dx + dy * dy).sqrt();
    if dist <= f32::EPSILON {
        return HitZone::Midship;
    }
    let along = (dx * heading.cos() + dy * heading.sin()) / dist;
    if along >= END_CONE_COS {
        HitZone::Bow
    } else if along <= -END_CONE_COS {
        HitZone::Stern
    } else {
        HitZone::Midship
    }
}

/// Pontos de leme (0..100) perdidos por um golpe na popa, proporcionais ao
/// casco do alvo: a mesma bala avaria mais o leme de um casco pequeno.
pub fn rudder_points(hull_damage: u32, target_max_hp: u32, zone: HitZone) -> f32 {
    if zone != HitZone::Stern || target_max_hp == 0 {
        return 0.0;
    }
    (hull_damage as f32 / target_max_hp as f32 * 250.0).min(100.0)
}

/// Resultado de uma abordagem.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BoardingOutcome {
    /// O alvo rendeu; o atacante perdeu `attacker_losses` marujos.
    Captured { attacker_losses: u16 },
    /// Abordagem repelida: baixas dos dois lados.
    Repelled {
        attacker_losses: u16,
        defender_losses: u16,
    },
}

/// Chance (0..1) de o atacante tomar o navio: força de cada lado é a
/// tripulação, e o defensor avariado luta pior (casco a 20% vale 60%).
pub fn boarding_chance(attacker_crew: u16, defender_crew: u16, defender_hull_ratio: f32) -> f32 {
    let attack = f32::from(attacker_crew);
    let defense = f32::from(defender_crew) * (0.5 + 0.5 * defender_hull_ratio.clamp(0.0, 1.0));
    if attack <= 0.0 {
        return 0.0;
    }
    (attack / (attack + defense.max(0.5))).clamp(0.05, 0.95)
}

/// Resolve a abordagem com um rolo `roll` em [0, 1) (o servidor sorteia).
pub fn resolve_boarding(
    attacker_crew: u16,
    defender_crew: u16,
    defender_hull_ratio: f32,
    roll: f32,
) -> BoardingOutcome {
    let chance = boarding_chance(attacker_crew, defender_crew, defender_hull_ratio);
    if roll < chance {
        BoardingOutcome::Captured {
            attacker_losses: (defender_crew / 3).min(attacker_crew),
        }
    } else {
        BoardingOutcome::Repelled {
            attacker_losses: attacker_crew.div_ceil(2),
            defender_losses: (attacker_crew / 3).min(defender_crew),
        }
    }
}

fn angle_delta(target: f32, current: f32) -> f32 {
    let mut delta = (target - current).rem_euclid(std::f32::consts::TAU);
    if delta > std::f32::consts::PI {
        delta -= std::f32::consts::TAU;
    }
    delta
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::f32::consts::FRAC_PI_2;

    #[test]
    fn aim_turns_the_facing_side_onto_the_target_in_any_direction() {
        // Aproado para +X; bombordo aponta para +Y.
        let (side, delta) = aim_at(0.0, (0.0, 0.0), (0.0, 100.0));
        assert_eq!(side, BroadsideSide::Port);
        assert!(delta.abs() < 1e-5, "alvo no través: tiro reto");

        let (side, delta) = aim_at(0.0, (0.0, 0.0), (30.0, -100.0));
        assert_eq!(side, BroadsideSide::Starboard);
        assert!(delta > 0.0 && delta < FRAC_PI_2, "puxa para a proa");

        // Pela proa: a salva gira ~90° até o alvo, sem arco limitando.
        let (side, delta) = aim_at(0.0, (0.0, 0.0), (100.0, 1.0));
        let flight = side.angle_offset() + delta;
        assert!(flight.abs() < 0.02, "sai rumo à proa");
    }

    #[test]
    fn nearest_in_range_ignores_far_and_picks_closest() {
        let picked = nearest_in_range(
            (0.0, 0.0),
            100.0,
            [(1, (90.0, 0.0)), (2, (0.0, 40.0)), (3, (500.0, 0.0))],
        );
        assert_eq!(picked, Some((2, (0.0, 40.0))));
        assert_eq!(nearest_in_range((0.0, 0.0), 10.0, [(1, (90.0, 0.0))]), None);
    }

    #[test]
    fn stern_hits_hurt_rudder_and_bow_or_midship_do_not() {
        assert_eq!(hit_zone((0.0, 0.0), 0.0, (-15.0, 1.0)), HitZone::Stern);
        assert_eq!(hit_zone((0.0, 0.0), 0.0, (15.0, -1.0)), HitZone::Bow);
        assert_eq!(hit_zone((0.0, 0.0), 0.0, (1.0, 15.0)), HitZone::Midship);
        assert!(rudder_points(20, 100, HitZone::Stern) > 0.0);
        assert_eq!(rudder_points(20, 100, HitZone::Midship), 0.0);
        assert_eq!(rudder_points(500, 100, HitZone::Stern), 100.0);
    }

    #[test]
    fn crew_and_damage_decide_boarding() {
        assert!(boarding_chance(20, 5, 0.2) > boarding_chance(5, 20, 1.0));
        assert_eq!(boarding_chance(0, 5, 0.5), 0.0);
        assert!(matches!(
            resolve_boarding(20, 4, 0.1, 0.0),
            BoardingOutcome::Captured { attacker_losses: 1 }
        ));
        assert_eq!(
            resolve_boarding(4, 20, 1.0, 0.99),
            BoardingOutcome::Repelled {
                attacker_losses: 2,
                defender_losses: 1
            }
        );
    }
}
