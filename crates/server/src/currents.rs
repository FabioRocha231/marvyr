//! v48: correntes marítimas. Uma faixa rápida por zona, sorteada por (seed,
//! semana) em `domain-world::current`; aqui o servidor guarda as da semana,
//! empurra os navios (em `simulate_movement`) e conta ao client.

use std::collections::HashSet;

use bevy::prelude::*;
use lightyear::prelude::server::*;
use lightyear::prelude::ClientId;
use marvyr_domain_world::current::{currents, Current};
use marvyr_protocol::SeaCurrents as SeaCurrentsWire;
use tracing::info;

use crate::net::{ReliableChannel, ServerShip, ServerWorldMap};
use crate::sets::SimulationSet;

#[derive(Resource, Default)]
pub struct SeaCurrents {
    week: Option<u32>,
    pub lanes: Vec<Current>,
    told: HashSet<ClientId>,
}

impl SeaCurrents {
    /// Empurrão (m/s) no ponto.
    pub fn push_at(&self, x: f32, y: f32) -> Vec2 {
        let (px, py) = marvyr_domain_world::current::push_at(&self.lanes, x, y);
        Vec2::new(px, py)
    }
}

pub fn install(app: &mut App) {
    app.init_resource::<SeaCurrents>().add_systems(
        FixedUpdate,
        (roll, tell).chain().in_set(SimulationSet::Snapshot),
    );
}

fn roll(map: Res<ServerWorldMap>, mut sea: ResMut<SeaCurrents>) {
    let (_, week) = crate::progress::today();
    if sea.week == Some(week) {
        return;
    }
    sea.week = Some(week);
    sea.lanes = currents(&map.0, week);
    sea.told.clear();
    info!(week, lanes = sea.lanes.len(), "correntes da semana");
    for lane in &sea.lanes {
        tracing::debug!(from = ?lane.from, to = ?lane.to, "corrente");
    }
}

fn tell(
    ships: Query<&ServerShip>,
    mut sea: ResMut<SeaCurrents>,
    mut connection_manager: ResMut<ConnectionManager>,
) {
    let wire = SeaCurrentsWire {
        lanes: sea
            .lanes
            .iter()
            .map(|lane| (lane.from.0, lane.from.1, lane.to.0, lane.to.1))
            .collect(),
    };
    let online: HashSet<ClientId> = ships.iter().filter_map(|s| s.client_id).collect();
    for client in online.difference(&sea.told) {
        let _ = connection_manager.send_message::<ReliableChannel, _>(*client, &wire);
    }
    sea.told = online;
}
