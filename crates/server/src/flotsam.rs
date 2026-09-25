//! Destroços à deriva (MV-067): a travessia rende alguma coisa. A cada
//! 30–60 s, perto de cada capitão no mar, boiam barris e tábuas com recurso
//! bruto da água em que ele está — nunca item pronto (pilar 1). Reusa o
//! wreck: mesmo saque (F), mesma expiração.

use std::collections::HashMap;

use bevy::prelude::*;
use marvyr_domain_ships::VesselPresence;
use marvyr_domain_world::{RiskTier, WorldMap};
use marvyr_shared::ids::ItemDefinitionId;

use crate::net::{
    DevItems, LiveWreckRecords, ServerShip, ServerWorldMap, ServerWreck, WreckIdCounter,
};
use crate::sets::SimulationSet;

/// Janela (s) entre dois achados para o mesmo capitão.
const EVERY: (f32, f32) = (30.0, 60.0);
/// Onde boia: à frente do casco, a esta distância (m)…
const AHEAD: (f32, f32) = (220.0, 380.0);
/// …e até esta distância de lado.
const SPREAD: f32 = 160.0;
/// Com tantos destroços já por perto, não aparece outro.
const NEARBY_CAP: usize = 2;
const NEARBY_RADIUS: f32 = 600.0;

/// Próximo achado de cada navio (s de simulação).
#[derive(Resource, Default)]
pub struct FlotsamClock(HashMap<u32, f32>);

pub fn install(app: &mut App) {
    app.init_resource::<FlotsamClock>();
    app.add_systems(
        FixedUpdate,
        drift_flotsam.in_set(SimulationSet::EconomyConsequences),
    );
}

fn unit() -> f32 {
    (crate::seafaring::roll() >> 104) as f32 / (1u32 << 24) as f32
}

fn between((min, max): (f32, f32)) -> f32 {
    min + (max - min) * unit()
}

/// O que boia depende da água: madeira e minério perto de casa, coral e
/// pérola onde é perigoso. Quantidades pequenas — é tempero, não farm.
/// Águas protegidas (o spawn) não dão nada: quem quer material sai do porto.
fn contents(tier: RiskTier, dev: &DevItems, pick: f32) -> Vec<(ItemDefinitionId, u32)> {
    match tier {
        RiskTier::Protected => Vec::new(),
        RiskTier::Frontier if pick < 0.5 => vec![(dev.timber, 7)],
        RiskTier::Frontier => vec![(dev.ore, 7)],
        RiskTier::Lawless if pick < 0.7 => vec![(dev.coral, 3)],
        RiskTier::Lawless => vec![(dev.abyssal_pearl, 1)],
    }
}

/// Tentativas por achado: à frente primeiro, depois girando em volta do
/// casco (proa contra a costa ou o paredão não pode zerar o achado).
const TRIES: usize = 8;

/// Ponto de água perto do navio, dentro de uma zona (fora das instâncias)
/// e longe de terra. `None` só se nenhuma direção serviu.
fn drift_point(map: &WorldMap, x: f32, y: f32, heading: f32) -> Option<(f32, f32)> {
    (0..TRIES).find_map(|k| {
        let bearing = heading + k as f32 * std::f32::consts::TAU / TRIES as f32;
        let ahead = between(AHEAD);
        let side = between((-SPREAD, SPREAD));
        let (fx, fy) = (bearing.cos(), bearing.sin());
        let at = (x + fx * ahead - fy * side, y + fy * ahead + fx * side);
        let is_open_water = map.area_at(at.0, at.1).is_some()
            && map.exit_at(at.0, at.1).is_none()
            && map.push_out_of_land(at.0, at.1, 30.0).is_none();
        is_open_water.then_some(at)
    })
}

// System Bevy: params são injeção de dependência, não assinatura.
#[allow(clippy::too_many_arguments)]
fn drift_flotsam(
    mut commands: Commands,
    time: Res<Time>,
    map: Res<ServerWorldMap>,
    dev: Res<DevItems>,
    mut clock: ResMut<FlotsamClock>,
    (mut wreck_ids, mut live_wrecks): (ResMut<WreckIdCounter>, ResMut<LiveWreckRecords>),
    ships: Query<&ServerShip>,
    wrecks: Query<&ServerWreck>,
) {
    let now = time.elapsed_secs();
    for ship in &ships {
        let at_sea = matches!(ship.presence, VesselPresence::AtSea);
        if ship.client_id.is_none() || !at_sea {
            continue;
        }
        let due = clock.0.entry(ship.ship_id).or_insert(now + between(EVERY));
        if now < *due {
            continue;
        }
        *due = now + between(EVERY);
        let (x, y) = (ship.motion.x, ship.motion.y);
        let nearby = wrecks
            .iter()
            .filter(|w| (w.x - x).hypot(w.y - y) <= NEARBY_RADIUS)
            .count();
        if nearby >= NEARBY_CAP {
            continue;
        }
        let Some(point) = drift_point(&map.0, x, y, ship.motion.heading) else {
            continue;
        };
        let Ok(zone) = map.0.zone_at(point.0, point.1) else {
            continue;
        };
        let spoils = contents(zone.tier, &dev, unit());
        if spoils.is_empty() {
            continue;
        }
        crate::npc::spawn_spoils_wreck(
            &mut commands,
            &mut wreck_ids,
            &mut live_wrecks,
            spoils,
            None,
            point,
            now,
        );
    }
    // Navio que saiu (desconectou, afundou) não guarda relógio.
    clock
        .0
        .retain(|ship_id, _| ships.iter().any(|ship| ship.ship_id == *ship_id));
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn flotsam_is_raw_resource_matching_the_water() {
        let dev = DevItems::new();
        for pick in [0.0, 0.6, 0.99] {
            for tier in [RiskTier::Protected, RiskTier::Frontier, RiskTier::Lawless] {
                for (item, quantity) in contents(tier, &dev, pick) {
                    let definition = dev.catalog.get(item).expect("no catálogo");
                    assert_eq!(definition.kind, marvyr_domain_items::ItemKind::Resource);
                    assert!((1..=7).contains(&quantity));
                }
            }
        }
    }

    #[test]
    fn protected_waters_drift_nothing() {
        let dev = DevItems::new();
        for pick in [0.0, 0.6, 0.99] {
            assert!(contents(RiskTier::Protected, &dev, pick).is_empty());
        }
    }

    #[test]
    fn drift_points_are_open_water_inside_a_zone() {
        let map = WorldMap::from_seed(crate::net::DEFAULT_WORLD_SEED);
        let (sx, sy) = map.features().spawn;
        let mut found = 0;
        for k in 0..200 {
            let heading = k as f32 * 0.37;
            if let Some((x, y)) = drift_point(&map, sx, sy, heading) {
                found += 1;
                assert!(map.push_out_of_land(x, y, 29.0).is_none());
                assert!(map.area_at(x, y).is_some());
            }
        }
        assert!(found > 190, "achado falhou demais: {found}/200");
    }
}
