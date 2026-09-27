//! v52: frete entre jogadores (inspirado no courier do EVE). As regras e o
//! escrow vivem no `ServerMarket` (gravados na mesma transação do mercado);
//! aqui a casca: intents de porto, entrega ao atracar, prazos e o quadro.
//! Nada vem de NPC: carga, prêmio e caução são itens de jogador.

use std::collections::HashMap;

use bevy::prelude::*;
use chrono::Utc;
use lightyear::prelude::server::*;
use lightyear::prelude::*;
use marvyr_domain_ships::VesselPresence;
use marvyr_protocol::{
    AcceptFreight, CancelFreight, FreightBoard, FreightLine, PostFreight, WorldEventKind,
};
use marvyr_shared::ids::CharacterId;
use tracing::info;

use crate::market::{market_result, region_name, ServerMarket};
use crate::net::{DevItems, ReliableChannel, ServerShip, ServerWorldMap};
use crate::persist::StoreHandle;
use crate::sets::SimulationSet;

pub const DELIVERY_RENOWN: u32 = 25;
pub const DELIVERY_REASON: &str = "frete entregue";
/// Entregas por dia que rendem Renome a um transportador (contas-sombra
/// trocando fretes de 1 unidade não viram fazenda).
pub const RENOWN_DELIVERIES_PER_DAY: u32 = 4;
/// Anunciar, aceitar ou cancelar grava o mercado: um por client a cada (s).
const INTENT_COOLDOWN: f64 = 1.0;
const TOO_FAST: &str = "Devagar no quadro de fretes.";
const BOARD_EVERY: f32 = 2.0;
const EXPIRE_EVERY: f32 = 5.0;

pub fn install(app: &mut App) {
    app.add_systems(
        FixedUpdate,
        (handle_post, handle_accept, handle_cancel).in_set(SimulationSet::Input),
    )
    .add_systems(
        FixedUpdate,
        (deliver_on_dock, expire).in_set(SimulationSet::EconomyConsequences),
    )
    .add_systems(FixedUpdate, send_boards.in_set(SimulationSet::Snapshot));
}

// System Bevy: params são injeção de dependência, não assinatura.
#[allow(clippy::too_many_arguments)]
fn handle_post(
    time: Res<Time>,
    mut events: EventReader<ServerReceiveMessage<PostFreight>>,
    ships: Query<&ServerShip>,
    map: Res<ServerWorldMap>,
    dev: Res<DevItems>,
    mut market: ResMut<ServerMarket>,
    mut connection_manager: ResMut<ConnectionManager>,
    mut last: Local<HashMap<ClientId, f64>>,
) {
    for event in events.read() {
        let client_id = event.from();
        let Some(ship) = ships.iter().find(|s| s.client_id == Some(client_id)) else {
            continue;
        };
        if !crate::session::intent_ready(
            &mut last,
            client_id,
            time.elapsed_secs_f64(),
            INTENT_COOLDOWN,
        ) {
            market_result(&mut connection_manager, client_id, false, TOO_FAST);
            continue;
        }
        let VesselPresence::Docked(origin) = ship.presence else {
            market_result(
                &mut connection_manager,
                client_id,
                false,
                "Atraque para anunciar frete.",
            );
            continue;
        };
        let post = event.message();
        let Some(dest) = map
            .0
            .regions()
            .iter()
            .find(|r| r.port.is_some() && r.name == post.dest)
            .map(|r| r.id)
        else {
            market_result(
                &mut connection_manager,
                client_id,
                false,
                "Porto de destino desconhecido.",
            );
            continue;
        };
        match market.post_freight(
            ship.character,
            origin,
            dest,
            (post.cargo_item, post.cargo_qty),
            (post.reward_item, post.reward_qty),
            post.collateral,
            &dev.catalog,
            Utc::now(),
        ) {
            Ok(num) => {
                info!(num, dest = %post.dest, "frete anunciado");
                market_result(
                    &mut connection_manager,
                    client_id,
                    true,
                    &format!("Frete #{num} anunciado para {}", post.dest),
                );
            }
            Err(reason) => market_result(&mut connection_manager, client_id, false, reason),
        }
    }
}

// System Bevy: params são injeção de dependência, não assinatura.
#[allow(clippy::too_many_arguments)]
fn handle_accept(
    time: Res<Time>,
    mut events: EventReader<ServerReceiveMessage<AcceptFreight>>,
    mut ships: Query<&mut ServerShip>,
    dev: Res<DevItems>,
    store: Res<StoreHandle>,
    mut market: ResMut<ServerMarket>,
    mut connection_manager: ResMut<ConnectionManager>,
    mut last: Local<HashMap<ClientId, f64>>,
) {
    for event in events.read() {
        let client_id = event.from();
        let Some(mut ship) = ships.iter_mut().find(|s| s.client_id == Some(client_id)) else {
            continue;
        };
        if !crate::session::intent_ready(
            &mut last,
            client_id,
            time.elapsed_secs_f64(),
            INTENT_COOLDOWN,
        ) {
            market_result(&mut connection_manager, client_id, false, TOO_FAST);
            continue;
        }
        let VesselPresence::Docked(region) = ship.presence else {
            market_result(
                &mut connection_manager,
                client_id,
                false,
                "Atraque para aceitar frete.",
            );
            continue;
        };
        let num = event.message().num;
        let character = ship.character;
        match market.accept_freight(
            character,
            region,
            num,
            &mut ship.hold,
            &dev.catalog,
            Utc::now(),
        ) {
            Ok(()) => {
                crate::net::save_ship_now(&store, &ship);
                info!(num, "frete aceito");
                market_result(
                    &mut connection_manager,
                    client_id,
                    true,
                    &format!("Frete #{num} no porão: entregue em até 2h"),
                );
            }
            Err(reason) => market_result(&mut connection_manager, client_id, false, reason),
        }
    }
}

fn handle_cancel(
    time: Res<Time>,
    mut events: EventReader<ServerReceiveMessage<CancelFreight>>,
    ships: Query<&ServerShip>,
    dev: Res<DevItems>,
    mut market: ResMut<ServerMarket>,
    mut connection_manager: ResMut<ConnectionManager>,
    mut last: Local<HashMap<ClientId, f64>>,
) {
    for event in events.read() {
        let client_id = event.from();
        let Some(ship) = ships.iter().find(|s| s.client_id == Some(client_id)) else {
            continue;
        };
        if !crate::session::intent_ready(
            &mut last,
            client_id,
            time.elapsed_secs_f64(),
            INTENT_COOLDOWN,
        ) {
            market_result(&mut connection_manager, client_id, false, TOO_FAST);
            continue;
        }
        let VesselPresence::Docked(region) = ship.presence else {
            market_result(
                &mut connection_manager,
                client_id,
                false,
                "Atraque para cancelar.",
            );
            continue;
        };
        let num = event.message().num;
        match market.cancel_freight(ship.character, region, num, &dev.catalog) {
            Ok(()) => market_result(
                &mut connection_manager,
                client_id,
                true,
                &format!("Frete #{num} cancelado: carga e prêmio no armazém"),
            ),
            Err(reason) => market_result(&mut connection_manager, client_id, false, reason),
        }
    }
}

fn notify(
    connection_manager: &mut ConnectionManager,
    ships: &Query<&mut ServerShip>,
    character: CharacterId,
    text: String,
) {
    if let Some(client) = ships
        .iter()
        .find(|s| s.character == character)
        .and_then(|s| s.client_id)
    {
        crate::reputation::send_event(connection_manager, &[client], text, WorldEventKind::Kill);
    }
}

// System Bevy: params são injeção de dependência, não assinatura.
#[allow(clippy::too_many_arguments)]
fn deliver_on_dock(
    mut ships: Query<&mut ServerShip>,
    dev: Res<DevItems>,
    map: Res<ServerWorldMap>,
    store: Res<StoreHandle>,
    mut market: ResMut<ServerMarket>,
    mut renown: EventWriter<crate::renown::RenownEarned>,
    mut discoveries: EventWriter<crate::progress::Discovered>,
    mut connection_manager: ResMut<ConnectionManager>,
    mut paid_today: Local<HashMap<(CharacterId, u32), u32>>,
) {
    let mut done = Vec::new();
    for mut ship in &mut ships {
        let VesselPresence::Docked(region) = ship.presence else {
            continue;
        };
        let character = ship.character;
        if !market
            .freights()
            .iter()
            .any(|f| f.courier == Some(character) && f.dest == region)
        {
            continue;
        }
        let delivered = market.deliver_freights(character, region, &mut ship.hold, &dev.catalog);
        if delivered.is_empty() {
            continue;
        }
        crate::net::save_ship_now(&store, &ship);
        for freight in delivered {
            done.push((character, region, freight));
        }
    }
    let (day, _) = crate::progress::today();
    paid_today.retain(|(_, d), _| *d == day);
    for (courier, region, freight) in done {
        info!(num = freight.num, "frete entregue");
        let paid = paid_today.entry((courier, day)).or_default();
        if *paid < RENOWN_DELIVERIES_PER_DAY {
            *paid += 1;
            renown.send(crate::renown::RenownEarned {
                character: courier,
                amount: DELIVERY_RENOWN,
                reason: DELIVERY_REASON,
            });
        }
        discoveries.send(crate::progress::Discovered {
            character: courier,
            entry: "Frete entregue",
        });
        let port = region_name(&map.0, region);
        notify(
            &mut connection_manager,
            &ships,
            courier,
            format!(
                "Frete #{} entregue em {port}: prêmio e caução no armazém",
                freight.num
            ),
        );
        notify(
            &mut connection_manager,
            &ships,
            freight.poster,
            format!(
                "Seu frete #{} chegou em {port}: a carga está no armazém de lá",
                freight.num
            ),
        );
    }
}

fn expire(
    time: Res<Time>,
    ships: Query<&mut ServerShip>,
    dev: Res<DevItems>,
    mut market: ResMut<ServerMarket>,
    mut connection_manager: ResMut<ConnectionManager>,
    mut clock: Local<f32>,
) {
    *clock += time.delta_secs();
    if *clock < EXPIRE_EVERY {
        return;
    }
    *clock = 0.0;
    for freight in market.expire_freights(Utc::now(), &dev.catalog) {
        info!(
            num = freight.num,
            carried = freight.courier.is_some(),
            "frete venceu"
        );
        let Some(courier) = freight.courier else {
            notify(
                &mut connection_manager,
                &ships,
                freight.poster,
                format!(
                    "Ninguém levou o frete #{}: carga e prêmio voltaram",
                    freight.num
                ),
            );
            continue;
        };
        notify(
            &mut connection_manager,
            &ships,
            freight.poster,
            format!(
                "O frete #{} não chegou: prêmio e caução são seus",
                freight.num
            ),
        );
        notify(
            &mut connection_manager,
            &ships,
            courier,
            format!(
                "Prazo do frete #{} venceu: a caução ficou com o dono",
                freight.num
            ),
        );
    }
}

/// Quadro para cada capitão atracado, a cada 2 s.
fn send_boards(
    time: Res<Time>,
    ships: Query<&ServerShip>,
    map: Res<ServerWorldMap>,
    dev: Res<DevItems>,
    market: Res<ServerMarket>,
    mut connection_manager: ResMut<ConnectionManager>,
    mut clock: Local<f32>,
) {
    *clock += time.delta_secs();
    if *clock < BOARD_EVERY {
        return;
    }
    *clock = 0.0;
    let name = |item| {
        dev.catalog
            .get(item)
            .map_or_else(|| String::from("?"), |d| d.display_name.clone())
    };
    let ports: Vec<String> = map
        .0
        .regions()
        .iter()
        .filter(|r| r.port.is_some())
        .map(|r| r.name.to_owned())
        .collect();
    let now = Utc::now();
    for ship in &ships {
        let (Some(client), VesselPresence::Docked(region)) = (ship.client_id, ship.presence) else {
            continue;
        };
        let me = ship.character;
        let lines = market
            .freights()
            .iter()
            .filter(|f| {
                f.poster == me
                    || f.courier == Some(me)
                    || (f.courier.is_none() && f.origin == region)
            })
            .map(|f| FreightLine {
                num: f.num,
                origin: region_name(&map.0, f.origin).to_owned(),
                dest: region_name(&map.0, f.dest).to_owned(),
                cargo_item: name(f.cargo.0),
                cargo_qty: f.cargo.1,
                reward_item: name(f.reward.0),
                reward_qty: f.reward.1,
                collateral: f.collateral,
                poster: crate::season::captain_label(f.poster),
                mine: f.poster == me,
                carrying: f.courier == Some(me),
                in_transit: f.courier.is_some(),
                minutes_left: u32::try_from((f.expires_at - now).num_minutes().max(0)).unwrap_or(0),
            })
            .collect();
        let _ = connection_manager.send_message::<ReliableChannel, _>(
            client,
            &FreightBoard {
                lines,
                ports: ports.clone(),
            },
        );
    }
}
