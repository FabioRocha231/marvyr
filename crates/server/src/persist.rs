//! Persistência do estado do servidor (MF-033/034, ADR-0004/0010).
//!
//! Boundary formal: o servidor fala com [`StateStore`], nunca com Postgres
//! ou arquivo diretamente. Duas implementações:
//!
//! * [`FileStateStore`] — snapshot JSON atômico (dev/smoke/testes).
//! * [`PostgresStateStore`] — o store de produção (ADR-0004): sqlx,
//!   migrations versionadas e cada operação crítica gravada
//!   **atomicamente** (uma transação por persistência — o estado nunca
//!   fica pela metade no banco, então um crash entre duas etapas não
//!   duplica item).
//!
//! Nenhum `domain-*` conhece este módulo (ADR-0006): o teste de arquitetura
//! em `tests/architecture.rs` barra sqlx/tokio/bevy fora do server.
//!
//! Concorrência (ADR-0010): o Alpha tem **um** escritor (o servidor) e a
//! unidade atômica é o estado completo dentro de `BEGIN..COMMIT`. Row-level
//! `SELECT .. FOR UPDATE` entra quando existir mais de um processo escritor
//! disputando linhas — mecanismo equivalente, justificado aqui.

use std::path::PathBuf;
use std::sync::Arc;

use bevy::ecs::prelude::Resource;
use marvyr_domain_economy::logbook::CaptainProgress;
use marvyr_domain_economy::{MarketOrder, OrderStatus};
use marvyr_domain_items::{Custody, ItemInstance};
use marvyr_domain_ships::{ShipKind, VesselPresence};
use marvyr_shared::ids::{CharacterId, ShipInstanceId, WreckId};
use serde::{Deserialize, Serialize};
use uuid::Uuid;

use crate::market::MarketSnapshot;

/// Registro persistido de um navio (MF-035: o navio sobrevive à sessão).
/// `cargo` são as custódias `ShipCargo`; `equipped` as `Equipped` nos slots
/// (MF-039 — o loadout volta como estava, stats são recalculados no restore).
#[derive(Debug, Clone, PartialEq)]
pub struct ShipRecord {
    pub ship_instance: ShipInstanceId,
    pub character: CharacterId,
    pub kind: ShipKind,
    pub hp: u32,
    pub x: f32,
    pub y: f32,
    pub heading: f32,
    pub cargo: Vec<Custody>,
    pub equipped: Vec<Custody>,
    /// MF-049: presença no momento da persistência. Restaurada como está;
    /// se for `AtSea`, o restore normaliza `trip_started_at` para `now`
    /// (não tentamos reconstruir duração anterior — sem dado persistido
    /// para isso). `Docked` mantém `trip_started_at = None`.
    pub presence: VesselPresence,
    /// MV-061: marujos a bordo (tripulação é propriedade embarcada).
    pub crew: u16,
}

/// Registro persistido de um wreck (MF-027 cont., PRD §67): apenas os
/// metadados do destroço. O conteúdo econômico do baú (`WreckChest`) já
/// vive em `item_instances` filtrado por `ItemLocation::Wreck`; esta
/// tabela guarda só o invólucro para que o wreck reapareça após restart
/// com killer, posição e janela corretos.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct WreckRecord {
    pub wreck_num: u32,
    pub wreck_id: WreckId,
    pub x: f32,
    pub y: f32,
    pub exclusive_looter: Option<CharacterId>,
    /// Segundos decorridos no momento do spawn, relativos ao boot do
    /// server. Permite recompor o tempo de vida restante após restart.
    pub spawned_at_secs: f64,
}

/// Cosméticos de um personagem (MV-066): ids do catálogo que ele possui e
/// o que está usando. Só aparência.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct CosmeticsRecord {
    pub owned: Vec<String>,
    pub sail: Option<String>,
    pub flag: Option<String>,
}

/// O contrato de persistência do servidor (MF-033). Síncrono de propósito:
/// o loop Bevy chama e espera; implementações bloqueantes (Postgres) rodam
/// em runtime próprio.
pub trait StateStore: Send + Sync {
    /// Estado econômico completo do boot. `None` = mundo novo.
    fn load_market(&self) -> Result<Option<MarketSnapshot>, String>;
    /// Persiste o estado econômico completo, atomicamente.
    fn save_market(&self, snapshot: &MarketSnapshot) -> Result<(), String>;
    /// Navio persistido de um personagem (restore pós-janela de graça).
    fn load_ship(&self, character: CharacterId) -> Result<Option<ShipRecord>, String>;
    /// Persiste o navio (e a carga embarcada) de um personagem.
    fn save_ship(&self, record: &ShipRecord) -> Result<(), String>;
    /// Naufrágio: o casco persistido (e o que tinha a bordo) deixa de
    /// existir — senão o próximo hello restauraria carga que já virou wreck.
    fn delete_ships_of(&self, character: CharacterId) -> Result<(), String>;
    /// `true` = salvamento periódico aceitável (arquivo de dev);
    /// `false` = persistência por operação crítica (produção).
    fn periodic_saving(&self) -> bool;
    /// Snapshot completo de wrecks ativos. Vazio = nenhum wreck vivo.
    fn load_wreck_snapshot(&self) -> Result<Vec<WreckRecord>, String>;
    /// Persiste o snapshot de wrecks, substituindo o anterior atomicamente.
    fn save_wreck_snapshot(&self, wrecks: &[WreckRecord]) -> Result<(), String>;
    /// Remove um wreck específico (expiração pontual antes do próximo
    /// snapshot completo).
    fn delete_wreck(&self, wreck_num: u32) -> Result<(), String>;
    /// Cosméticos concedidos e em uso (MV-066). Personagem sem linha = nada.
    fn load_cosmetics(&self, character: CharacterId) -> Result<CosmeticsRecord, String>;
    /// Grava o que o capitão está usando (`None` = padrão do casco).
    fn save_cosmetic_choice(
        &self,
        character: CharacterId,
        sail: Option<&str>,
        flag: Option<&str>,
    ) -> Result<(), String>;
    /// Renome acumulado (MV-067). Personagem sem linha = 0.
    fn load_renown(&self, character: CharacterId) -> Result<u64, String>;
    /// Grava o Renome de vários capitões numa transação. Renome só sobe:
    /// um total menor (sessão que não conseguiu ler o banco) nunca apaga
    /// o gravado.
    fn save_renown(&self, totals: &[(CharacterId, u64)]) -> Result<(), String>;
    /// Talentos da Rosa dos Ventos (MV-067); vazio se nunca aprendeu.
    fn load_talents(&self, character: CharacterId) -> Result<Vec<String>, String>;
    fn save_talents(&self, character: CharacterId, talents: &[String]) -> Result<(), String>;
    /// v35: progressão do capitão (Diário de Bordo). Sem banco, fica só na
    /// sessão.
    fn load_progress(&self, _character: CharacterId) -> Result<CaptainProgress, String> {
        Ok(CaptainProgress::default())
    }
    fn save_progress(
        &self,
        _character: CharacterId,
        _progress: &CaptainProgress,
    ) -> Result<(), String> {
        Ok(())
    }
    /// v45: quem tem mais influência no porto nesta semana.
    fn load_port_lord(
        &self,
        _week: u32,
        _port: &str,
    ) -> Result<Option<(CharacterId, u32)>, String> {
        Ok(None)
    }
    /// v43: os `limit` capitães com mais pontos na temporada `season`.
    fn load_season_top(
        &self,
        _season: u32,
        _limit: u32,
    ) -> Result<Vec<(CharacterId, u32)>, String> {
        Ok(Vec::new())
    }
    /// Telemetria de retenção: (capitão, tipo, detalhe) em lote. Sem banco,
    /// ninguém lê.
    fn append_events(&self, _events: &[(CharacterId, &'static str, String)]) -> Result<(), String> {
        Ok(())
    }
    /// Hash do certificado WebTransport deste boot, para o `marvyr-auth`
    /// entregar ao browser (`GET /v1/web-cert`). Sem banco, ninguém lê.
    fn publish_web_cert(&self, _digest: &str) -> Result<(), String> {
        Ok(())
    }
}

type ItemRow = (
    Uuid,
    Uuid,
    Uuid,
    i32,
    Option<i16>,
    serde_json::Value,
    Option<serde_json::Value>,
);
type ShipItemRow = (
    Uuid,
    Uuid,
    i32,
    Option<i16>,
    serde_json::Value,
    Option<serde_json::Value>,
);

/// Raridade/afixos para a coluna `quality` (NULL na peça Normal).
fn quality_json(instance: &ItemInstance) -> Result<Option<serde_json::Value>, sqlx::Error> {
    instance
        .quality
        .as_ref()
        .map(serde_json::to_value)
        .transpose()
        .map_err(|error| sqlx::Error::ColumnDecode {
            index: "quality".into(),
            source: Box::new(error),
        })
}

/// Afixo ilegível derruba a leitura (MV-067): peça sem afixo gravada por
/// cima apagaria o que o jogador fabricou.
fn decode_quality(
    id: Uuid,
    quality: Option<serde_json::Value>,
) -> Result<Option<marvyr_domain_items::Quality>, String> {
    quality
        .map(serde_json::from_value)
        .transpose()
        .map_err(|error| format!("item {id} com afixos ilegíveis: {error}"))
}

/// Snapshot JSON em arquivo (`MARVYR_STATE_PATH`), escrita atômica via
/// tmp+rename. Escopo: dev, smoke e testes (MF-033) — NÃO é a persistência
/// de produção. Navios não são persistidos aqui: no modo dev o mundo nasce
/// limpo por sessão.
pub struct FileStateStore {
    path: PathBuf,
}

impl FileStateStore {
    pub fn new(path: PathBuf) -> Self {
        Self { path }
    }
}

impl StateStore for FileStateStore {
    fn load_market(&self) -> Result<Option<MarketSnapshot>, String> {
        match std::fs::read(&self.path) {
            Ok(bytes) => serde_json::from_slice(&bytes)
                .map(Some)
                .map_err(|error| format!("snapshot ilegível: {error}")),
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(None),
            Err(error) => Err(error.to_string()),
        }
    }

    fn save_market(&self, snapshot: &MarketSnapshot) -> Result<(), String> {
        let bytes = serde_json::to_vec_pretty(snapshot).map_err(|error| error.to_string())?;
        let mut temp = self.path.clone();
        temp.set_extension("tmp");
        // Escrita atômica via rename: crash no meio não corrompe o arquivo.
        std::fs::write(&temp, bytes)
            .and_then(|()| std::fs::rename(&temp, &self.path))
            .map_err(|error| error.to_string())
    }

    fn load_ship(&self, _character: CharacterId) -> Result<Option<ShipRecord>, String> {
        Ok(None)
    }

    fn save_ship(&self, _record: &ShipRecord) -> Result<(), String> {
        Ok(())
    }

    fn delete_ships_of(&self, _character: CharacterId) -> Result<(), String> {
        Ok(())
    }

    fn periodic_saving(&self) -> bool {
        true
    }

    fn load_wreck_snapshot(&self) -> Result<Vec<WreckRecord>, String> {
        let mut path = self.path.clone();
        path.set_extension("wrecks.json");
        match std::fs::read(&path) {
            Ok(bytes) => serde_json::from_slice(&bytes)
                .map_err(|error| format!("wreck snapshot ilegível: {error}")),
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(Vec::new()),
            Err(error) => Err(error.to_string()),
        }
    }

    fn save_wreck_snapshot(&self, wrecks: &[WreckRecord]) -> Result<(), String> {
        let bytes = serde_json::to_vec_pretty(wrecks).map_err(|error| error.to_string())?;
        let mut path = self.path.clone();
        path.set_extension("wrecks.json");
        let mut temp = path.clone();
        temp.set_extension("wrecks.tmp");
        // Escrita atômica via rename: o snapshot de wrecks é arquivo
        // separado do mercado; o crash entre as duas escritas deixa cada
        // arquivo consistente (rename é atômico por arquivo).
        std::fs::write(&temp, bytes)
            .and_then(|()| std::fs::rename(&temp, &path))
            .map_err(|error| error.to_string())
    }

    fn delete_wreck(&self, _wreck_num: u32) -> Result<(), String> {
        // No store de arquivo, a remoção individual é aplicada no próximo
        // save_wreck_snapshot completo. Mantemos a no-op para não quebrar
        // o contrato.
        Ok(())
    }

    fn load_cosmetics(&self, _character: CharacterId) -> Result<CosmeticsRecord, String> {
        // Dev: nada concedido (`MARVYR_DEV_COSMETICS` libera o catálogo).
        Ok(CosmeticsRecord::default())
    }

    fn save_cosmetic_choice(
        &self,
        _character: CharacterId,
        _sail: Option<&str>,
        _flag: Option<&str>,
    ) -> Result<(), String> {
        Ok(())
    }

    // Dev: Renome vive só na sessão (como os navios neste store).
    fn load_renown(&self, _character: CharacterId) -> Result<u64, String> {
        // Dev: `MARVYR_DEV_RENOWN=N` faz o capitão nascer com N de Renome
        // (testar a Rosa dos Ventos sem jogar horas). Só no store de dev.
        Ok(std::env::var("MARVYR_DEV_RENOWN")
            .ok()
            .and_then(|value| value.parse().ok())
            .unwrap_or(0))
    }

    fn save_renown(&self, _totals: &[(CharacterId, u64)]) -> Result<(), String> {
        Ok(())
    }

    fn load_talents(&self, _character: CharacterId) -> Result<Vec<String>, String> {
        Ok(Vec::new())
    }

    fn save_talents(&self, _character: CharacterId, _talents: &[String]) -> Result<(), String> {
        Ok(())
    }
}

/// Store de produção (ADR-0004): PostgreSQL via sqlx. O runtime tokio vive
/// nesta struct; as chamadas são síncronas (block_on) — o servidor Alpha é
/// single-writer e o custo de uma escrita local é de milissegundos.
pub struct PostgresStateStore {
    runtime: tokio::runtime::Runtime,
    pool: sqlx::PgPool,
}

impl PostgresStateStore {
    /// Conecta e roda as migrations versionadas do workspace.
    pub fn connect(url: &str) -> Result<Self, String> {
        let runtime = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .map_err(|error| error.to_string())?;
        let pool = runtime
            .block_on(async {
                sqlx::postgres::PgPoolOptions::new()
                    .max_connections(2)
                    .connect(url)
                    .await
            })
            .map_err(|error| format!("conexão postgres falhou: {error}"))?;
        runtime
            .block_on(async { sqlx::migrate!("../../migrations").run(&pool).await })
            .map_err(|error| format!("migrations falharam: {error}"))?;
        Ok(Self { runtime, pool })
    }

    async fn insert_custody(
        tx: &mut sqlx::PgConnection,
        owner: CharacterId,
        custody: &Custody,
    ) -> Result<(), sqlx::Error> {
        let location =
            serde_json::to_value(custody.location).map_err(|error| sqlx::Error::ColumnDecode {
                index: "location".into(),
                source: Box::new(error),
            })?;
        sqlx::query(
            "INSERT INTO item_instances \
             (id, owner_character_id, definition_id, quantity, durability, location, quality) \
             VALUES ($1, $2, $3, $4, $5, $6, $7) \
             ON CONFLICT (id) DO UPDATE SET \
             owner_character_id = EXCLUDED.owner_character_id, \
             definition_id = EXCLUDED.definition_id, quantity = EXCLUDED.quantity, \
             durability = EXCLUDED.durability, location = EXCLUDED.location, \
             quality = EXCLUDED.quality",
        )
        .bind(custody.instance.id.0)
        .bind(owner.0)
        .bind(custody.instance.definition.0)
        .bind(custody.instance.quantity as i32)
        .bind(custody.instance.durability.map(|d| d as i16))
        .bind(location)
        .bind(quality_json(&custody.instance)?)
        .execute(&mut *tx)
        .await
        .map(|_| ())
    }

    async fn insert_order(
        tx: &mut sqlx::PgConnection,
        order: &MarketOrder,
        order_num: u32,
    ) -> Result<(), sqlx::Error> {
        sqlx::query(
            "INSERT INTO market_orders \
             (id, seller_character_id, item_definition_id, quantity, \
              ask_item_definition_id, ask_quantity, \
              region_id, status, created_at, expires_at, order_num) \
             VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9, $10, $11) \
             ON CONFLICT (id) DO UPDATE SET status = EXCLUDED.status",
        )
        .bind(order.id.0)
        .bind(order.seller.0)
        .bind(order.item.0)
        .bind(order.quantity as i32)
        .bind(order.ask_item.0)
        .bind(order.ask_quantity as i32)
        .bind(order.region.0)
        .bind(format!("{:?}", order.status).to_lowercase())
        .bind(order.created_at)
        .bind(order.expires_at)
        .bind(order_num as i32)
        .execute(&mut *tx)
        .await
        .map(|_| ())
    }
}

impl StateStore for PostgresStateStore {
    fn load_market(&self) -> Result<Option<MarketSnapshot>, String> {
        self.runtime.block_on(async {
            let mut tx = self.pool.begin().await.map_err(|error| error.to_string())?;

            // Identidade persistente: token (name) → CharacterId.
            let identities: std::collections::HashMap<String, CharacterId> =
                sqlx::query_as::<_, (Uuid, String)>("SELECT id, name FROM characters")
                    .fetch_all(&mut *tx)
                    .await
                    .map_err(|error| error.to_string())?
                    .into_iter()
                    .map(|(id, token)| (token, CharacterId(id)))
                    .collect();
            if identities.is_empty() {
                return Ok(None); // banco vazio = mundo novo
            }

            // Itens por localização: storage regional e escrow de orders.
            let item_rows =
                sqlx::query_as::<_, ItemRow>(
                    "SELECT id, owner_character_id, definition_id, quantity, durability, location, \
                 quality FROM item_instances WHERE location ? 'PortStorage' OR location ? 'MarketEscrow'",
                )
                .fetch_all(&mut *tx)
                .await
                .map_err(|error| error.to_string())?;

            let mut storage: Vec<crate::market::StorageEntry> = Vec::new();
            let mut escrow: Vec<crate::market::EscrowEntry> = Vec::new();
            for (id, owner, definition, quantity, durability, location, quality) in item_rows {
                let instance = ItemInstance {
                    id: marvyr_shared::ids::ItemInstanceId(id),
                    definition: marvyr_shared::ids::ItemDefinitionId(definition),
                    quantity: quantity.max(0) as u32,
                    durability: durability.map(|d| d.max(0) as u16),
                    quality: decode_quality(id, quality)?,
                };
                let Ok(location) = serde_json::from_value(location) else {
                    return Err(format!("item {id} com location ilegível no banco"));
                };
                match location {
                    marvyr_domain_items::ItemLocation::PortStorage(region) => {
                        let custody = Custody { instance, location };
                        let owner_id = CharacterId(owner);
                        match storage
                            .iter_mut()
                            .find(|entry| entry.character == owner_id && entry.region == region)
                        {
                            Some(entry) => entry.stacks.push(custody),
                            None => storage.push(crate::market::StorageEntry {
                                character: owner_id,
                                region,
                                stacks: vec![custody],
                            }),
                        }
                    }
                    marvyr_domain_items::ItemLocation::MarketEscrow(order_id) => {
                        let order_num = escrow_order_num(&mut tx, order_id).await?;
                        let custody = Custody { instance, location };
                        match escrow.iter_mut().find(|entry| entry.order_num == order_num) {
                            Some(entry) => entry.stacks.push(custody),
                            None => escrow.push(crate::market::EscrowEntry {
                                order_num,
                                stacks: vec![custody],
                            }),
                        }
                    }
                    _ => {}
                }
            }

            let order_rows = sqlx::query_as::<
                _,
                (
                    Uuid,
                    Uuid,
                    Uuid,
                    i32,
                    Option<Uuid>,
                    Option<i32>,
                    Uuid,
                    String,
                    chrono::DateTime<chrono::Utc>,
                    chrono::DateTime<chrono::Utc>,
                    i32,
                ),
            >(
                "SELECT id, seller_character_id, item_definition_id, quantity, \
                 ask_item_definition_id, ask_quantity, \
                 region_id, status, created_at, expires_at, order_num \
                 FROM market_orders",
            )
            .fetch_all(&mut *tx)
            .await
            .map_err(|error| error.to_string())?;

            let mut board = Vec::new();
            let mut order_nums = std::collections::HashMap::new();
            let mut next_order_num = 0u32;
            for (
                id,
                seller,
                item,
                quantity,
                ask_item,
                ask_quantity,
                region,
                status,
                created_at,
                expires_at,
                num,
            ) in order_rows
            {
                let Ok(status) = status.parse::<StoredOrderStatus>() else {
                    return Err(format!("order {id} com status desconhecido"));
                };
                let region = marvyr_shared::ids::RegionId(region);
                let seller = CharacterId(seller);
                let (Some(ask_item), Some(ask_quantity)) = (ask_item, ask_quantity) else {
                    // Oferta da era do ouro: cancela e devolve o escrow ao
                    // armazém do vendedor, no porto da oferta.
                    let returned = escrow
                        .iter()
                        .position(|entry| entry.order_num == num as u32)
                        .map(|index| escrow.remove(index).stacks)
                        .unwrap_or_default();
                    let returned = returned.into_iter().map(|custody| {
                        custody
                            .with_location(marvyr_domain_items::ItemLocation::PortStorage(region))
                    });
                    match storage
                        .iter_mut()
                        .find(|entry| entry.character == seller && entry.region == region)
                    {
                        Some(entry) => entry.stacks.extend(returned),
                        None => storage.push(crate::market::StorageEntry {
                            character: seller,
                            region,
                            stacks: returned.collect(),
                        }),
                    }
                    continue;
                };
                board.push(MarketOrder {
                    id: marvyr_shared::ids::MarketOrderId(id),
                    seller,
                    item: marvyr_shared::ids::ItemDefinitionId(item),
                    quantity: quantity.max(0) as u32,
                    ask_item: marvyr_shared::ids::ItemDefinitionId(ask_item),
                    ask_quantity: ask_quantity.max(0) as u32,
                    region,
                    status: status.0,
                    created_at,
                    expires_at,
                });
                order_nums.insert(num as u32, marvyr_shared::ids::MarketOrderId(id));
                next_order_num = next_order_num.max(num as u32 + 1);
            }

            tx.commit().await.map_err(|error| error.to_string())?;
            Ok(Some(MarketSnapshot {
                identities,
                storage,
                escrow,
                board,
                order_nums,
                next_order_num,
            }))
        })
    }

    fn save_market(&self, snapshot: &MarketSnapshot) -> Result<(), String> {
        self.runtime.block_on(async {
            // Uma transação por persistência (ADR-0010): ou o banco reflete
            // a operação inteira, ou não reflete nada — crash no meio não
            // duplica item.
            let mut tx = self.pool.begin().await.map_err(|error| error.to_string())?;

            for (token, character) in &snapshot.identities {
                // Conta real (`account:<uuid>`, criada pelo marvyr-auth) ou
                // conta-sombra do token anônimo de dev (id = personagem).
                let account = crate::market::account_of_identity(token).unwrap_or(character.0);
                let email = format!("char-{}@local.dev", account.simple());
                sqlx::query(
                    "INSERT INTO accounts (id, email, password_hash) VALUES ($1, $2, '') \
                     ON CONFLICT (id) DO NOTHING",
                )
                .bind(account)
                .bind(&email)
                .execute(&mut *tx)
                .await
                .map_err(|error| error.to_string())?;
                sqlx::query(
                    "INSERT INTO characters (id, account_id, name, region_id, last_port_region_id) \
                     VALUES ($1, $4, $2, $3, $3) ON CONFLICT (id) DO UPDATE SET \
                     name = EXCLUDED.name, last_seen_at = now()",
                )
                .bind(character.0)
                .bind(token)
                .bind(Uuid::nil())
                .bind(account)
                .execute(&mut *tx)
                .await
                .map_err(|error| error.to_string())?;
            }

            // Estado mutável é substituído inteiro dentro da transação;
            // storage/escrow/orders nunca ficam pela metade.
            // Só storage/escrow pertencem a este snapshot: carga de navio,
            // equipamento instalado e baús de wreck têm dono próprio
            // (`save_ship`/wrecks) e não podem sumir num save de mercado.
            sqlx::query(
                "DELETE FROM item_instances \
                 WHERE location ? 'PortStorage' OR location ? 'MarketEscrow'",
            )
            .execute(&mut *tx)
            .await
            .map_err(|error| error.to_string())?;
            sqlx::query("DELETE FROM market_orders")
                .execute(&mut *tx)
                .await
                .map_err(|error| error.to_string())?;

            for entry in &snapshot.storage {
                for custody in &entry.stacks {
                    Self::insert_custody(&mut tx, entry.character, custody)
                        .await
                        .map_err(|error| error.to_string())?;
                }
            }
            for entry in &snapshot.escrow {
                // O dono das custódias em escrow é o vendedor da order; sem
                // order no board, o escrow é órfão e não entra no banco.
                let Some(order_id) = snapshot.order_nums.get(&entry.order_num) else {
                    continue;
                };
                let Some(order) = snapshot.board.iter().find(|order| &order.id == order_id) else {
                    continue;
                };
                for custody in &entry.stacks {
                    Self::insert_custody(&mut tx, order.seller, custody)
                        .await
                        .map_err(|error| error.to_string())?;
                }
            }
            for order in &snapshot.board {
                let order_num = snapshot
                    .order_nums
                    .iter()
                    .find(|(_, id)| **id == order.id)
                    .map(|(num, _)| *num)
                    .unwrap_or(0);
                Self::insert_order(&mut tx, order, order_num)
                    .await
                    .map_err(|error| error.to_string())?;
            }

            tx.commit().await.map_err(|error| error.to_string())?;
            Ok(())
        })
    }

    fn load_ship(&self, character: CharacterId) -> Result<Option<ShipRecord>, String> {
        self.runtime.block_on(async {
            let Some((id, kind, hp, x, y, heading, presence, crew)) =
                sqlx::query_as::<_, (Uuid, String, i32, f64, f64, f64, String, i32)>(
                    "SELECT id, ship_kind, current_hp, position_x, position_y, heading, presence, \
                 crew FROM ship_instances WHERE character_id = $1 \
                 ORDER BY updated_at DESC LIMIT 1",
                )
                .bind(character.0)
                .fetch_optional(&self.pool)
                .await
                .map_err(|error| error.to_string())?
            else {
                return Ok(None);
            };
            let Ok(kind) = kind.parse::<StoredShipKind>() else {
                return Err(format!("navio {id} com ship_kind desconhecido"));
            };
            let presence = decode_presence(&presence)
                .map_err(|error| format!("navio {id} com presence ilegível: {error}"))?;

            let rows = sqlx::query_as::<_, ShipItemRow>(
                "SELECT id, definition_id, quantity, durability, location, quality \
                 FROM item_instances \
                 WHERE location ->> 'ShipCargo' = $2 \
                 OR location -> 'Equipped' ->> 'ship' = $2",
            )
            .bind(character.0)
            .bind(id.to_string())
            .fetch_all(&self.pool)
            .await
            .map_err(|error| error.to_string())?;

            let mut cargo = Vec::new();
            let mut equipped = Vec::new();
            for (item_id, definition, quantity, durability, location, quality) in rows {
                let Ok(location) = serde_json::from_value(location) else {
                    return Err(format!("item {item_id} com location ilegível"));
                };
                let custody = Custody {
                    instance: ItemInstance {
                        id: marvyr_shared::ids::ItemInstanceId(item_id),
                        definition: marvyr_shared::ids::ItemDefinitionId(definition),
                        quantity: quantity.max(0) as u32,
                        durability: durability.map(|d| d.max(0) as u16),
                        quality: decode_quality(item_id, quality)?,
                    },
                    location,
                };
                if matches!(
                    custody.location,
                    marvyr_domain_items::ItemLocation::Equipped { .. }
                ) {
                    equipped.push(custody);
                } else {
                    cargo.push(custody);
                }
            }

            Ok(Some(ShipRecord {
                ship_instance: ShipInstanceId(id),
                character,
                kind: kind.0,
                hp: hp.max(0) as u32,
                x: x as f32,
                y: y as f32,
                heading: heading as f32,
                cargo,
                equipped,
                presence,
                crew: crew.clamp(0, i32::from(u16::MAX)) as u16,
            }))
        })
    }

    fn save_ship(&self, record: &ShipRecord) -> Result<(), String> {
        self.runtime.block_on(async {
            let mut tx = self.pool.begin().await.map_err(|error| error.to_string())?;
            let presence = serde_json::to_string(&record.presence)
                .map_err(|error| format!("presence ilegível: {error}"))?;
            sqlx::query(
                "INSERT INTO ship_instances \
                 (id, character_id, definition_id, ship_kind, equipped_components, \
                  current_hp, current_region_id, position_x, position_y, heading, \
                  presence, crew) \
                 VALUES ($1, $2, $3, $4, '{}'::jsonb, $5, $3, $6, $7, $8, $9, $10) \
                 ON CONFLICT (id) DO UPDATE SET current_hp = EXCLUDED.current_hp, \
                 position_x = EXCLUDED.position_x, position_y = EXCLUDED.position_y, \
                 heading = EXCLUDED.heading, presence = EXCLUDED.presence, \
                 crew = EXCLUDED.crew, updated_at = now()",
            )
            .bind(record.ship_instance.0)
            .bind(record.character.0)
            .bind(Uuid::nil())
            .bind(format!("{:?}", record.kind))
            .bind(record.hp as i32)
            .bind(record.x as f64)
            .bind(record.y as f64)
            .bind(record.heading as f64)
            .bind(presence)
            .bind(i32::from(record.crew))
            .execute(&mut *tx)
            .await
            .map_err(|error| error.to_string())?;

            // Um personagem tem UM navio vivo: cascos antigos (afundados
            // ou substituídos por construção) saem junto com o que tinham a
            // bordo — senão o restore escolheria um casco qualquer.
            let stale: Vec<(Uuid,)> = sqlx::query_as(
                "SELECT id FROM ship_instances WHERE character_id = $1 AND id <> $2",
            )
            .bind(record.character.0)
            .bind(record.ship_instance.0)
            .fetch_all(&mut *tx)
            .await
            .map_err(|error| error.to_string())?;
            for (ship,) in stale
                .iter()
                .chain(std::iter::once(&(record.ship_instance.0,)))
            {
                // A carga e o equipamento substituem inteiros (itens do
                // navio somem e renascem do estado atual — mesma transação).
                sqlx::query(
                    "DELETE FROM item_instances WHERE location ->> 'ShipCargo' = $1 \
                     OR location -> 'Equipped' ->> 'ship' = $1",
                )
                .bind(ship.to_string())
                .execute(&mut *tx)
                .await
                .map_err(|error| error.to_string())?;
            }
            sqlx::query("DELETE FROM ship_instances WHERE character_id = $1 AND id <> $2")
                .bind(record.character.0)
                .bind(record.ship_instance.0)
                .execute(&mut *tx)
                .await
                .map_err(|error| error.to_string())?;
            for custody in record.cargo.iter().chain(&record.equipped) {
                Self::insert_custody(&mut tx, record.character, custody)
                    .await
                    .map_err(|error| error.to_string())?;
            }
            tx.commit().await.map_err(|error| error.to_string())?;
            Ok(())
        })
    }

    fn delete_ships_of(&self, character: CharacterId) -> Result<(), String> {
        self.runtime.block_on(async {
            let mut tx = self.pool.begin().await.map_err(|error| error.to_string())?;
            sqlx::query(
                "DELETE FROM item_instances WHERE \
                 location ->> 'ShipCargo' IN \
                   (SELECT id::text FROM ship_instances WHERE character_id = $1) \
                 OR location -> 'Equipped' ->> 'ship' IN \
                   (SELECT id::text FROM ship_instances WHERE character_id = $1)",
            )
            .bind(character.0)
            .execute(&mut *tx)
            .await
            .map_err(|error| error.to_string())?;
            sqlx::query("DELETE FROM ship_instances WHERE character_id = $1")
                .bind(character.0)
                .execute(&mut *tx)
                .await
                .map_err(|error| error.to_string())?;
            tx.commit().await.map_err(|error| error.to_string())
        })
    }

    fn periodic_saving(&self) -> bool {
        false
    }

    fn load_wreck_snapshot(&self) -> Result<Vec<WreckRecord>, String> {
        self.runtime.block_on(async {
            let rows = sqlx::query_as::<
                _,
                (i32, Uuid, f64, f64, Option<Uuid>, f64),
            >(
                "SELECT wreck_num, wreck_id, position_x, position_y,                  exclusive_looter, spawned_at_secs FROM wrecks",
            )
            .fetch_all(&self.pool)
            .await
            .map_err(|error| error.to_string())?;

            rows.into_iter()
                .map(|(num, id, x, y, looter, spawned)| {
                    Ok(WreckRecord {
                        wreck_num: num.max(0) as u32,
                        wreck_id: WreckId(id),
                        x: x as f32,
                        y: y as f32,
                        exclusive_looter: looter.map(CharacterId),
                        spawned_at_secs: spawned.max(0.0),
                    })
                })
                .collect()
        })
    }

    fn save_wreck_snapshot(&self, wrecks: &[WreckRecord]) -> Result<(), String> {
        self.runtime.block_on(async {
            // Mesma semântica do save_market: DELETE+INSERT dentro de uma
            // transação. O snapshot é pequeno (uma linha por wreck ativo).
            let mut tx = self.pool.begin().await.map_err(|error| error.to_string())?;
            sqlx::query("DELETE FROM wrecks")
                .execute(&mut *tx)
                .await
                .map_err(|error| error.to_string())?;
            for wreck in wrecks {
                sqlx::query(
                    "INSERT INTO wrecks                      (wreck_num, wreck_id, position_x, position_y,                       exclusive_looter, spawned_at_secs)                      VALUES ($1, $2, $3, $4, $5, $6)                      ON CONFLICT (wreck_num) DO UPDATE SET                      wreck_id = EXCLUDED.wreck_id,                      position_x = EXCLUDED.position_x,                      position_y = EXCLUDED.position_y,                      exclusive_looter = EXCLUDED.exclusive_looter,                      spawned_at_secs = EXCLUDED.spawned_at_secs",
                )
                .bind(wreck.wreck_num as i32)
                .bind(wreck.wreck_id.0)
                .bind(wreck.x as f64)
                .bind(wreck.y as f64)
                .bind(wreck.exclusive_looter.map(|c| c.0))
                .bind(wreck.spawned_at_secs)
                .execute(&mut *tx)
                .await
                .map_err(|error| error.to_string())?;
            }
            tx.commit().await.map_err(|error| error.to_string())?;
            Ok(())
        })
    }

    fn delete_wreck(&self, wreck_num: u32) -> Result<(), String> {
        self.runtime.block_on(async {
            sqlx::query("DELETE FROM wrecks WHERE wreck_num = $1")
                .bind(wreck_num as i32)
                .execute(&self.pool)
                .await
                .map_err(|error| error.to_string())?;
            Ok(())
        })
    }

    fn load_cosmetics(&self, character: CharacterId) -> Result<CosmeticsRecord, String> {
        self.runtime.block_on(async {
            let owned: Vec<(String,)> = sqlx::query_as(
                "SELECT cosmetic_id FROM character_cosmetics WHERE character_id = $1",
            )
            .bind(character.0)
            .fetch_all(&self.pool)
            .await
            .map_err(|error| error.to_string())?;
            let worn: Option<(Option<String>, Option<String>)> =
                sqlx::query_as("SELECT sail_cosmetic, flag_cosmetic FROM characters WHERE id = $1")
                    .bind(character.0)
                    .fetch_optional(&self.pool)
                    .await
                    .map_err(|error| error.to_string())?;
            let (sail, flag) = worn.unwrap_or_default();
            Ok(CosmeticsRecord {
                owned: owned.into_iter().map(|(id,)| id).collect(),
                sail,
                flag,
            })
        })
    }

    fn save_cosmetic_choice(
        &self,
        character: CharacterId,
        sail: Option<&str>,
        flag: Option<&str>,
    ) -> Result<(), String> {
        self.runtime.block_on(async {
            sqlx::query(
                "UPDATE characters SET sail_cosmetic = $2, flag_cosmetic = $3 WHERE id = $1",
            )
            .bind(character.0)
            .bind(sail)
            .bind(flag)
            .execute(&self.pool)
            .await
            .map_err(|error| error.to_string())?;
            Ok(())
        })
    }

    fn load_renown(&self, character: CharacterId) -> Result<u64, String> {
        self.runtime.block_on(async {
            let row: Option<(i64,)> = sqlx::query_as("SELECT renown FROM characters WHERE id = $1")
                .bind(character.0)
                .fetch_optional(&self.pool)
                .await
                .map_err(|error| error.to_string())?;
            Ok(row.map_or(0, |(renown,)| renown.max(0) as u64))
        })
    }

    fn save_renown(&self, totals: &[(CharacterId, u64)]) -> Result<(), String> {
        self.runtime.block_on(async {
            let mut tx = self.pool.begin().await.map_err(|error| error.to_string())?;
            for (character, total) in totals {
                sqlx::query("UPDATE characters SET renown = GREATEST(renown, $2) WHERE id = $1")
                    .bind(character.0)
                    .bind(i64::try_from(*total).unwrap_or(i64::MAX))
                    .execute(&mut *tx)
                    .await
                    .map_err(|error| error.to_string())?;
            }
            tx.commit().await.map_err(|error| error.to_string())
        })
    }

    fn load_talents(&self, character: CharacterId) -> Result<Vec<String>, String> {
        self.runtime.block_on(async {
            let row: Option<(Vec<String>,)> =
                sqlx::query_as("SELECT talents FROM characters WHERE id = $1")
                    .bind(character.0)
                    .fetch_optional(&self.pool)
                    .await
                    .map_err(|error| error.to_string())?;
            Ok(row.map(|(talents,)| talents).unwrap_or_default())
        })
    }

    fn save_talents(&self, character: CharacterId, talents: &[String]) -> Result<(), String> {
        self.runtime.block_on(async {
            let updated = sqlx::query("UPDATE characters SET talents = $2 WHERE id = $1")
                .bind(character.0)
                .bind(talents)
                .execute(&self.pool)
                .await
                .map_err(|error| error.to_string())?;
            if updated.rows_affected() == 0 {
                return Err(String::from("personagem não existe no banco"));
            }
            Ok(())
        })
    }

    fn load_progress(&self, character: CharacterId) -> Result<CaptainProgress, String> {
        self.runtime.block_on(async {
            let row: Option<(String, i64, i64, i64)> = sqlx::query_as(
                "SELECT progress::text, season, season_points, crowns \
                 FROM characters WHERE id = $1",
            )
            .bind(character.0)
            .fetch_optional(&self.pool)
            .await
            .map_err(|error| error.to_string())?;
            let Some((json, season, season_points, crowns)) = row else {
                return Ok(CaptainProgress::default());
            };
            let mut progress: CaptainProgress = serde_json::from_str(&json)
                .map_err(|error| format!("progresso ilegível: {error}"))?;
            progress.season = u32::try_from(season).unwrap_or(0);
            progress.season_points = u32::try_from(season_points).unwrap_or(0);
            progress.crowns = u32::try_from(crowns).unwrap_or(0);
            // Só a semana mais recente conta; as antigas ficam de histórico.
            let rows: Vec<(String, i64, i64)> = sqlx::query_as(
                "SELECT port, week, points FROM port_influence WHERE character_id = $1 \
                 AND week = (SELECT max(week) FROM port_influence WHERE character_id = $1)",
            )
            .bind(character.0)
            .fetch_all(&self.pool)
            .await
            .map_err(|error| error.to_string())?;
            for (port, week, points) in rows {
                progress.influence_week = u32::try_from(week).unwrap_or(0);
                progress
                    .influence
                    .insert(port, u32::try_from(points).unwrap_or(u32::MAX));
            }
            Ok(progress)
        })
    }

    fn save_progress(
        &self,
        character: CharacterId,
        progress: &CaptainProgress,
    ) -> Result<(), String> {
        let json = serde_json::to_string(progress).map_err(|error| error.to_string())?;
        self.runtime.block_on(async {
            let mut tx = self.pool.begin().await.map_err(|error| error.to_string())?;
            let updated = sqlx::query(
                "UPDATE characters SET progress = $2::jsonb, season = $3, \
                 season_points = $4, crowns = $5 WHERE id = $1",
            )
            .bind(character.0)
            .bind(json)
            .bind(i64::from(progress.season))
            .bind(i64::from(progress.season_points))
            .bind(i64::from(progress.crowns))
            .execute(&mut *tx)
            .await
            .map_err(|error| error.to_string())?;
            if updated.rows_affected() == 0 {
                return Err(String::from("personagem não existe no banco"));
            }
            // Semanas velhas não decidem mais Senhor nenhum: guarda só as
            // últimas `INFLUENCE_WEEKS_KEPT` como histórico.
            sqlx::query("DELETE FROM port_influence WHERE character_id = $1 AND week < $2")
                .bind(character.0)
                .bind(i64::from(progress.influence_week) - INFLUENCE_WEEKS_KEPT)
                .execute(&mut *tx)
                .await
                .map_err(|error| error.to_string())?;
            for (port, points) in &progress.influence {
                sqlx::query(
                    "INSERT INTO port_influence (character_id, port, week, points) \
                     VALUES ($1, $2, $3, $4) ON CONFLICT (character_id, port, week) \
                     DO UPDATE SET points = EXCLUDED.points",
                )
                .bind(character.0)
                .bind(port)
                .bind(i64::from(progress.influence_week))
                .bind(i64::from(*points))
                .execute(&mut *tx)
                .await
                .map_err(|error| error.to_string())?;
            }
            tx.commit().await.map_err(|error| error.to_string())
        })
    }

    fn load_port_lord(&self, week: u32, port: &str) -> Result<Option<(CharacterId, u32)>, String> {
        self.runtime.block_on(async {
            let row: Option<(Uuid, i64)> = sqlx::query_as(
                "SELECT character_id, points FROM port_influence \
                 WHERE week = $1 AND port = $2 AND points > 0 \
                 ORDER BY points DESC LIMIT 1",
            )
            .bind(i64::from(week))
            .bind(port)
            .fetch_optional(&self.pool)
            .await
            .map_err(|error| error.to_string())?;
            Ok(
                row.map(|(id, points)| {
                    (CharacterId(id), u32::try_from(points).unwrap_or(u32::MAX))
                }),
            )
        })
    }

    fn load_season_top(&self, season: u32, limit: u32) -> Result<Vec<(CharacterId, u32)>, String> {
        self.runtime.block_on(async {
            let rows: Vec<(Uuid, i64)> = sqlx::query_as(
                "SELECT id, season_points FROM characters \
                 WHERE season = $1 AND season_points > 0 \
                 ORDER BY season_points DESC LIMIT $2",
            )
            .bind(i64::from(season))
            .bind(i64::from(limit))
            .fetch_all(&self.pool)
            .await
            .map_err(|error| error.to_string())?;
            Ok(rows
                .into_iter()
                .map(|(id, points)| (CharacterId(id), u32::try_from(points).unwrap_or(u32::MAX)))
                .collect())
        })
    }

    fn append_events(&self, events: &[(CharacterId, &'static str, String)]) -> Result<(), String> {
        let ids: Vec<Uuid> = events.iter().map(|(character, _, _)| character.0).collect();
        let kinds: Vec<&str> = events.iter().map(|(_, kind, _)| *kind).collect();
        let details: Vec<&str> = events
            .iter()
            .map(|(_, _, detail)| detail.as_str())
            .collect();
        self.runtime.block_on(async {
            sqlx::query(
                "INSERT INTO captain_events (character_id, kind, detail) \
                 SELECT * FROM UNNEST($1::uuid[], $2::text[], $3::text[])",
            )
            .bind(ids)
            .bind(kinds)
            .bind(details)
            .execute(&self.pool)
            .await
            .map(|_| ())
            .map_err(|error| error.to_string())
        })
    }

    fn publish_web_cert(&self, digest: &str) -> Result<(), String> {
        self.runtime.block_on(async {
            sqlx::query(
                "INSERT INTO web_cert (id, digest, updated_at) VALUES (TRUE, $1, now()) \
                 ON CONFLICT (id) DO UPDATE SET \
                 digest = EXCLUDED.digest, updated_at = EXCLUDED.updated_at",
            )
            .bind(digest)
            .execute(&self.pool)
            .await
            .map(|_| ())
            .map_err(|error| error.to_string())
        })
    }
}

/// Semanas de influência de porto guardadas por capitão (a atual e
/// as anteriores), além delas o save poda.
const INFLUENCE_WEEKS_KEPT: i64 = 4;

/// Aceita a representação JSON atual e o default textual da primeira
/// migração MF-049. Bancos que já receberam `AtSea` não perdem o navio no
/// restore antes do primeiro salvamento pelo servidor novo.
fn decode_presence(value: &str) -> Result<VesselPresence, serde_json::Error> {
    serde_json::from_str(value).or_else(|json_error| {
        if value == "AtSea" {
            Ok(VesselPresence::AtSea)
        } else {
            Err(json_error)
        }
    })
}

/// Wrapper para parse do ShipKind armazenado ("SmallMerchant", ...).
struct StoredShipKind(ShipKind);

impl std::str::FromStr for StoredShipKind {
    type Err = ();

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        <ShipKind as serde::Deserialize>::deserialize(serde::de::value::StrDeserializer::<
            serde::de::value::Error,
        >::new(s))
        .map(Self)
        .map_err(|_| ())
    }
}

/// Wrapper para parse do status armazenado ("open", "partial", ...).
struct StoredOrderStatus(OrderStatus);

impl std::str::FromStr for StoredOrderStatus {
    type Err = ();

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        match s {
            "open" => Ok(Self(OrderStatus::Open)),
            // Parcial só existiu na era do ouro; essas ofertas são
            // canceladas no load.
            "partial" => Ok(Self(OrderStatus::Open)),
            "filled" => Ok(Self(OrderStatus::Filled)),
            "cancelled" => Ok(Self(OrderStatus::Cancelled)),
            "expired" => Ok(Self(OrderStatus::Expired)),
            _ => Err(()),
        }
    }
}

/// Número de protocolo de uma order em escrow (u32::MAX se a order não está
/// mais no board — os itens órfãos não agrupam com nenhuma escrow ativa).
async fn escrow_order_num(
    tx: &mut sqlx::PgConnection,
    order_id: marvyr_shared::ids::MarketOrderId,
) -> Result<u32, String> {
    let row: Option<(i32,)> = sqlx::query_as("SELECT order_num FROM market_orders WHERE id = $1")
        .bind(order_id.0)
        .fetch_optional(&mut *tx)
        .await
        .map_err(|error| error.to_string())?;
    Ok(row.map(|(num,)| num as u32).unwrap_or(u32::MAX))
}

/// Resource Bevy com o store ativo da sessão (`None` = dev puro, sem
/// persistência — mundo descartável).
#[derive(Resource, Clone, Default)]
pub struct StoreHandle(pub Option<Arc<dyn StateStore>>);

impl StoreHandle {
    /// Persiste o mercado, logando falha (não entra em pânico: a memória é
    /// a verdade da sessão; o store é a âncora de sobrevivência).
    pub fn save_market_quiet(&self, snapshot: &MarketSnapshot) {
        if let Some(store) = &self.0 {
            if let Err(error) = store.save_market(snapshot) {
                tracing::warn!(error = %error, "falha ao persistir estado econômico");
            }
        }
    }

    /// Persiste o snapshot de wrecks ativos, logando falha (mesma
    /// filosofia do save_market_quiet: a memória é a verdade da sessão,
    /// o store é a âncora).
    pub fn save_wreck_quiet(&self, wrecks: &[WreckRecord]) {
        if let Some(store) = &self.0 {
            if let Err(error) = store.save_wreck_snapshot(wrecks) {
                tracing::warn!(error = %error, "falha ao persistir wrecks");
            }
        }
    }
}

/// Fábrica a partir do ambiente (MF-033): Postgres de produção, arquivo de
/// dev, ou nada. Banco configurado e inacessível é erro de boot (fail-closed,
/// §69) — silenciosamente degradar persistência é como não tê-la.
pub fn store_from_env() -> StoreHandle {
    if let Ok(url) = std::env::var("MARVYR_DATABASE_URL") {
        match PostgresStateStore::connect(&url) {
            Ok(store) => {
                tracing::info!("persistência: PostgreSQL (ADR-0004)");
                return StoreHandle(Some(Arc::new(store)));
            }
            Err(error) => {
                panic!("MARVYR_DATABASE_URL configurado mas o banco não abriu: {error}");
            }
        }
    }
    if let Some(path) = std::env::var_os("MARVYR_STATE_PATH") {
        tracing::info!(
            path = %path.to_string_lossy(),
            "persistência: arquivo de dev (não é produção)"
        );
        return StoreHandle(Some(Arc::new(FileStateStore::new(PathBuf::from(path)))));
    }
    tracing::info!("persistência: nenhuma (dev puro, mundo descartável)");
    StoreHandle(None)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn legacy_text_presence_default_decodes_without_discarding_ship() {
        assert_eq!(
            decode_presence("AtSea").expect("default da migração é compatível"),
            VesselPresence::AtSea
        );
        assert_eq!(
            decode_presence("\"AtSea\"").expect("formato JSON atual é compatível"),
            VesselPresence::AtSea
        );
    }
}
