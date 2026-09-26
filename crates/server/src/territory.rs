//! v45: influência de porto. Os portos fora das águas da coroa são
//! disputados: todo Renome ganho perto de um deles vira influência ali, e
//! quem tem mais na semana é o Senhor do Porto. O Senhor, ao atracar no seu
//! porto, cobra uma vez por dia o tributo — o recurso do porto, pago pela
//! guilda (bruto de NPC, Pilar 1). Nada sai do bolso de outro capitão.
//!
//! ponytail: sem guilda de jogador ainda, a disputa é de capitão; quando
//! houver companhias, a influência soma por companhia.

use std::collections::HashMap;

use bevy::prelude::*;
use lightyear::prelude::server::*;
use marvyr_domain_ships::VesselPresence;
use marvyr_domain_world::{RiskTier, WorldMap};
use marvyr_protocol::WorldEventKind;
use marvyr_shared::ids::{CharacterId, ItemDefinitionId};
use tracing::{info, warn};

use crate::market::ServerMarket;
use crate::net::{DevItems, ServerShip, ServerWorldMap};
use crate::persist::StoreHandle;
use crate::progress::CaptainLogbook;
use crate::sets::SimulationSet;

/// Renome ganho até esta distância de um porto disputado conta para ele (m).
pub const INFLUENCE_RADIUS: f32 = 600.0;
/// Tributo diário do Senhor do Porto (unidades do recurso do porto).
pub const TRIBUTE: u32 = 20;
const REFRESH_EVERY: f32 = 10.0;

/// Portos disputados: os que não estão em águas protegidas.
pub fn contested_ports(map: &WorldMap) -> Vec<(&'static str, Vec2)> {
    map.regions()
        .iter()
        .filter_map(|region| region.port.as_ref())
        .filter(|port| {
            map.zone_at(port.x, port.y)
                .is_ok_and(|zone| zone.tier != RiskTier::Protected)
        })
        .map(|port| (port.name, Vec2::new(port.x, port.y)))
        .collect()
}

/// Porto disputado mais perto de (x, y), se estiver no raio.
pub fn port_near(map: &WorldMap, at: Vec2) -> Option<&'static str> {
    contested_ports(map)
        .into_iter()
        .map(|(name, port)| (name, port.distance(at)))
        .filter(|(_, distance)| *distance <= INFLUENCE_RADIUS)
        .min_by(|a, b| a.1.total_cmp(&b.1))
        .map(|(name, _)| name)
}

/// Senhores da semana: porto → (capitão, influência).
#[derive(Resource, Default)]
pub struct PortLords {
    week: Option<u32>,
    pub lords: HashMap<&'static str, (CharacterId, u32)>,
    clock: f32,
}

impl PortLords {
    pub fn lord_of(&self, port: &str) -> Option<CharacterId> {
        self.lords.get(port).map(|(character, _)| *character)
    }
}

pub fn install(app: &mut App) {
    app.init_resource::<PortLords>().add_systems(
        FixedUpdate,
        (refresh_lords, collect_tribute)
            .chain()
            .in_set(SimulationSet::EconomyConsequences),
    );
}

fn refresh_lords(
    time: Res<Time>,
    map: Res<ServerWorldMap>,
    store: Res<StoreHandle>,
    logbook: Res<CaptainLogbook>,
    mut lords: ResMut<PortLords>,
) {
    lords.clock += time.delta_secs();
    if lords.clock < REFRESH_EVERY && lords.week.is_some() {
        return;
    }
    lords.clock = 0.0;
    let (_, week) = crate::progress::today();
    let ports = contested_ports(&map.0);
    if lords.week != Some(week) {
        lords.week = Some(week);
        lords.lords.clear();
        if let Some(store) = &store.0 {
            for (port, _) in &ports {
                match store.load_port_lord(week, port) {
                    Ok(Some(best)) => {
                        lords.lords.insert(port, best);
                    }
                    Ok(None) => {}
                    Err(error) => warn!(%error, port, "Senhor do Porto não carregou do banco"),
                }
            }
        }
    }
    for (port, _) in &ports {
        for (character, progress) in logbook.captains() {
            let points = progress.influence_at(week, port);
            let best = lords.lords.get(port).map_or(0, |(_, p)| *p);
            let holder = lords.lord_of(port);
            if points > best || (holder == Some(character) && points > 0) {
                lords.lords.insert(port, (character, points));
            }
        }
    }
}

/// O Senhor atracou no seu porto: tributo do dia no armazém.
// System Bevy: params são injeção de dependência, não assinatura.
#[allow(clippy::too_many_arguments)]
fn collect_tribute(
    ships: Query<&ServerShip>,
    map: Res<ServerWorldMap>,
    lords: Res<PortLords>,
    dev: Res<DevItems>,
    mut market: ResMut<ServerMarket>,
    mut logbook: ResMut<CaptainLogbook>,
    store: Res<StoreHandle>,
    mut connection_manager: ResMut<ConnectionManager>,
) {
    let (day, _) = crate::progress::today();
    for ship in &ships {
        let VesselPresence::Docked(region) = ship.presence else {
            continue;
        };
        let Some(port) = map
            .0
            .regions()
            .iter()
            .find(|r| r.id == region)
            .and_then(|r| r.port.as_ref())
            .map(|port| port.name)
        else {
            continue;
        };
        if lords.lord_of(port) != Some(ship.character) {
            continue;
        }
        if !logbook.claim_tribute(&store, ship.character, day) {
            continue;
        }
        let item_name = marvyr_domain_economy::guild::payout(port);
        let id = ItemDefinitionId::stable(item_name);
        if dev.catalog.get(id).is_none() {
            continue;
        }
        market.grant_to_storage(ship.character, region, id, TRIBUTE, &dev.catalog);
        info!(character = ?ship.character, port, "tributo de Senhor do Porto");
        if let Some(client) = ship.client_id {
            crate::reputation::send_event(
                &mut connection_manager,
                &[client],
                format!("Senhor do Porto: tributo de {TRIBUTE} {item_name} no armazem"),
                WorldEventKind::Kill,
            );
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn only_ports_outside_crown_waters_are_contested() {
        for seed in [0, 1, 7, 42] {
            let map = WorldMap::from_seed(seed);
            let ports = contested_ports(&map);
            assert!(!ports.is_empty(), "seed {seed}: algum porto em disputa");
            for (name, at) in &ports {
                assert!(!["Porto da Serra", "Porto da Mina"].contains(name));
                assert_eq!(port_near(&map, *at), Some(*name));
            }
        }
        let map = WorldMap::from_seed(0);
        assert_eq!(port_near(&map, Vec2::from(map.features().spawn)), None);
    }
}
