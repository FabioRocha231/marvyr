//! Guilda Mercante (NPC de câmbio) e Quadro de Contratos por porto. As
//! regras vivem em `domain-economy` (guild.rs, contract.rs); aqui é casca:
//! valida atracado, move item pelo storage do `ServerMarket` (persistido
//! igual ao mercado) e empurra snapshots ao client.
//!
//! ponytail: saturação da guilda e contratos vivem só em memória — restart
//! zera preços e contratos ativos; persistir quando virar reclamação.

use std::collections::{HashMap, HashSet};

use bevy::ecs::prelude::*;
use bevy::time::Time;
use lightyear::prelude::server::*;
use lightyear::prelude::*;
use marvyr_domain_economy::contract::OFFERS_PER_PORT;
use marvyr_domain_economy::guild::{payout, GUILD_BASE_VALUES};
use marvyr_domain_economy::{
    generate_offers, ActiveContract, Contract, ContractKind, GuildBook, HuntingGround, PortSite,
};
use marvyr_domain_items::ItemCatalog;
use marvyr_domain_ships::VesselPresence;
use marvyr_domain_world::{RiskTier, WorldMap};
use marvyr_protocol::{
    AbandonContract, AcceptContract, ContractLine, ContractResult, ContractsSnapshot,
    GuildPriceLine, GuildPrices, PortStorageSnapshot, SellToGuild,
};
use marvyr_shared::ids::{CharacterId, ItemDefinitionId, ItemInstanceId, RegionId};
use tracing::info;

use crate::market::{market_result, region_name, ServerMarket};
use crate::net::{port_storage_snapshot, DevItems, ReliableChannel, ServerShip, ServerWorldMap};
use crate::sets::SimulationSet;

/// Intervalo de renovação do Quadro de Contratos.
const BOARD_REFRESH_SECS: f64 = 300.0;

pub struct GuildPlugin;

impl bevy::app::Plugin for GuildPlugin {
    fn build(&self, app: &mut bevy::app::App) {
        app.init_resource::<ServerGuild>();
        app.add_systems(
            bevy::app::FixedUpdate,
            (handle_sell_to_guild, handle_contract_intents).in_set(SimulationSet::Input),
        );
        app.add_systems(
            bevy::app::FixedUpdate,
            tick_contracts.in_set(SimulationSet::EconomyConsequences),
        );
        app.add_systems(
            bevy::app::FixedUpdate,
            push_guild_state.in_set(SimulationSet::Snapshot),
        );
    }
}

#[derive(Resource, Default)]
pub struct ServerGuild {
    pub book: GuildBook,
    boards: HashMap<RegionId, Vec<Contract>>,
    active: HashMap<CharacterId, ActiveContract>,
    /// Quadro de onde saiu o contrato ativo: abandonar devolve a oferta, senão
    /// aceitar+abandonar em loop esvazia o quadro de todo mundo.
    accepted_at: HashMap<CharacterId, RegionId>,
    next_refresh_secs: f64,
    seed: u64,
    next_id: u32,
}

/// Portos do mapa como (região, sítio) — nome da região = nome do porto.
/// Posição na carta de zonas (MV-066): a distância do contrato é a de
/// viagem, não a do plano onde as zonas moram longe umas das outras.
fn port_sites(map: &WorldMap) -> Vec<(RegionId, PortSite<'static>)> {
    map.regions()
        .iter()
        .filter_map(|region| {
            let port = region.port.as_ref()?;
            let (x, y) = map.chart_position(port.x, port.y);
            Some((
                region.id,
                PortSite {
                    name: region.name,
                    x,
                    y,
                },
            ))
        })
        .collect()
}

/// Zonas com pirata ou saqueador: onde as Caçadas mandam o capitão.
fn hunting_grounds(map: &WorldMap) -> Vec<HuntingGround<'static>> {
    let features = map.features();
    features
        .areas
        .iter()
        .enumerate()
        .filter(|(index, _)| {
            features
                .pirate_spawns
                .iter()
                .chain(&features.raider_spawns)
                .any(|&(x, y)| map.area_at(x, y) == Some(*index))
        })
        .map(|(_, area)| HuntingGround {
            name: area.name,
            lawless: area.tier == RiskTier::Lawless,
        })
        .collect()
}

impl ServerGuild {
    /// Renova as 3 ofertas de cada porto (as não aceitas somem).
    fn refresh_boards(&mut self, map: &WorldMap) {
        if self.seed == 0 {
            self.seed = std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .map(|elapsed| elapsed.as_nanos() as u64)
                .unwrap_or(1)
                | 1;
        }
        let ports = port_sites(map);
        let sites: Vec<PortSite> = ports.iter().map(|(_, site)| *site).collect();
        let grounds = hunting_grounds(map);
        for (region, site) in &ports {
            self.seed = self
                .seed
                .wrapping_mul(0x9E37_79B9_7F4A_7C15)
                .wrapping_add(1);
            let offers = generate_offers(site, &sites, &grounds, self.seed, self.next_id);
            self.next_id += offers.len() as u32;
            self.boards.insert(*region, offers);
        }
    }
}

fn contract_line(contract: &Contract, active: Option<(&ActiveContract, f64)>) -> ContractLine {
    ContractLine {
        id: contract.id,
        title: contract.title(),
        reward_item: contract.reward_item.to_owned(),
        reward_quantity: contract.reward_quantity,
        duration_secs: contract.duration_secs as u32,
        remaining_secs: active
            .map(|(active, now)| active.remaining_secs(now).ceil() as u32)
            .unwrap_or(0),
        progress: active.map(|(active, _)| active.progress()).unwrap_or(0),
        target: contract.target(),
        hunt: matches!(contract.kind, ContractKind::Hunt { .. }),
    }
}

fn catalog_id(catalog: &ItemCatalog, name: &str) -> Option<ItemDefinitionId> {
    catalog
        .items()
        .find(|definition| definition.display_name == name)
        .map(|definition| definition.id)
}

fn send_contract_result(
    connection_manager: &mut ConnectionManager,
    client_id: Option<ClientId>,
    success: bool,
    reason: String,
) {
    if let Some(client_id) = client_id {
        let _ = connection_manager
            .send_message::<ReliableChannel, _>(client_id, &ContractResult { success, reason });
    }
}

/// Troca do storage do porto atracado com a guilda, que paga com o recurso
/// do porto. Fail-closed: item fora da tabela ou sem estoque é recusado e
/// nada se move.
#[allow(clippy::too_many_arguments)]
fn handle_sell_to_guild(
    mut events: EventReader<ServerReceiveMessage<SellToGuild>>,
    mut connection_manager: ResMut<ConnectionManager>,
    mut market: ResMut<ServerMarket>,
    mut guild: ResMut<ServerGuild>,
    dev: Res<DevItems>,
    map: Res<ServerWorldMap>,
    time: Res<Time>,
    ships: Query<&ServerShip>,
) {
    let now = time.elapsed_secs_f64();
    for event in events.read() {
        let client_id = event.from();
        let message = event.message();
        let Some(ship) = ships.iter().find(|ship| ship.client_id == Some(client_id)) else {
            continue;
        };
        let VesselPresence::Docked(region) = ship.presence else {
            market_result(
                &mut connection_manager,
                client_id,
                false,
                "atraca primeiro (E)",
            );
            continue;
        };
        let Some(name) = dev
            .catalog
            .get(message.item)
            .map(|definition| definition.display_name.clone())
        else {
            market_result(
                &mut connection_manager,
                client_id,
                false,
                "item desconhecido",
            );
            continue;
        };
        let port = region_name(&map.0, region);
        let available = market.storage_quantity(ship.character, region, message.item);
        let quantity = message.quantity.min(available);
        if quantity == 0 {
            market_result(
                &mut connection_manager,
                client_id,
                false,
                "nada disso no armazem",
            );
            continue;
        }
        let receive_name = payout(port);
        let (Some(paid), Some(receive)) = (
            guild.book.exchange_quote(port, &name, quantity, now),
            catalog_id(&dev.catalog, receive_name),
        ) else {
            market_result(
                &mut connection_manager,
                client_id,
                false,
                &format!("a guilda daqui nao aceita {name}"),
            );
            continue;
        };
        if paid == 0 {
            market_result(
                &mut connection_manager,
                client_id,
                false,
                &format!("pouco demais para valer 1 {receive_name}"),
            );
            continue;
        }
        if let Err(error) = market.exchange_with_guild(
            ship.character,
            region,
            message.item,
            quantity,
            receive,
            paid,
            &dev.catalog,
        ) {
            market_result(
                &mut connection_manager,
                client_id,
                false,
                &error.to_string(),
            );
            continue;
        }
        guild.book.record_sale(port, &name, quantity, now);
        info!(port, item = %name, quantity, receive = receive_name, paid, "guilda trocou; item destruído");
        market_result(
            &mut connection_manager,
            client_id,
            true,
            &format!("Guilda trocou {quantity} {name} por {paid} {receive_name}"),
        );
    }
}

/// Aceitar (só atracado, 1 por vez) e abandonar (em qualquer lugar).
fn handle_contract_intents(
    mut accepts: EventReader<ServerReceiveMessage<AcceptContract>>,
    mut abandons: EventReader<ServerReceiveMessage<AbandonContract>>,
    mut connection_manager: ResMut<ConnectionManager>,
    mut guild: ResMut<ServerGuild>,
    dev: Res<DevItems>,
    time: Res<Time>,
    ships: Query<&ServerShip>,
) {
    let now = time.elapsed_secs_f64();
    for event in accepts.read() {
        let client_id = Some(event.from());
        let Some(ship) = ships.iter().find(|ship| ship.client_id == client_id) else {
            continue;
        };
        let VesselPresence::Docked(region) = ship.presence else {
            send_contract_result(
                &mut connection_manager,
                client_id,
                false,
                "atraca primeiro (E)".into(),
            );
            continue;
        };
        if guild.active.contains_key(&ship.character) {
            send_contract_result(
                &mut connection_manager,
                client_id,
                false,
                "ja existe um contrato ativo".into(),
            );
            continue;
        }
        let id = event.message().id;
        let offer = guild
            .boards
            .get(&region)
            .and_then(|board| board.iter().position(|contract| contract.id == id));
        let Some(index) = offer else {
            send_contract_result(
                &mut connection_manager,
                client_id,
                false,
                "contrato indisponivel".into(),
            );
            continue;
        };
        let contract = guild.boards[&region][index].clone();
        let hold = match &contract.kind {
            ContractKind::Delivery { item, .. } => hold_stacks(ship, &dev.catalog, item),
            ContractKind::Hunt { .. } => Vec::new(),
        };
        let Some(active) = ActiveContract::accept(contract, now, &hold) else {
            send_contract_result(
                &mut connection_manager,
                client_id,
                false,
                "carregue a carga no porao antes de aceitar".into(),
            );
            continue;
        };
        if let Some(board) = guild.boards.get_mut(&region) {
            board.remove(index);
        }
        let reason = format!("aceito: {}", active.contract.title());
        guild.active.insert(ship.character, active);
        guild.accepted_at.insert(ship.character, region);
        send_contract_result(&mut connection_manager, client_id, true, reason);
    }
    for event in abandons.read() {
        let client_id = Some(event.from());
        let Some(ship) = ships.iter().find(|ship| ship.client_id == client_id) else {
            continue;
        };
        let abandoned = guild.active.remove(&ship.character);
        let origin = guild.accepted_at.remove(&ship.character);
        if let (Some(active), Some(region)) = (&abandoned, origin) {
            // Oferta volta ao quadro, sem passar do tamanho de um quadro novo.
            if let Some(board) = guild.boards.get_mut(&region) {
                if board.len() < OFFERS_PER_PORT {
                    board.push(active.contract.clone());
                }
            }
        }
        let abandoned = abandoned.is_some();
        let reason = if abandoned {
            "contrato abandonado"
        } else {
            "nenhum contrato ativo"
        };
        send_contract_result(&mut connection_manager, client_id, abandoned, reason.into());
    }
}

/// Pilhas de `item` no porão do navio: (instância, quantidade).
fn hold_stacks(ship: &ServerShip, catalog: &ItemCatalog, item: &str) -> Vec<(ItemInstanceId, u32)> {
    let Some(item_id) = catalog_id(catalog, item) else {
        return Vec::new();
    };
    ship.hold
        .items()
        .iter()
        .filter(|custody| custody.instance.definition == item_id)
        .map(|custody| (custody.instance.id, custody.instance.quantity))
        .collect()
}

/// Paga a recompensa em recurso bruto no armazém de `region` (pilar 1: NPC
/// não dá item útil).
fn pay_contract(
    market: &mut ServerMarket,
    catalog: &ItemCatalog,
    character: CharacterId,
    region: RegionId,
    contract: &Contract,
) {
    if let Some(item) = catalog_id(catalog, contract.reward_item) {
        market.grant_to_storage(character, region, item, contract.reward_quantity, catalog);
    }
}

/// Renova quadros, conta abates de Caça, expira prazos e conclui entregas
/// de quem está atracado no destino com a carga no porão.
// System Bevy: params são injeção de dependência, não assinatura.
#[allow(clippy::too_many_arguments)]
fn tick_contracts(
    mut connection_manager: ResMut<ConnectionManager>,
    mut market: ResMut<ServerMarket>,
    mut guild: ResMut<ServerGuild>,
    dev: Res<DevItems>,
    map: Res<ServerWorldMap>,
    time: Res<Time>,
    mut ships: Query<&mut ServerShip>,
    mut renown: EventWriter<crate::renown::RenownEarned>,
) {
    let now = time.elapsed_secs_f64();
    if now >= guild.next_refresh_secs {
        guild.refresh_boards(&map.0);
        guild.next_refresh_secs = now + BOARD_REFRESH_SECS;
    }
    let viewers: Vec<(Option<ClientId>, CharacterId)> = ships
        .iter()
        .map(|ship| (ship.client_id, ship.character))
        .collect();
    let client_of = |character: CharacterId| {
        viewers
            .iter()
            .find(|(_, owner)| *owner == character)
            .and_then(|(client_id, _)| *client_id)
    };
    // (quem, porto onde recebe, contrato): a Caça paga no porto do aceite,
    // a Entrega no destino.
    let mut completed: Vec<(CharacterId, RegionId, Contract)> = Vec::new();

    for (killer, sunk_in) in std::mem::take(&mut market.npc_kills) {
        if let Some(active) = guild.active.get_mut(&killer) {
            if !active.expired(now) && active.record_kill(sunk_in) {
                let active = guild.active.remove(&killer).expect("checado acima");
                if let Some(region) = guild.accepted_at.remove(&killer) {
                    completed.push((killer, region, active.contract));
                }
            }
        }
    }

    let expired: Vec<CharacterId> = guild
        .active
        .iter()
        .filter(|(_, active)| active.expired(now))
        .map(|(character, _)| *character)
        .collect();
    for character in expired {
        guild.active.remove(&character);
        guild.accepted_at.remove(&character);
        send_contract_result(
            &mut connection_manager,
            client_of(character),
            false,
            "contrato expirou".into(),
        );
    }

    for mut ship in &mut ships {
        // Todo tick, em qualquer lugar: a consignação só pode encolher.
        if let Some(active) = guild.active.get_mut(&ship.character) {
            if let ContractKind::Delivery { item, .. } = &active.contract.kind {
                let hold = hold_stacks(&ship, &dev.catalog, item);
                active.observe_hold(&hold);
            }
        }
        let VesselPresence::Docked(region) = ship.presence else {
            continue;
        };
        let Some(active) = guild.active.get(&ship.character) else {
            continue;
        };
        let ContractKind::Delivery { item, quantity, .. } = &active.contract.kind else {
            continue;
        };
        let Some(item_id) = catalog_id(&dev.catalog, item) else {
            continue;
        };
        let hold = hold_stacks(&ship, &dev.catalog, item);
        if !active.delivery_ready(region_name(&map.0, region), &hold) {
            continue;
        }
        let quantity = *quantity;
        // Entregue: a carga some do porão (sink) e o contrato paga.
        if ship.hold.remove(item_id, quantity).is_ok() {
            let active = guild.active.remove(&ship.character).expect("checado acima");
            guild.accepted_at.remove(&ship.character);
            completed.push((ship.character, region, active.contract));
        }
    }

    for (character, region, contract) in completed {
        pay_contract(&mut market, &dev.catalog, character, region, &contract);
        renown.send(crate::renown::RenownEarned {
            character,
            amount: marvyr_domain_economy::renown::PER_CONTRACT,
            reason: "contrato entregue",
        });
        info!(
            contract = contract.id,
            reward = contract.reward_quantity,
            item = contract.reward_item,
            "contrato concluído"
        );
        send_contract_result(
            &mut connection_manager,
            client_of(character),
            true,
            format!(
                "contrato concluido: +{} {} no armazem de {}",
                contract.reward_quantity,
                contract.reward_item,
                region_name(&map.0, region)
            ),
        );
    }
}

/// Câmbio da guilda em todos os portos + carga do porão (só leitura).
fn guild_prices(
    guild: &ServerGuild,
    catalog: &ItemCatalog,
    map: &WorldMap,
    ship: &ServerShip,
    now: f64,
) -> GuildPrices {
    let ports = port_sites(map);
    GuildPrices {
        ports: ports.iter().map(|(_, site)| site.name.to_owned()).collect(),
        payouts: ports
            .iter()
            .map(|(_, site)| payout(site.name).to_owned())
            .collect(),
        lines: GUILD_BASE_VALUES
            .iter()
            .filter_map(|(name, _)| {
                Some(GuildPriceLine {
                    item: catalog_id(catalog, name)?,
                    item_name: (*name).to_owned(),
                    per_ten: ports
                        .iter()
                        .map(|(_, site)| {
                            guild
                                .book
                                .exchange_quote(site.name, name, 10, now)
                                .unwrap_or(0)
                        })
                        .collect(),
                })
            })
            .collect(),
        cargo: port_storage_snapshot(catalog, "", ship.hold.items()).lines,
    }
}

fn contracts_snapshot(guild: &ServerGuild, ship: &ServerShip, now: f64) -> ContractsSnapshot {
    let offers = match ship.presence {
        VesselPresence::Docked(region) => guild
            .boards
            .get(&region)
            .map(|board| board.iter().map(|c| contract_line(c, None)).collect())
            .unwrap_or_default(),
        VesselPresence::AtSea => Vec::new(),
    };
    ContractsSnapshot {
        offers,
        active: guild
            .active
            .get(&ship.character)
            .map(|active| contract_line(&active.contract, Some((active, now)))),
    }
}

#[derive(Default)]
struct SentState {
    prices: Option<GuildPrices>,
    storage: Option<PortStorageSnapshot>,
    /// Snapshot com `remaining_secs` zerado: o client faz a contagem.
    contracts: Option<ContractsSnapshot>,
}

/// Envia a cada client só o que mudou: preços (mudança de 1g já conta),
/// storage do porto (depósito, craft, venda) e contratos.
#[allow(clippy::too_many_arguments)]
fn push_guild_state(
    mut connection_manager: ResMut<ConnectionManager>,
    market: Res<ServerMarket>,
    guild: Res<ServerGuild>,
    dev: Res<DevItems>,
    map: Res<ServerWorldMap>,
    time: Res<Time>,
    ships: Query<&ServerShip>,
    mut sent: Local<HashMap<ClientId, SentState>>,
) {
    let now = time.elapsed_secs_f64();
    let online: HashSet<ClientId> = ships.iter().filter_map(|ship| ship.client_id).collect();
    sent.retain(|client_id, _| online.contains(client_id));
    for ship in &ships {
        let Some(client_id) = ship.client_id else {
            continue;
        };
        let state = sent.entry(client_id).or_default();

        let contracts = contracts_snapshot(&guild, ship, now);
        let mut key = contracts.clone();
        if let Some(active) = key.active.as_mut() {
            active.remaining_secs = 0;
        }
        if state.contracts.as_ref() != Some(&key) {
            let _ = connection_manager.send_message::<ReliableChannel, _>(client_id, &contracts);
            state.contracts = Some(key);
        }

        let VesselPresence::Docked(region) = ship.presence else {
            state.prices = None;
            state.storage = None;
            continue;
        };
        let prices = guild_prices(&guild, &dev.catalog, &map.0, ship, now);
        if state.prices.as_ref() != Some(&prices) {
            let _ = connection_manager.send_message::<ReliableChannel, _>(client_id, &prices);
            state.prices = Some(prices);
        }
        let storage = crate::net::port_storage_with_elsewhere(
            &dev.catalog,
            &map.0,
            &market,
            ship.character,
            region,
        );
        if state.storage.as_ref() != Some(&storage) {
            let _ = connection_manager.send_message::<ReliableChannel, _>(client_id, &storage);
            state.storage = Some(storage);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn hunts_go_where_hostiles_spawn_never_to_protected_bays() {
        let map = WorldMap::from_seed(crate::net::DEFAULT_WORLD_SEED);
        let grounds = hunting_grounds(&map);
        assert!(grounds.len() >= 3, "{grounds:?}");
        assert!(grounds.iter().any(|ground| ground.lawless));
        for ground in &grounds {
            let area = map
                .features()
                .areas
                .iter()
                .find(|area| area.name == ground.name)
                .expect("zona existe");
            assert_ne!(area.tier, RiskTier::Protected, "{}", ground.name);
        }
    }
}
