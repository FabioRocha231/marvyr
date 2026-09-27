//! Quem é quem (v55): o nome do capitão sai do hello (conta ou sufixo do
//! token anônimo) e vai para todos a cada 2 s, por navio. Só apresentação:
//! nenhuma regra olha o nome.

use std::collections::{HashMap, HashSet};

use bevy::prelude::*;
use lightyear::prelude::server::*;
use lightyear::prelude::*;
use marvyr_protocol::{ClientHello, ShipNames, TakeoverRequest};

use crate::net::{ReliableChannel, ServerShip};

/// A cada quantos segundos a lista de nomes vai para todos.
const NAMES_EVERY_SECS: f32 = 2.0;

/// Nome do capitão por sessão.
#[derive(Resource, Default)]
pub struct CaptainNames(pub HashMap<ClientId, String>);

impl CaptainNames {
    /// Nome do capitão deste navio (sem sessão: nome genérico).
    pub fn of(&self, ship: &ServerShip) -> String {
        ship.client_id
            .and_then(|client| self.0.get(&client).cloned())
            .unwrap_or_else(|| String::from("Capitão"))
    }
}

/// v55: sessões que pediram para derrubar a outra sessão do mesmo capitão
/// (o `handle_hello` consome).
#[derive(Resource, Default)]
pub struct Takeovers(pub HashSet<ClientId>);

pub fn install(app: &mut App) {
    app.init_resource::<CaptainNames>()
        .init_resource::<Takeovers>();
}

/// O pedido chega antes do hello no mesmo canal ordenado.
pub fn note_takeovers(
    mut requests: EventReader<ServerReceiveMessage<TakeoverRequest>>,
    mut takeovers: ResMut<Takeovers>,
) {
    for event in requests.read() {
        takeovers.0.insert(event.from());
    }
}

/// Lê os hellos de novo (cursor próprio) só para guardar o nome. A
/// validação de verdade é a do `handle_hello`: hello recusado não tem
/// navio, então o nome guardado nunca aparece.
pub fn record_names(
    mut hellos: EventReader<ServerReceiveMessage<ClientHello>>,
    auth: Res<crate::session::AuthConfig>,
    mut names: ResMut<CaptainNames>,
) {
    for event in hellos.read() {
        if let Ok(identity) = crate::session::resolve_identity(&event.message().identity, &auth) {
            names.0.insert(event.from(), identity.display_name);
        }
    }
}

/// Sessão que caiu esquece o nome (o próximo hello grava de novo).
pub fn forget_names(
    mut disconnects: EventReader<DisconnectEvent>,
    mut names: ResMut<CaptainNames>,
    mut takeovers: ResMut<Takeovers>,
) {
    for event in disconnects.read() {
        names.0.remove(&event.client_id);
        takeovers.0.remove(&event.client_id);
    }
}

pub fn broadcast_names(
    time: Res<Time>,
    mut clock: Local<f32>,
    names: Res<CaptainNames>,
    companies: Res<crate::companies::Companies>,
    ships: Query<&ServerShip>,
    mut connection_manager: ResMut<ConnectionManager>,
) {
    *clock += time.delta_secs();
    if *clock < NAMES_EVERY_SECS {
        return;
    }
    *clock = 0.0;
    let online: Vec<ClientId> = ships.iter().filter_map(|ship| ship.client_id).collect();
    let message = ShipNames {
        names: ships
            .iter()
            // v62: a tag da companhia vai na frente do nome.
            .map(|ship| {
                let name = names.of(ship);
                let name = match companies.company_of(ship.character) {
                    Some(company) => format!("[{}] {name}", company.tag),
                    None => name,
                };
                (ship.ship_id, name)
            })
            .collect(),
    };
    for client in online {
        let _ = connection_manager.send_message::<ReliableChannel, _>(client, &message);
    }
}
