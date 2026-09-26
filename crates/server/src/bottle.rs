//! v51: mensagem na garrafa. No mar, o capitão joga uma frase montada de
//! peças prontas (`marvyr_protocol::bottle`); a garrafa deriva com a
//! corrente da semana (ou devagar, fora dela) e outro capitão pesca e lê.
//! Quem jogou ganha um pouco de Renome quando alguém lê. Social assíncrono:
//! o mar tem gente mesmo com o servidor vazio.
//!
//! ponytail: garrafas só em memória — restart as afunda. Persistir quando
//! o mar estiver cheio delas e isso incomodar.

use std::collections::{HashMap, HashSet};

use bevy::prelude::*;
use lightyear::prelude::server::*;
use lightyear::prelude::*;
use marvyr_domain_ships::VesselPresence;
use marvyr_protocol::{ActionKind, BottleRead, BottlesUpdate, PickBottle, ThrowBottle};
use marvyr_shared::ids::CharacterId;
use tracing::info;

use crate::net::{ReliableChannel, ServerShip, ServerWorldMap};
use crate::seafaring::send_action;
use crate::sets::SimulationSet;

/// Uma garrafa por capitão a cada tanto (s).
const THROW_COOLDOWN: f64 = 300.0;
/// Garrafas no mar ao mesmo tempo (as mais velhas afundam).
const MAX_BOTTLES: usize = 200;
/// Afunda sozinha depois de (s).
const LIFETIME: f64 = 24.0 * 3_600.0;
/// Pescar só perto (m) e devagar (m/s).
pub const PICK_RANGE: f32 = 60.0;
const MAX_SPEED: f32 = 3.0;
/// Deriva fora de corrente (m/s): quase parada.
const DRIFT: f32 = 0.3;
pub const READ_RENOWN: u32 = 5;
pub const READ_REASON: &str = "garrafa lida";
const BROADCAST_EVERY: f32 = 5.0;

#[derive(Debug, Clone, Copy)]
pub struct Bottle {
    pub id: u32,
    pub author: CharacterId,
    pub words: [u8; 3],
    pub at: Vec2,
    born: f64,
}

#[derive(Resource, Default)]
pub struct Bottles {
    pub list: Vec<Bottle>,
    next_id: u32,
    last_throw: HashMap<CharacterId, f64>,
    told: HashSet<ClientId>,
    changed: bool,
    clock: f32,
}

impl Bottles {
    /// Joga a garrafa (as mais velhas afundam se o mar lotar).
    pub fn throw(&mut self, author: CharacterId, words: [u8; 3], at: Vec2, now: f64) -> u32 {
        self.next_id += 1;
        self.list.push(Bottle {
            id: self.next_id,
            author,
            words,
            at,
            born: now,
        });
        if self.list.len() > MAX_BOTTLES {
            self.list.remove(0);
        }
        self.last_throw.insert(author, now);
        self.changed = true;
        self.next_id
    }

    /// A garrafa mais perto no alcance que não é do próprio capitão.
    pub fn nearest(&self, picker: CharacterId, at: Vec2) -> Option<usize> {
        self.list
            .iter()
            .enumerate()
            .filter(|(_, b)| b.author != picker && b.at.distance(at) <= PICK_RANGE)
            .min_by(|a, b| a.1.at.distance(at).total_cmp(&b.1.at.distance(at)))
            .map(|(index, _)| index)
    }
}

pub fn install(app: &mut App) {
    app.init_resource::<Bottles>()
        .add_systems(
            FixedUpdate,
            (handle_throw, handle_pick).in_set(SimulationSet::Input),
        )
        .add_systems(
            FixedUpdate,
            (drift, broadcast).chain().in_set(SimulationSet::Snapshot),
        );
}

fn handle_throw(
    time: Res<Time>,
    mut events: EventReader<ServerReceiveMessage<ThrowBottle>>,
    ships: Query<&ServerShip>,
    mut bottles: ResMut<Bottles>,
    mut connection_manager: ResMut<ConnectionManager>,
) {
    let now = time.elapsed_secs_f64();
    for event in events.read() {
        let client_id = event.from();
        let Some(ship) = ships.iter().find(|s| s.client_id == Some(client_id)) else {
            continue;
        };
        let refuse = |cm: &mut ConnectionManager, reason: &str| {
            send_action(cm, client_id, ActionKind::Bottle, false, reason);
        };
        if ship.presence != VesselPresence::AtSea {
            refuse(&mut connection_manager, "Garrafa só se joga no mar.");
            continue;
        }
        let words = event.message().words;
        if marvyr_protocol::bottle::pieces(words).is_none() {
            continue;
        }
        if bottles
            .last_throw
            .get(&ship.character)
            .is_some_and(|at| now - at < THROW_COOLDOWN)
        {
            refuse(&mut connection_manager, "Uma garrafa a cada 5 minutos.");
            continue;
        }
        let at = Vec2::new(ship.motion.x, ship.motion.y);
        let id = bottles.throw(ship.character, words, at, now);
        info!(id, ?words, "garrafa ao mar");
        send_action(
            &mut connection_manager,
            client_id,
            ActionKind::Bottle,
            true,
            "Garrafa ao mar!",
        );
    }
}

// System Bevy: params são injeção de dependência, não assinatura.
#[allow(clippy::too_many_arguments)]
fn handle_pick(
    mut events: EventReader<ServerReceiveMessage<PickBottle>>,
    ships: Query<&ServerShip>,
    mut bottles: ResMut<Bottles>,
    mut renown: EventWriter<crate::renown::RenownEarned>,
    mut discoveries: EventWriter<crate::progress::Discovered>,
    mut connection_manager: ResMut<ConnectionManager>,
) {
    for event in events.read() {
        let client_id = event.from();
        let Some(ship) = ships.iter().find(|s| s.client_id == Some(client_id)) else {
            continue;
        };
        if ship.presence != VesselPresence::AtSea || ship.motion.speed.abs() > MAX_SPEED {
            send_action(
                &mut connection_manager,
                client_id,
                ActionKind::Bottle,
                false,
                "Recolha as velas para pescar a garrafa.",
            );
            continue;
        }
        let at = Vec2::new(ship.motion.x, ship.motion.y);
        let Some(index) = bottles.nearest(ship.character, at) else {
            send_action(
                &mut connection_manager,
                client_id,
                ActionKind::Bottle,
                false,
                "Nenhuma garrafa ao alcance.",
            );
            continue;
        };
        let bottle = bottles.list.remove(index);
        bottles.changed = true;
        renown.send(crate::renown::RenownEarned {
            character: bottle.author,
            amount: READ_RENOWN,
            reason: READ_REASON,
        });
        discoveries.send(crate::progress::Discovered {
            character: ship.character,
            entry: "Garrafa pescada",
        });
        let _ = connection_manager.send_message::<ReliableChannel, _>(
            client_id,
            &BottleRead {
                words: bottle.words,
                author: crate::season::captain_label(bottle.author),
            },
        );
        send_action(
            &mut connection_manager,
            client_id,
            ActionKind::Bottle,
            true,
            "Garrafa pescada!",
        );
    }
}

/// A corrente leva a garrafa; fora dela, deriva devagar para leste. Afunda
/// velha ou na terra.
fn drift(
    time: Res<Time>,
    currents: Res<crate::currents::SeaCurrents>,
    map: Res<ServerWorldMap>,
    mut bottles: ResMut<Bottles>,
) {
    let dt = time.delta_secs();
    let now = time.elapsed_secs_f64();
    let before = bottles.list.len();
    for bottle in &mut bottles.list {
        let push = currents.push_at(bottle.at.x, bottle.at.y);
        let step = if push == Vec2::ZERO {
            Vec2::X * DRIFT
        } else {
            push
        };
        bottle.at += step * dt;
    }
    bottles
        .list
        .retain(|b| now - b.born < LIFETIME && !map.0.is_land(b.at.x, b.at.y));
    if bottles.list.len() != before {
        bottles.changed = true;
    }
}

fn broadcast(
    time: Res<Time>,
    ships: Query<&ServerShip>,
    mut bottles: ResMut<Bottles>,
    mut connection_manager: ResMut<ConnectionManager>,
) {
    bottles.clock += time.delta_secs();
    let online: HashSet<ClientId> = ships.iter().filter_map(|s| s.client_id).collect();
    let fresh = online.difference(&bottles.told).count() > 0;
    // A deriva é lenta: a lista inteira a cada 5 s basta; mudança vai já.
    if !(bottles.changed || fresh || bottles.clock >= BROADCAST_EVERY) {
        return;
    }
    bottles.clock = 0.0;
    bottles.changed = false;
    let update = BottlesUpdate {
        list: bottles
            .list
            .iter()
            .map(|b| (b.id, b.at.x, b.at.y))
            .collect(),
    };
    for client in &online {
        let _ = connection_manager.send_message::<ReliableChannel, _>(*client, &update);
    }
    bottles.told = online;
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn nobody_fishes_their_own_bottle_and_the_sea_has_a_cap() {
        let mut bottles = Bottles::default();
        let author = CharacterId::new();
        let reader = CharacterId::new();
        bottles.throw(author, [0, 0, 0], Vec2::ZERO, 0.0);
        assert_eq!(bottles.nearest(author, Vec2::ZERO), None);
        assert_eq!(bottles.nearest(reader, Vec2::X * 10.0), Some(0));
        assert_eq!(bottles.nearest(reader, Vec2::X * (PICK_RANGE + 1.0)), None);
        for i in 0..MAX_BOTTLES + 3 {
            bottles.throw(author, [0, 0, 0], Vec2::X * i as f32, 0.0);
        }
        assert_eq!(bottles.list.len(), MAX_BOTTLES);
    }
}
