//! v47: moral da tripulação. No mar ela cai devagar (mais rápido à noite);
//! na luz de um farol se recupera; atracar enche. Abaixo de `RATION_AT` a
//! tripulação come um Peixe do porão sozinha. Moral baixa solta menos pano:
//! o navio anda menos (`speed_factor`). Motivo para voltar ao porto, pescar
//! e erguer faróis — nada disso vem de NPC.

use bevy::prelude::*;
use lightyear::prelude::server::*;
use marvyr_domain_ships::VesselPresence;
use marvyr_protocol::ActionKind;

use crate::net::ServerShip;
use crate::seafaring::send_action;
use crate::sets::SimulationSet;

pub const FULL: f32 = 100.0;
/// Queda por minuto no mar de dia; a noite soma até outro tanto.
pub const DECAY_PER_MIN: f32 = 4.0;
/// Recuperação por minuto na luz de um farol.
pub const LIGHT_PER_MIN: f32 = 6.0;
/// Abaixo disto a tripulação come um Peixe do porão.
pub const RATION_AT: f32 = 60.0;
pub const RATION: f32 = 15.0;
const RATION_COOLDOWN: f32 = 20.0;
/// Abaixo disto o navio anda menos (e o capitão é avisado).
pub const LOW: f32 = 40.0;

/// Pano que a tripulação solta com a moral dada.
pub fn speed_factor(morale: f32) -> f32 {
    if morale >= LOW {
        1.0
    } else if morale >= 20.0 {
        0.9
    } else {
        0.8
    }
}

/// Moral depois de `dt` segundos no mar (sem ração).
pub fn step(morale: f32, lit: bool, night: f32, dt: f32) -> f32 {
    let per_min = if lit {
        LIGHT_PER_MIN
    } else {
        -DECAY_PER_MIN * (1.0 + night)
    };
    (morale + per_min * dt / 60.0).clamp(0.0, FULL)
}

pub fn install(app: &mut App) {
    app.add_systems(
        FixedUpdate,
        tick_morale.in_set(SimulationSet::EconomyConsequences),
    );
}

fn tick_morale(
    time: Res<Time>,
    counter: Res<crate::plugin::TickCounter>,
    lighthouses: Res<crate::lighthouse::Lighthouses>,
    mut ships: Query<&mut ServerShip>,
    mut connection_manager: ResMut<ConnectionManager>,
) {
    let dt = time.delta_secs();
    let night = marvyr_protocol::night_of(u64::from(counter.0));
    let fish = crate::fishing::fish_id();
    for mut ship in &mut ships {
        if matches!(ship.presence, VesselPresence::Docked(_)) {
            ship.sea.morale = FULL;
            ship.sea.ration_cooldown = 0.0;
            continue;
        }
        let at = Vec2::new(ship.motion.x, ship.motion.y);
        let before = ship.sea.morale;
        let mut after = step(before, lighthouses.lit(at), night, dt);
        ship.sea.ration_cooldown = (ship.sea.ration_cooldown - dt).max(0.0);
        let mut ate = false;
        if after < RATION_AT && ship.sea.ration_cooldown <= 0.0 && ship.hold.remove(fish, 1).is_ok()
        {
            after = (after + RATION).min(FULL);
            ship.sea.ration_cooldown = RATION_COOLDOWN;
            ate = true;
        }
        ship.sea.morale = after;
        let Some(client_id) = ship.client_id else {
            continue;
        };
        if ate {
            send_action(
                &mut connection_manager,
                client_id,
                ActionKind::Morale,
                true,
                "A tripulação comeu um Peixe: moral +15.",
            );
        } else if before >= LOW && after < LOW {
            send_action(
                &mut connection_manager,
                client_id,
                ActionKind::Morale,
                false,
                "A tripulação desanimou: o navio anda menos. Peixe no porão, farol ou porto levantam a moral.",
            );
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn night_drains_twice_as_fast_and_a_lighthouse_restores() {
        let day = FULL - step(FULL, false, 0.0, 60.0);
        let night = FULL - step(FULL, false, 1.0, 60.0);
        assert!((day - DECAY_PER_MIN).abs() < 1e-4);
        assert!((night - 2.0 * DECAY_PER_MIN).abs() < 1e-4);
        assert!(step(50.0, true, 1.0, 60.0) > 50.0);
        assert_eq!(step(0.5, false, 1.0, 60.0), 0.0);
    }

    #[test]
    fn low_morale_slows_the_ship() {
        assert_eq!(speed_factor(FULL), 1.0);
        assert!(speed_factor(30.0) < 1.0);
        assert!(speed_factor(5.0) < speed_factor(30.0));
    }
}
