//! Mercado regional no servidor (PRD §43-47, MF-023..026). Sem moeda.
//!
//! Regras do slice:
//! - Storage é separado por `RegionId`; não existe GlobalStorage (§30).
//! - Oferta de escambo move item do storage pro escrow atomicamente (MF-024).
//! - Oferta nunca cruza região (§44) e você só opera no porto onde está (§45).
//! - Aceitar é tudo ou nada: o pedido sai do storage de quem aceita para o
//!   do vendedor, no mesmo porto; a oferta vai para quem aceitou.

use std::collections::HashMap;

use serde::{Deserialize, Serialize};

use bevy::app::AppExit;
use bevy::ecs::prelude::*;
use bevy::time::Time;
use chrono::{DateTime, Utc};
use lightyear::prelude::server::*;
use lightyear::prelude::*;
use marvyr_domain_crafting::{craft_in_storage, CraftError, Recipe, StationKind};
use marvyr_domain_economy::{
    validate_new_order, MarketError, MarketOrder, OrderStatus, ORDER_DURATION_SECS,
};
use marvyr_domain_items::{put_stack, CargoHold, Custody, ItemCatalog, ItemInstance, ItemLocation};
use marvyr_domain_ships::VesselPresence;
use marvyr_domain_world::WorldMap;
use marvyr_protocol::{
    BuySellOrder, CancelSellOrder, CatalogSnapshot, CreateSellOrder, ItemLine, MarketResult,
    OrderLine, OrdersSnapshot, StorageDeposit, StorageDepositAll, StorageWithdraw,
    StorageWithdrawAll,
};
use marvyr_shared::ids::{CharacterId, ItemDefinitionId, ItemInstanceId, MarketOrderId, RegionId};
use tracing::{info, warn};

use crate::net::{DevItems, ReliableChannel, ServerShip, ServerWorldMap};

/// Estado econômico serializável (MF-027, Phase 9): sobrevive a restart.
/// `FileStateStore` persiste este contrato em arquivo; `PostgresStateStore`
/// persiste o mesmo estado nas tabelas do ADR-0004 (persist.rs).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MarketSnapshot {
    /// Token de identidade persistente → CharacterId (MF-035): o dono é o
    /// personagem; a conexão/client_num é só transporte da sessão.
    pub identities: HashMap<String, CharacterId>,
    /// Chave composta vira lista de entradas: JSON não aceita chave-tupla.
    pub storage: Vec<StorageEntry>,
    pub escrow: Vec<EscrowEntry>,
    pub board: Vec<MarketOrder>,
    pub order_nums: HashMap<u32, MarketOrderId>,
    pub next_order_num: u32,
}

/// Uma gaveta de storage regional no snapshot (personagem × região).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct StorageEntry {
    pub character: CharacterId,
    pub region: RegionId,
    pub stacks: Vec<Custody>,
}

/// Um escrow de oferta no snapshot.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct EscrowEntry {
    pub order_num: u32,
    pub stacks: Vec<Custody>,
}

/// Toda a máquina econômica da sessão: storage regional, escrow e as
/// ofertas abertas. Um único Resource porque as operações de mercado
/// (listar, aceitar) tocam várias partes atomicamente.
#[derive(Resource)]
pub struct ServerMarket {
    identities: HashMap<String, CharacterId>,
    /// (personagem, região) → custódias guardadas (§30: storage regional).
    storage: HashMap<(CharacterId, RegionId), Vec<Custody>>,
    /// order_num → custódias em escrow, location `MarketEscrow` (MF-024).
    escrow: HashMap<u32, Vec<Custody>>,
    board: Vec<MarketOrder>,
    order_nums: HashMap<u32, MarketOrderId>,
    next_order_num: u32,
    /// Duração de uma oferta nova (MF-041); os testes encurtam.
    pub order_duration_secs: i64,
    /// Âncora de sobrevivência (MF-033/034). `Some(File)` = salvamento
    /// periódico; `Some(Postgres)` = persistência por operação crítica;
    /// `None` = dev puro, mundo descartável.
    store: Option<std::sync::Arc<dyn crate::persist::StateStore>>,
    /// Abates de NPC ainda não contados nas Caçadas: (quem, zona onde afundou).
    /// Transitório: não entra no snapshot.
    pub(crate) npc_kills: Vec<(CharacterId, Option<&'static str>)>,
}

impl Default for ServerMarket {
    fn default() -> Self {
        Self::new()
    }
}

impl ServerMarket {
    pub fn new() -> Self {
        Self::with_store(None)
    }

    pub fn with_store(store: Option<std::sync::Arc<dyn crate::persist::StateStore>>) -> Self {
        Self {
            identities: HashMap::new(),
            storage: HashMap::new(),
            escrow: HashMap::new(),
            board: Vec::new(),
            order_nums: HashMap::new(),
            next_order_num: 0,
            order_duration_secs: ORDER_DURATION_SECS,
            store,
            npc_kills: Vec::new(),
        }
    }

    /// Personagem já conhecido para a identidade, sem cunhar nada.
    pub fn known_character(&self, identity_token: &str) -> Option<CharacterId> {
        self.identities.get(identity_token).copied()
    }

    /// Identidade persistente do jogador (MF-035), cunhada no primeiro
    /// toque. O token vem do client e é o ÚNICO vínculo duradouro — conexão
    /// nenhuma é dona de nada.
    pub fn character(&mut self, identity_token: &str) -> CharacterId {
        if let Some(id) = self.identities.get(identity_token) {
            return *id;
        }
        let id = CharacterId::new();
        self.identities.insert(identity_token.to_string(), id);
        info!(character = ?id, "personagem novo");
        self.persist();
        id
    }

    /// Persiste o estado no store ativo (MF-034: operação crítica chega
    /// atomicamente ao banco; arquivo de dev salva no ritmo periódico).
    /// Chamado ao fim de toda mutação econômica.
    pub(crate) fn persist(&self) {
        if let Some(store) = &self.store {
            let snapshot = self.snapshot();
            if let Err(error) = store.save_market(&snapshot) {
                warn!(error = %error, "falha ao persistir estado econômico");
            }
        }
    }

    /// Estado atual como snapshot persistível.
    pub fn snapshot(&self) -> MarketSnapshot {
        MarketSnapshot {
            identities: self.identities.clone(),
            storage: self
                .storage
                .iter()
                .map(|((character, region), stacks)| StorageEntry {
                    character: *character,
                    region: *region,
                    stacks: stacks.clone(),
                })
                .collect(),
            escrow: self
                .escrow
                .iter()
                .map(|(order_num, stacks)| EscrowEntry {
                    order_num: *order_num,
                    stacks: stacks.clone(),
                })
                .collect(),
            board: self.board.clone(),
            order_nums: self.order_nums.clone(),
            next_order_num: self.next_order_num,
        }
    }

    /// Reconstrói o estado de um snapshot (boot com store). O store volta
    /// como âncora da sessão — persistências seguintes continuam por ele.
    pub fn restore_with_store(
        snapshot: MarketSnapshot,
        store: Option<std::sync::Arc<dyn crate::persist::StateStore>>,
    ) -> Self {
        Self {
            identities: snapshot.identities,
            storage: snapshot
                .storage
                .into_iter()
                .map(|entry| ((entry.character, entry.region), entry.stacks))
                .collect(),
            escrow: snapshot
                .escrow
                .into_iter()
                .map(|entry| (entry.order_num, entry.stacks))
                .collect(),
            board: snapshot.board,
            order_nums: snapshot.order_nums,
            next_order_num: snapshot.next_order_num,
            order_duration_secs: ORDER_DURATION_SECS,
            store,
            npc_kills: Vec::new(),
        }
    }

    /// Restore sem store (testes e dev puro).
    pub fn restore(snapshot: MarketSnapshot) -> Self {
        Self::restore_with_store(snapshot, None)
    }

    // ===== Operações atômicas (§70: apenas uma vence) =====
    // Os handlers Bevy são casca fina: detectam porto e traduzem resultado.
    // A lógica vive aqui, onde testes de concorrência chegam sem ECS.

    /// Deposita TODO o porão no storage regional (MF-023).
    pub fn deposit_all(
        &mut self,
        character: CharacterId,
        region: RegionId,
        hold: &mut CargoHold,
        catalog: &ItemCatalog,
    ) -> Result<(usize, u32), MarketError> {
        let drained = hold.drain();
        if drained.is_empty() {
            return Err(MarketError::EmptyStorage);
        }
        let weight: u32 = drained
            .iter()
            .filter_map(|custody| {
                catalog
                    .get(custody.instance.definition)
                    .map(|definition| definition.base_weight * custody.instance.quantity)
            })
            .sum();
        let stacks = drained.len();
        self.storage.entry((character, region)).or_default().extend(
            drained
                .into_iter()
                .map(|custody| custody.with_location(ItemLocation::PortStorage(region))),
        );
        self.persist();
        Ok((stacks, weight))
    }

    /// v28: guarda só um tipo do porão (a peça `instance`, ou todas as
    /// pilhas de `item`). Soma na pilha que já houver no armazém.
    #[allow(clippy::too_many_arguments)]
    pub fn deposit_item(
        &mut self,
        character: CharacterId,
        region: RegionId,
        hold: &mut CargoHold,
        catalog: &ItemCatalog,
        item: ItemDefinitionId,
        instance: Option<ItemInstanceId>,
    ) -> Result<usize, MarketError> {
        let ids: Vec<ItemInstanceId> = hold
            .items()
            .iter()
            .filter(|c| {
                c.instance.definition == item && instance.map_or(true, |id| c.instance.id == id)
            })
            .map(|c| c.instance.id)
            .collect();
        if ids.is_empty() {
            return Err(MarketError::EmptyStorage);
        }
        let max_stack = catalog
            .get(item)
            .map_or(1, |definition| definition.max_stack);
        let storage = self.storage.entry((character, region)).or_default();
        for id in &ids {
            if let Some(custody) = hold.remove_instance(*id) {
                put_stack(
                    storage,
                    custody.with_location(ItemLocation::PortStorage(region)),
                    max_stack,
                );
            }
        }
        self.persist();
        Ok(ids.len())
    }

    /// v28: leva só um tipo do armazém para o porão (a peça `instance`, ou
    /// as pilhas de `item` que couberem). Parcial é permitido.
    #[allow(clippy::too_many_arguments)]
    pub fn withdraw_item(
        &mut self,
        character: CharacterId,
        region: RegionId,
        hold: &mut CargoHold,
        catalog: &ItemCatalog,
        item: ItemDefinitionId,
        instance: Option<ItemInstanceId>,
    ) -> Result<usize, MarketError> {
        let Some(storage) = self.storage.get_mut(&(character, region)) else {
            return Err(MarketError::EmptyStorage);
        };
        let mut withdrawn = 0usize;
        let mut index = 0;
        while index < storage.len() {
            let wanted = storage[index].instance.definition == item
                && instance.map_or(true, |id| storage[index].instance.id == id);
            if wanted
                && hold
                    .insert(catalog, storage[index].instance.clone())
                    .is_ok()
            {
                storage.remove(index);
                withdrawn += 1;
            } else {
                index += 1;
            }
        }
        if withdrawn == 0 {
            return Err(MarketError::NotInStorage);
        }
        self.persist();
        Ok(withdrawn)
    }

    /// Retira do storage tudo que couber no porão (MF-023). Parcial é
    /// permitido: o que não coube fica guardado.
    pub fn withdraw_all(
        &mut self,
        character: CharacterId,
        region: RegionId,
        hold: &mut CargoHold,
        catalog: &ItemCatalog,
    ) -> Result<usize, MarketError> {
        let Some(storage) = self.storage.get_mut(&(character, region)) else {
            return Err(MarketError::EmptyStorage);
        };
        let mut withdrawn = 0usize;
        let mut index = 0;
        while index < storage.len() {
            let instance = storage[index].instance.clone();
            match hold.insert(catalog, instance) {
                Ok(()) => {
                    storage.remove(index);
                    withdrawn += 1;
                }
                Err(_) => index += 1,
            }
        }
        if withdrawn > 0 {
            self.persist();
        }
        Ok(withdrawn)
    }

    /// Cria oferta de escambo: storage → escrow atômico (MF-024). A oferta
    /// é fixa: sem estoque para a quantidade inteira, nada se move.
    /// Retorna o número de protocolo da oferta.
    pub fn create_order(
        &mut self,
        character: CharacterId,
        region: RegionId,
        item: ItemDefinitionId,
        quantity: u32,
        ask_item: ItemDefinitionId,
        ask_quantity: u32,
    ) -> Result<u32, MarketError> {
        validate_new_order(item, quantity, ask_item, ask_quantity)?;
        if self.storage_quantity(character, region, item) < quantity {
            return Err(MarketError::NotInStorage);
        }
        // Escrow atômico (MF-024): falha nenhuma chega depois daqui.
        let order_id = MarketOrderId::new();
        let escrowed = take_from_storage(
            self.storage
                .get_mut(&(character, region))
                .expect("checado acima"),
            item,
            quantity,
            ItemLocation::MarketEscrow(order_id),
        );
        let order_num = self.next_order_num;
        self.next_order_num += 1;
        self.order_nums.insert(order_num, order_id);
        self.escrow.insert(order_num, escrowed);
        let now = Utc::now();
        self.board.push(MarketOrder {
            id: order_id,
            seller: character,
            item,
            quantity,
            ask_item,
            ask_quantity,
            region,
            status: OrderStatus::Open,
            created_at: now,
            expires_at: now + chrono::Duration::seconds(self.order_duration_secs),
        });
        self.persist();
        Ok(order_num)
    }

    /// Quantidade de `item` no storage (personagem, região) — para validar
    /// receitas de oficina contra a riqueza guardada (MF-037).
    pub fn storage_quantity(
        &self,
        character: CharacterId,
        region: RegionId,
        item: ItemDefinitionId,
    ) -> u32 {
        self.storage
            .get(&(character, region))
            .map(|storage| storage_quantity(storage, item))
            .unwrap_or(0)
    }

    /// Total guardado por porto, fora `here` (o armazém é por porto: a UI
    /// mostra onde ficou o resto).
    pub(crate) fn storage_elsewhere(
        &self,
        character: CharacterId,
        here: RegionId,
    ) -> Vec<(RegionId, u32)> {
        let mut totals: Vec<(RegionId, u32)> = self
            .storage
            .iter()
            .filter(|((owner, region), _)| *owner == character && *region != here)
            .map(|((_, region), items)| (*region, items.iter().map(|c| c.instance.quantity).sum()))
            .filter(|(_, total)| *total > 0)
            .collect();
        totals.sort_by_key(|(region, _)| region.0);
        totals
    }

    /// Leitura do storage regional para snapshots de UI (porto atracado).
    pub(crate) fn port_storage(
        &self,
        character: CharacterId,
        region: RegionId,
    ) -> Option<&[Custody]> {
        self.storage.get(&(character, region)).map(Vec::as_slice)
    }

    /// Consome `quantity` de `item` do storage (insumos de oficina, MF-037).
    /// Fail-closed: sem estoque suficiente é erro e nada se move.
    pub fn consume_from_storage(
        &mut self,
        character: CharacterId,
        region: RegionId,
        item: ItemDefinitionId,
        quantity: u32,
    ) -> Result<(), MarketError> {
        let Some(storage) = self.storage.get_mut(&(character, region)) else {
            return Err(MarketError::NotInStorage);
        };
        if storage_quantity(storage, item) < quantity {
            return Err(MarketError::NotInStorage);
        }
        // Consumido: regrava a localização e descarta as pilhas retiradas.
        let _consumed =
            take_from_storage(storage, item, quantity, ItemLocation::PortStorage(region));
        self.persist();
        Ok(())
    }

    /// Troca com a Guilda Mercante: destrói `quantity` de `item` do storage
    /// (sink) e cria `paid` de `receive` no mesmo storage — tudo ou nada, um
    /// único persist.
    #[allow(clippy::too_many_arguments)]
    pub fn exchange_with_guild(
        &mut self,
        character: CharacterId,
        region: RegionId,
        item: ItemDefinitionId,
        quantity: u32,
        receive: ItemDefinitionId,
        paid: u32,
        catalog: &ItemCatalog,
    ) -> Result<(), MarketError> {
        let storage = self
            .storage
            .get_mut(&(character, region))
            .ok_or(MarketError::NotInStorage)?;
        if quantity == 0 || storage_quantity(storage, item) < quantity {
            return Err(MarketError::NotInStorage);
        }
        let _destroyed =
            take_from_storage(storage, item, quantity, ItemLocation::PortStorage(region));
        self.grant_to_storage(character, region, receive, paid, catalog);
        Ok(())
    }

    /// Cria `quantity` de um recurso bruto no storage do porto (pagamento de
    /// guilda e contrato). Persiste.
    pub fn grant_to_storage(
        &mut self,
        character: CharacterId,
        region: RegionId,
        item: ItemDefinitionId,
        quantity: u32,
        catalog: &ItemCatalog,
    ) {
        if quantity > 0 {
            let custody = Custody::new(
                ItemInstance::new_resource(ItemInstanceId::new(), item, quantity),
                ItemLocation::PortStorage(region),
            );
            self.return_to_storage(character, region, custody, catalog);
        }
    }

    /// Oficina do porto (MF-037): executa a receita sobre o storage regional
    /// — insumos saem do storage, output volta para o storage. O porão não
    /// é insumo automático: quem embarca decide o que embarca (Pilar 2).
    pub fn craft_at_storage(
        &mut self,
        character: CharacterId,
        region: RegionId,
        recipe: &Recipe,
        catalog: &ItemCatalog,
        station: StationKind,
    ) -> Result<ItemInstance, CraftError> {
        let Some(storage) = self.storage.get_mut(&(character, region)) else {
            return Err(CraftError::EmptyStorage);
        };
        let output = craft_in_storage(recipe, storage, catalog, station, region)?;
        self.persist();
        Ok(output)
    }

    /// Retira UMA unidade do item do storage (equipar, MF-039), ou a peça
    /// `instance` quando o client escolheu uma. Fail-closed.
    pub fn take_one_from_storage(
        &mut self,
        character: CharacterId,
        region: RegionId,
        item: ItemDefinitionId,
        instance: Option<ItemInstanceId>,
    ) -> Result<Custody, MarketError> {
        let storage = self
            .storage
            .get_mut(&(character, region))
            .ok_or(MarketError::NotInStorage)?;
        // Peça exata (afixos diferem entre peças do mesmo tipo).
        if let Some(id) = instance {
            let index = storage
                .iter()
                .position(|c| c.instance.id == id && c.instance.definition == item)
                .ok_or(MarketError::NotInStorage)?;
            let custody = storage.remove(index);
            self.persist();
            return Ok(custody);
        }
        let mut taken = take_from_storage(storage, item, 1, ItemLocation::PortStorage(region));
        match taken.pop() {
            Some(custody) => {
                self.persist();
                Ok(custody)
            }
            None => Err(MarketError::NotInStorage),
        }
    }

    /// Devolve uma custódia ao storage (swap de loadout, MF-039). Storage
    /// não tem limite: nunca falha, nunca destrói.
    pub fn return_to_storage(
        &mut self,
        character: CharacterId,
        region: RegionId,
        custody: Custody,
        catalog: &ItemCatalog,
    ) {
        let max_stack = catalog
            .get(custody.instance.definition)
            .map(|definition| definition.max_stack)
            .unwrap_or(1);
        let storage = self.storage.entry((character, region)).or_default();
        put_stack(
            storage,
            custody.with_location(ItemLocation::PortStorage(region)),
            max_stack,
        );
        self.persist();
    }

    /// Cancela SUA order: item volta do escrow pro storage; fee não volta.
    pub fn cancel_order(
        &mut self,
        character: CharacterId,
        order_num: u32,
    ) -> Result<(), MarketError> {
        let order_id = self
            .order_nums
            .get(&order_num)
            .copied()
            .ok_or(MarketError::UnknownOrder)?;
        let position = self
            .board
            .iter()
            .position(|order| order.id == order_id)
            .ok_or(MarketError::OrderNotOpen)?;
        if self.board[position].seller != character {
            return Err(MarketError::NotOrderOwner);
        }
        if self.board[position].status != OrderStatus::Open {
            return Err(MarketError::OrderNotOpen);
        }
        let order = self.board.remove(position);
        if let Some(escrowed) = self.escrow.remove(&order_num) {
            self.storage
                .entry((character, order.region))
                .or_default()
                .extend(
                    escrowed.into_iter().map(|custody| {
                        custody.with_location(ItemLocation::PortStorage(order.region))
                    }),
                );
        }
        self.persist();
        Ok(())
    }

    /// Aceita uma oferta (MF-025/§43-45), tudo ou nada: o pedido sai do
    /// storage de quem aceita para o do vendedor, e o escrow vai para quem
    /// aceitou — os dois no porto da oferta. Apenas uma operação vence (§70).
    pub fn buy(
        &mut self,
        buyer: CharacterId,
        buyer_region: RegionId,
        order_num: u32,
    ) -> Result<BuyReceipt, MarketError> {
        let order_id = self
            .order_nums
            .get(&order_num)
            .copied()
            .ok_or(MarketError::UnknownOrder)?;
        let position = self
            .board
            .iter()
            .position(|order| order.id == order_id)
            .ok_or(MarketError::OrderNotOpen)?;
        let order = self.board[position].clone();
        if order.status != OrderStatus::Open {
            return Err(MarketError::OrderNotOpen);
        }
        if order.region != buyer_region {
            return Err(MarketError::RegionMismatch);
        }
        if order.seller == buyer {
            return Err(MarketError::OwnOrder);
        }
        let available = self.storage_quantity(buyer, buyer_region, order.ask_item);
        if available < order.ask_quantity {
            return Err(MarketError::NotEnoughToPay {
                needed: order.ask_quantity,
                available,
            });
        }
        let region = ItemLocation::PortStorage(buyer_region);
        let payment = take_from_storage(
            self.storage
                .get_mut(&(buyer, buyer_region))
                .expect("checado acima"),
            order.ask_item,
            order.ask_quantity,
            region,
        );
        self.storage
            .entry((order.seller, buyer_region))
            .or_default()
            .extend(payment);
        let escrowed = self.escrow.remove(&order_num).unwrap_or_default();
        self.storage
            .entry((buyer, buyer_region))
            .or_default()
            .extend(
                escrowed
                    .into_iter()
                    .map(|custody| custody.with_location(region)),
            );
        self.board.remove(position);
        self.persist();
        Ok(BuyReceipt {
            order_num,
            seller: order.seller,
            item: order.item,
            quantity: order.quantity,
            ask_item: order.ask_item,
            ask_quantity: order.ask_quantity,
            region: order.region,
        })
    }

    /// Expira ofertas vencidas (MF-041). Escrow volta ao storage do seller na
    /// região da oferta.
    pub fn expire_orders(&mut self, now: DateTime<Utc>) -> usize {
        // ponytail: varredura global; agendamento por região se o board crescer.
        let mut expired = Vec::new();
        for (index, order) in self.board.iter().enumerate() {
            if order.expires_at >= now || order.status != OrderStatus::Open {
                continue;
            }
            let order_num = self
                .order_nums
                .iter()
                .find(|(_, id)| **id == order.id)
                .map(|(num, _)| *num)
                .expect("invariant: order_id present in order_nums");
            if let Some(escrowed) = self.escrow.remove(&order_num) {
                self.storage
                    .entry((order.seller, order.region))
                    .or_default()
                    .extend(escrowed.into_iter().map(|custody| {
                        custody.with_location(ItemLocation::PortStorage(order.region))
                    }));
            }
            expired.push(index);
        }
        if expired.is_empty() {
            return 0;
        }
        let expired_count = expired.len();
        for index in expired {
            self.board[index].status = OrderStatus::Expired;
        }
        self.persist();
        expired_count
    }
}

/// Recibo de uma troca aceita (para log do handler).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BuyReceipt {
    pub order_num: u32,
    pub seller: CharacterId,
    pub item: ItemDefinitionId,
    pub quantity: u32,
    pub ask_item: ItemDefinitionId,
    pub ask_quantity: u32,
    pub region: RegionId,
}

/// A região cujo porto contém o navio (§45: você opera onde está).
pub fn port_region(map: &WorldMap, x: f32, y: f32) -> Option<(RegionId, &str)> {
    map.regions()
        .iter()
        .find(|region| region.port.as_ref().is_some_and(|port| port.contains(x, y)))
        .and_then(|region| region.port.as_ref().map(|_| (region.id, region.name)))
}

fn storage_quantity(storage: &[Custody], item: ItemDefinitionId) -> u32 {
    storage
        .iter()
        .filter(|custody| custody.instance.definition == item)
        .map(|custody| custody.instance.quantity)
        .sum()
}

/// Retira `quantity` unidades de um item da lista, regravando a localização
/// pelo destino. Consome pilhas na ordem; parcial é permitido.
fn take_from_storage(
    storage: &mut Vec<Custody>,
    item: ItemDefinitionId,
    quantity: u32,
    destination: ItemLocation,
) -> Vec<Custody> {
    let mut remaining = quantity;
    let mut taken = Vec::new();
    let mut index = 0;
    while index < storage.len() && remaining > 0 {
        if storage[index].instance.definition != item {
            index += 1;
            continue;
        }
        let available = storage[index].instance.quantity;
        if available <= remaining {
            let custody = storage.remove(index);
            remaining -= available;
            taken.push(custody.with_location(destination));
        } else {
            storage[index].instance.quantity -= remaining;
            // A parte retirada é outra instância: com o mesmo id, o upsert
            // por id do Postgres juntava as duas numa linha só.
            let mut partial = storage[index].clone();
            partial.instance.id = ItemInstanceId::new();
            partial.instance.quantity = remaining;
            remaining = 0;
            taken.push(partial.with_location(destination));
        }
    }
    taken
}

pub(crate) fn market_result(
    connection_manager: &mut ConnectionManager,
    client_id: ClientId,
    success: bool,
    reason: &str,
) {
    let _ = connection_manager.send_message::<ReliableChannel, _>(
        client_id,
        &MarketResult {
            success,
            reason: String::from(reason),
        },
    );
}

pub(crate) fn region_name(map: &WorldMap, region: RegionId) -> &'static str {
    map.regions()
        .iter()
        .find(|candidate| candidate.id == region)
        .map(|candidate| candidate.name)
        .unwrap_or("?")
}

/// v28: arrastar um item entre porão e armazém. Mesmo serviço de porto do
/// "tudo"; o armazém atualizado chega pelo sync da guilda.
pub fn handle_storage_item(
    mut deposits: EventReader<ServerReceiveMessage<StorageDeposit>>,
    mut withdraws: EventReader<ServerReceiveMessage<StorageWithdraw>>,
    mut connection_manager: ResMut<ConnectionManager>,
    mut market: ResMut<ServerMarket>,
    dev: Res<DevItems>,
    mut ships: Query<&mut ServerShip>,
) {
    let moves = deposits
        .read()
        .map(|e| (e.from(), true, e.message().item, e.message().instance))
        .chain(
            withdraws
                .read()
                .map(|e| (e.from(), false, e.message().item, e.message().instance)),
        )
        .collect::<Vec<_>>();
    for (client_id, deposit, item, instance) in moves {
        let Some(mut ship) = ships.iter_mut().find(|s| s.client_id == Some(client_id)) else {
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
        let character = ship.character;
        let result = if deposit {
            market.deposit_item(
                character,
                region,
                &mut ship.hold,
                &dev.catalog,
                item,
                instance,
            )
        } else {
            market.withdraw_item(
                character,
                region,
                &mut ship.hold,
                &dev.catalog,
                item,
                instance,
            )
        };
        let (ok, reason) = match (result, deposit) {
            (Ok(_), true) => (true, "guardado no armazém"),
            (Ok(_), false) => (true, "levado para o porão"),
            (Err(_), true) => (false, "isso não está no porão"),
            (Err(_), false) => (false, "não coube no porão"),
        };
        market_result(&mut connection_manager, client_id, ok, reason);
    }
}

/// Deposit/withdraw do porão no storage do porto (PRD MF-023, §63 TransferItem).
pub fn handle_storage(
    mut deposit_events: EventReader<ServerReceiveMessage<StorageDepositAll>>,
    mut withdraw_events: EventReader<ServerReceiveMessage<StorageWithdrawAll>>,
    mut connection_manager: ResMut<ConnectionManager>,
    mut market: ResMut<ServerMarket>,
    dev: Res<DevItems>,
    map: Res<ServerWorldMap>,
    mut ships: Query<&mut ServerShip>,
) {
    for event in deposit_events.read() {
        let client_id = event.from();
        let Some(mut ship) = ships
            .iter_mut()
            .find(|ship| ship.client_id == Some(client_id))
        else {
            continue;
        };
        // MF-036: serviço de porto exige ATRACADO — água protegida não basta.
        let VesselPresence::Docked(region_id) = ship.presence else {
            info!(
                ship_id = ship.ship_id,
                "depósito recusado: atraca primeiro (E)"
            );
            market_result(
                &mut connection_manager,
                client_id,
                false,
                "atraca primeiro (E)",
            );
            continue;
        };
        let name = region_name(&map.0, region_id);
        let character = ship.character;
        match market.deposit_all(character, region_id, &mut ship.hold, &dev.catalog) {
            Ok((stacks, weight)) => {
                info!(
                    ship_id = ship.ship_id,
                    region = name,
                    stacks,
                    weight,
                    "porão depositado no storage regional"
                );
                market_result(
                    &mut connection_manager,
                    client_id,
                    true,
                    &format!("depositado em {name}"),
                );
            }
            Err(_) => {
                market_result(&mut connection_manager, client_id, false, "porão vazio");
            }
        }
    }

    for event in withdraw_events.read() {
        let client_id = event.from();
        let Some(mut ship) = ships
            .iter_mut()
            .find(|ship| ship.client_id == Some(client_id))
        else {
            continue;
        };
        let VesselPresence::Docked(region_id) = ship.presence else {
            market_result(
                &mut connection_manager,
                client_id,
                false,
                "atraca primeiro (E)",
            );
            continue;
        };
        let name = region_name(&map.0, region_id);
        let character = ship.character;
        match market.withdraw_all(character, region_id, &mut ship.hold, &dev.catalog) {
            Ok(0) => {
                info!(
                    ship_id = ship.ship_id,
                    "saque recusado: porão sem espaço ou storage vazio"
                );
                market_result(
                    &mut connection_manager,
                    client_id,
                    false,
                    "nada a retirar (storage vazio ou porão cheio)",
                );
            }
            Ok(stacks) => {
                info!(
                    ship_id = ship.ship_id,
                    region = name,
                    stacks,
                    "carga retirada do storage regional"
                );
                market_result(
                    &mut connection_manager,
                    client_id,
                    true,
                    &format!("retirado de {name}"),
                );
            }
            Err(_) => {
                market_result(
                    &mut connection_manager,
                    client_id,
                    false,
                    "storage vazio nesta região",
                );
            }
        }
    }
}

/// Oferta de escambo (MF-024/025): storage → escrow atômico.
pub fn handle_sell(
    mut sell_events: EventReader<ServerReceiveMessage<CreateSellOrder>>,
    mut connection_manager: ResMut<ConnectionManager>,
    mut market: ResMut<ServerMarket>,
    dev: Res<DevItems>,
    map: Res<ServerWorldMap>,
    ships: Query<&ServerShip>,
) {
    for event in sell_events.read() {
        let client_id = event.from();
        let message = event.message();
        let Some(ship) = ships.iter().find(|ship| ship.client_id == Some(client_id)) else {
            continue;
        };
        let Some((region_id, name)) = port_region(&map.0, ship.motion.x, ship.motion.y) else {
            market_result(
                &mut connection_manager,
                client_id,
                false,
                "fora de qualquer porto",
            );
            continue;
        };
        let character = ship.character;
        match market.create_order(
            character,
            region_id,
            message.item,
            message.quantity,
            message.ask_item,
            message.ask_quantity,
        ) {
            Ok(order_num) => {
                info!(
                    order_num,
                    region = name,
                    item = %item_name(&dev.catalog, message.item),
                    quantity = message.quantity,
                    ask = %item_name(&dev.catalog, message.ask_item),
                    ask_quantity = message.ask_quantity,
                    "oferta criada; item em escrow"
                );
                let viewers = viewers_of(&ships);
                broadcast_orders(
                    &mut connection_manager,
                    &market,
                    &dev.catalog,
                    &map.0,
                    &viewers,
                );
                market_result(
                    &mut connection_manager,
                    client_id,
                    true,
                    &format!("oferta {order_num} listada em {name}"),
                );
            }
            Err(error) => {
                warn!(error = %error, "sell order recusada");
                market_result(
                    &mut connection_manager,
                    client_id,
                    false,
                    &error.to_string(),
                );
            }
        }
    }
}

/// Aceitar oferta (MF-025/026): mesma região, tudo ou nada.
pub fn handle_buy(
    mut buy_events: EventReader<ServerReceiveMessage<BuySellOrder>>,
    mut connection_manager: ResMut<ConnectionManager>,
    mut market: ResMut<ServerMarket>,
    dev: Res<DevItems>,
    map: Res<ServerWorldMap>,
    ships: Query<&ServerShip>,
) {
    for event in buy_events.read() {
        let client_id = event.from();
        let message = event.message();
        let Some(ship) = ships.iter().find(|ship| ship.client_id == Some(client_id)) else {
            continue;
        };
        let VesselPresence::Docked(buyer_region_id) = ship.presence else {
            info!(
                ship_id = ship.ship_id,
                "compra recusada: atraca primeiro (E) — mercado é serviço de porto"
            );
            market_result(
                &mut connection_manager,
                client_id,
                false,
                "atraca primeiro (E)",
            );
            continue;
        };
        let buyer_region_name = region_name(&map.0, buyer_region_id);
        let buyer = ship.character;
        match market.buy(buyer, buyer_region_id, message.order_num) {
            Ok(receipt) => {
                info!(
                    order_num = receipt.order_num,
                    region = buyer_region_name,
                    item = %item_name(&dev.catalog, receipt.item),
                    quantity = receipt.quantity,
                    ask = %item_name(&dev.catalog, receipt.ask_item),
                    ask_quantity = receipt.ask_quantity,
                    "troca executada; itens mudaram de dono"
                );
                let viewers = viewers_of(&ships);
                broadcast_orders(
                    &mut connection_manager,
                    &market,
                    &dev.catalog,
                    &map.0,
                    &viewers,
                );
                market_result(
                    &mut connection_manager,
                    client_id,
                    true,
                    &format!(
                        "troca feita: {} {} no armazém",
                        receipt.quantity,
                        item_name(&dev.catalog, receipt.item)
                    ),
                );
            }
            Err(error) => {
                warn!(error = %error, "compra recusada");
                market_result(
                    &mut connection_manager,
                    client_id,
                    false,
                    &error.to_string(),
                );
            }
        }
    }
}

/// Cancelamento (§63): item volta do escrow pro storage da região da oferta.
pub fn handle_cancel(
    mut cancel_events: EventReader<ServerReceiveMessage<CancelSellOrder>>,
    mut connection_manager: ResMut<ConnectionManager>,
    mut market: ResMut<ServerMarket>,
    dev: Res<DevItems>,
    map: Res<ServerWorldMap>,
    ships: Query<&ServerShip>,
) {
    for event in cancel_events.read() {
        let client_id = event.from();
        let order_num = event.message().order_num;
        let Some(ship) = ships.iter().find(|ship| ship.client_id == Some(client_id)) else {
            continue;
        };
        let character = ship.character;
        match market.cancel_order(character, order_num) {
            Ok(()) => {
                info!(order_num, "oferta cancelada; item devolvido ao storage");
                let viewers = viewers_of(&ships);
                broadcast_orders(
                    &mut connection_manager,
                    &market,
                    &dev.catalog,
                    &map.0,
                    &viewers,
                );
                market_result(&mut connection_manager, client_id, true, "oferta cancelada");
            }
            Err(error) => {
                warn!(error = %error, "cancelamento recusado");
                market_result(
                    &mut connection_manager,
                    client_id,
                    false,
                    &error.to_string(),
                );
            }
        }
    }
}

/// Snapshot de orders personalizado por observador (o campo `mine` é do
/// dono — MF-035: personagem vs. vendedor). `viewers` são os pares
/// (sessão, personagem) online — o mercado não conhece o ECS. Enviado no
/// hello e a cada mudança no board.
pub fn broadcast_orders(
    connection_manager: &mut ConnectionManager,
    market: &ServerMarket,
    catalog: &ItemCatalog,
    map: &WorldMap,
    viewers: &[(Option<ClientId>, CharacterId)],
) {
    for (client_id, character) in viewers {
        if let Some(client_id) = client_id {
            let _ = connection_manager.send_message::<ReliableChannel, _>(
                *client_id,
                &OrdersSnapshot {
                    orders: order_lines_for(market, catalog, map, *character),
                },
            );
        }
    }
}

/// Linhas ativas do board para um observador (MF-041: Expired fica no estado
/// do servidor para auditoria, mas nunca vai ao client).
fn order_lines_for(
    market: &ServerMarket,
    catalog: &ItemCatalog,
    map: &WorldMap,
    character: CharacterId,
) -> Vec<OrderLine> {
    market
        .board
        .iter()
        .filter(|order| order.status == OrderStatus::Open)
        .map(|order| {
            let order_num = market
                .order_nums
                .iter()
                .find(|(_, id)| **id == order.id)
                .map(|(num, _)| *num)
                .unwrap_or(0);
            OrderLine {
                order_num,
                region: String::from(region_name(map, order.region)),
                item_name: item_name(catalog, order.item),
                quantity: order.quantity,
                ask_item_name: item_name(catalog, order.ask_item),
                ask_quantity: order.ask_quantity,
                mine: character == order.seller,
            }
        })
        .collect()
}

fn item_name(catalog: &ItemCatalog, item: ItemDefinitionId) -> String {
    catalog
        .get(item)
        .map(|definition| definition.display_name.clone())
        .unwrap_or_default()
}

/// Pares (sessão, personagem) dos navios — bridge ECS → funções puras.
pub fn viewers_of(ships: &Query<&ServerShip>) -> Vec<(Option<ClientId>, CharacterId)> {
    ships
        .iter()
        .map(|ship| (ship.client_id, ship.character))
        .collect()
}

/// Catálogo de itens para o client no hello (MF-023): nomes e pesos para a
/// UI, ids reais para os intents.
pub fn catalog_snapshot(catalog: &ItemCatalog) -> CatalogSnapshot {
    CatalogSnapshot {
        items: catalog
            .items()
            .map(|definition| ItemLine {
                id: definition.id,
                name: definition.display_name.clone(),
                weight: definition.base_weight,
                equipment_slot: definition
                    .equipment
                    .as_ref()
                    .map(|equipment| equipment.slot),
            })
            .collect(),
    }
}

/// Carrega o estado econômico do store ativo no boot (MF-027/033). Sem
/// store configurado, o mundo nasce limpo — comportamento de dev puro;
/// store que não lê derruba o boot.
pub fn load_state(store: Res<crate::persist::StoreHandle>, mut market: ResMut<ServerMarket>) {
    let Some(store) = store.0.clone() else {
        return;
    };
    match store.load_market() {
        Ok(Some(snapshot)) => {
            *market = ServerMarket::restore_with_store(snapshot, Some(store));
            info!("estado econômico restaurado do store");
        }
        Ok(None) => info!("mundo econômico novo (store vazio)"),
        // MV-067: começar limpo com o store ligado grava o vazio por cima do
        // real no primeiro save. Fail-closed, como o banco que não abre.
        Err(error) => panic!("store econômico ilegível; recusando subir: {error}"),
    }
}

/// Salvamento periódico (a cada 10s e no AppExit) — só para stores que
/// declaram esse modo (arquivo de dev). Postgres persiste por operação
/// crítica (MF-034); dev puro não persiste.
pub fn save_state(
    store: Res<crate::persist::StoreHandle>,
    market: Res<ServerMarket>,
    mut exit: EventReader<AppExit>,
    time: Res<Time>,
    mut timer: Local<f32>,
) {
    const SAVE_INTERVAL: f32 = 10.0;
    let Some(store) = store.0.clone() else {
        return;
    };
    if !store.periodic_saving() {
        return;
    }
    *timer += time.delta_secs();
    let exiting = exit.read().next().is_some();
    if *timer < SAVE_INTERVAL && !exiting {
        return;
    }
    *timer = 0.0;
    match store.save_market(&market.snapshot()) {
        Ok(()) => info!("estado econômico persistido"),
        Err(error) => warn!(error = %error, "falha ao persistir estado econômico"),
    }
}

/// Chave de identidade de uma conta autenticada pelo `marvyr-auth`. É o que
/// vai para `characters.name` — nunca o JWT (que é credencial).
pub fn account_identity(account: uuid::Uuid) -> String {
    format!("{ACCOUNT_IDENTITY_PREFIX}{account}")
}

pub const ACCOUNT_IDENTITY_PREFIX: &str = "account:";

/// Conta dona de uma chave de identidade (`None` = token anônimo de dev).
pub fn account_of_identity(identity: &str) -> Option<uuid::Uuid> {
    identity
        .strip_prefix(ACCOUNT_IDENTITY_PREFIX)
        .and_then(|raw| uuid::Uuid::parse_str(raw).ok())
}

#[cfg(test)]
mod tests {
    use chrono::Duration;

    use marvyr_domain_items::{CargoHold, ItemDefinition, ItemInstance, ItemKind};
    use marvyr_shared::ids::{ItemInstanceId, ShipInstanceId};

    use super::*;

    #[test]
    fn unreadable_store_refuses_to_boot_and_keeps_the_file() {
        use bevy::ecs::system::RunSystemOnce;

        let path =
            std::env::temp_dir().join(format!("marvyr-corrupt-{:?}.json", ItemInstanceId::new()));
        std::fs::write(&path, b"{ corrompido").unwrap();
        let store: std::sync::Arc<dyn crate::persist::StateStore> =
            std::sync::Arc::new(crate::persist::FileStateStore::new(path.clone()));
        let mut world = World::new();
        world.insert_resource(crate::persist::StoreHandle(Some(store.clone())));
        world.insert_resource(ServerMarket::with_store(Some(store)));

        let booted = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
            let _ = world.run_system_once(load_state);
        }));

        assert!(booted.is_err(), "store ilegível não sobe com mercado vazio");
        assert_eq!(std::fs::read(&path).unwrap(), b"{ corrompido");
        let _ = std::fs::remove_file(&path);
    }

    /// Catálogo com madeira e minério.
    fn catalog_with_items() -> (ItemCatalog, ItemDefinitionId, ItemDefinitionId) {
        let mut catalog = ItemCatalog::default();
        let mut ids = Vec::new();
        for name in ["Madeira", "Minério"] {
            let id = ItemDefinitionId::new();
            catalog
                .register(ItemDefinition {
                    id,
                    kind: ItemKind::Resource,
                    equipment: None,
                    max_stack: 100,
                    base_weight: 1,
                    tags: Default::default(),
                    display_name: String::from(name),
                })
                .expect("catálogo de teste não registra duplicatas");
            ids.push(id);
        }
        (catalog, ids[0], ids[1])
    }

    fn put_in_storage(
        market: &mut ServerMarket,
        character: CharacterId,
        region: RegionId,
        item: ItemDefinitionId,
        catalog: &ItemCatalog,
        quantity: u32,
    ) {
        let mut hold = CargoHold::new(ShipInstanceId::new(), 1_000);
        hold.insert(
            catalog,
            ItemInstance::new_resource(ItemInstanceId::new(), item, quantity),
        )
        .expect("teste cabe no porão");
        market
            .deposit_all(character, region, &mut hold, catalog)
            .expect("teste deposita no storage");
    }

    #[test]
    fn one_item_moves_between_hold_and_storage_without_touching_the_rest() {
        let mut market = ServerMarket::new();
        let character = market.character("capitão");
        let region = RegionId::new();
        let (catalog, wood, ore) = catalog_with_items();
        let mut hold = CargoHold::new(ShipInstanceId::new(), 1_000);
        for (item, quantity) in [(wood, 10), (ore, 5), (wood, 7)] {
            hold.insert(
                &catalog,
                ItemInstance::new_resource(ItemInstanceId::new(), item, quantity),
            )
            .unwrap();
        }
        assert_eq!(
            market.deposit_item(character, region, &mut hold, &catalog, wood, None),
            Ok(2)
        );
        assert_eq!(market.storage_quantity(character, region, wood), 17);
        assert_eq!(
            market.port_storage(character, region).unwrap().len(),
            1,
            "somou na pilha"
        );
        assert_eq!(hold.items().len(), 1, "o minério ficou");
        assert_eq!(
            market.deposit_item(character, region, &mut hold, &catalog, wood, None),
            Err(MarketError::EmptyStorage)
        );

        assert_eq!(
            market.withdraw_item(character, region, &mut hold, &catalog, wood, None),
            Ok(1)
        );
        assert_eq!(market.storage_quantity(character, region, wood), 0);
        let mut tiny = CargoHold::new(ShipInstanceId::new(), 3);
        market
            .deposit_item(character, region, &mut hold, &catalog, wood, None)
            .unwrap();
        assert_eq!(
            market.withdraw_item(character, region, &mut tiny, &catalog, wood, None),
            Err(MarketError::NotInStorage),
            "não coube: nada se move"
        );
        assert_eq!(market.storage_quantity(character, region, wood), 17);
    }

    #[test]
    fn guild_exchange_destroys_what_was_given_and_grants_the_payout() {
        let mut market = ServerMarket::new();
        let character = market.character("seller");
        let region = RegionId::new();
        let (catalog, wood, ore) = catalog_with_items();
        put_in_storage(&mut market, character, region, ore, &catalog, 10);

        // Fail-closed: mais do que há no storage não move nada.
        assert_eq!(
            market.exchange_with_guild(character, region, ore, 11, wood, 99, &catalog),
            Err(MarketError::NotInStorage)
        );
        assert_eq!(market.storage_quantity(character, region, wood), 0);

        market
            .exchange_with_guild(character, region, ore, 4, wood, 9, &catalog)
            .expect("estoque suficiente");
        assert_eq!(market.storage_quantity(character, region, ore), 6);
        assert_eq!(market.storage_quantity(character, region, wood), 9);
    }

    #[test]
    fn create_order_sets_expiry_from_duration() {
        let mut market = ServerMarket::new();
        let character = market.character("seller");
        let region = RegionId::new();
        let (catalog, wood, ore) = catalog_with_items();
        put_in_storage(&mut market, character, region, wood, &catalog, 10);
        market.order_duration_secs = 123;

        let order_num = market
            .create_order(character, region, wood, 10, ore, 3)
            .expect("storage tem estoque");
        let snapshot = market.snapshot();
        let order_id = snapshot.order_nums[&order_num];
        let order = snapshot
            .board
            .iter()
            .find(|order| order.id == order_id)
            .expect("oferta criada");

        assert_eq!(order.expires_at - order.created_at, Duration::seconds(123));
    }

    #[test]
    fn offer_is_fixed_so_short_stock_lists_nothing() {
        let mut market = ServerMarket::new();
        let character = market.character("seller");
        let region = RegionId::new();
        let (catalog, wood, ore) = catalog_with_items();
        put_in_storage(&mut market, character, region, wood, &catalog, 5);

        assert_eq!(
            market.create_order(character, region, wood, 6, ore, 3),
            Err(MarketError::NotInStorage)
        );
        assert_eq!(market.storage_quantity(character, region, wood), 5);
        assert!(market.snapshot().board.is_empty());
    }

    #[test]
    fn expire_orders_flips_status_and_returns_escrow() {
        let mut market = ServerMarket::new();
        let character = market.character("seller");
        let region = RegionId::new();
        let (catalog, wood, ore) = catalog_with_items();
        put_in_storage(&mut market, character, region, wood, &catalog, 10);
        let order_num = market
            .create_order(character, region, wood, 10, ore, 3)
            .expect("storage tem estoque");
        let order = market.snapshot().board[0].clone();

        assert_eq!(
            market.expire_orders(order.expires_at + Duration::seconds(1)),
            1
        );
        let snapshot = market.snapshot();
        let expired = snapshot
            .board
            .iter()
            .find(|order| order.id == snapshot.order_nums[&order_num])
            .expect("oferta continua no board para auditoria");
        assert_eq!(expired.status, OrderStatus::Expired);
        assert!(snapshot.escrow.is_empty());
        assert_eq!(market.storage_quantity(character, region, wood), 10);
    }

    #[test]
    fn expired_order_cannot_be_cancelled_or_bought() {
        let mut market = ServerMarket::new();
        let character = market.character("seller");
        let buyer = market.character("buyer");
        let region = RegionId::new();
        let (catalog, wood, ore) = catalog_with_items();
        put_in_storage(&mut market, character, region, wood, &catalog, 10);
        put_in_storage(&mut market, buyer, region, ore, &catalog, 10);
        let order_num = market
            .create_order(character, region, wood, 10, ore, 3)
            .expect("storage tem estoque");
        let order = market.snapshot().board[0].clone();
        market.expire_orders(order.expires_at + Duration::seconds(1));

        assert_eq!(
            market.cancel_order(character, order_num),
            Err(MarketError::OrderNotOpen)
        );
        assert_eq!(
            market.buy(buyer, region, order_num),
            Err(MarketError::OrderNotOpen)
        );
    }

    #[test]
    fn order_lines_exclude_expired_orders() {
        let mut market = ServerMarket::new();
        let character = market.character("seller");
        let region = RegionId::new();
        let (catalog, wood, ore) = catalog_with_items();
        put_in_storage(&mut market, character, region, wood, &catalog, 20);
        market.order_duration_secs = 60;
        let open_num = market
            .create_order(character, region, wood, 10, ore, 3)
            .expect("primeira oferta");
        market.order_duration_secs = 0;
        let expired_num = market
            .create_order(character, region, wood, 10, ore, 3)
            .expect("segunda oferta");
        let expired = market.snapshot().board[1].clone();
        market.expire_orders(expired.expires_at + Duration::seconds(1));
        let map = WorldMap::vertical_slice();

        let lines = order_lines_for(&market, &catalog, &map, character);

        assert_eq!(lines.len(), 1);
        assert_eq!(lines[0].order_num, open_num);
        assert_eq!(lines[0].ask_item_name, "Minério");
        assert_eq!(lines[0].ask_quantity, 3);
        assert!(lines.iter().all(|line| line.order_num != expired_num));
    }

    #[test]
    fn accepting_swaps_both_sides_all_or_nothing() {
        let mut market = ServerMarket::new();
        let seller = market.character("seller");
        let buyer = market.character("buyer");
        let region = RegionId::new();
        let (catalog, wood, ore) = catalog_with_items();
        put_in_storage(&mut market, seller, region, wood, &catalog, 20);
        put_in_storage(&mut market, buyer, region, ore, &catalog, 5);
        let order_num = market
            .create_order(seller, region, wood, 20, ore, 6)
            .expect("seller tem estoque");

        // Falta pagamento: nada se move.
        assert_eq!(
            market.buy(buyer, region, order_num),
            Err(MarketError::NotEnoughToPay {
                needed: 6,
                available: 5
            })
        );
        assert_eq!(market.storage_quantity(buyer, region, ore), 5);
        // Outro porto e a própria oferta são recusados.
        assert_eq!(
            market.buy(buyer, RegionId::new(), order_num),
            Err(MarketError::RegionMismatch)
        );
        assert_eq!(
            market.buy(seller, region, order_num),
            Err(MarketError::OwnOrder)
        );

        put_in_storage(&mut market, buyer, region, ore, &catalog, 3);
        let receipt = market.buy(buyer, region, order_num).expect("troca");
        assert_eq!((receipt.quantity, receipt.ask_quantity), (20, 6));
        assert_eq!(market.storage_quantity(buyer, region, wood), 20);
        assert_eq!(market.storage_quantity(buyer, region, ore), 2);
        assert_eq!(market.storage_quantity(seller, region, ore), 6);
        assert!(market.snapshot().board.is_empty());
        assert!(market.snapshot().escrow.is_empty());
        // Só uma aceitação vence.
        assert_eq!(
            market.buy(buyer, region, order_num),
            Err(MarketError::OrderNotOpen)
        );
    }

    #[test]
    fn partial_take_is_a_new_instance() {
        let mut market = ServerMarket::new();
        let seller = market.character("seller");
        let buyer = market.character("buyer");
        let region = RegionId::new();
        let (catalog, wood, ore) = catalog_with_items();
        put_in_storage(&mut market, seller, region, wood, &catalog, 20);
        put_in_storage(&mut market, buyer, region, ore, &catalog, 10);
        let order_num = market
            .create_order(seller, region, wood, 5, ore, 4)
            .expect("oferta");
        market.buy(buyer, region, order_num).expect("troca");

        // Cada item mora em um lugar só: nenhum id repetido no estado.
        let snapshot = market.snapshot();
        let mut ids: Vec<_> = snapshot
            .storage
            .iter()
            .flat_map(|entry| entry.stacks.iter().map(Custody::instance_id))
            .collect();
        let total = ids.len();
        ids.sort_by_key(|id| id.0);
        ids.dedup();
        assert_eq!(ids.len(), total);
    }
}
