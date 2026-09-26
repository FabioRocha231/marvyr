//! protocol: tipos de wire do Marvyr (PRD §63/§64).
//!
//! Tipos puros com serde — o transporte (lightyear, ADR-0002) escolhe o
//! formato na camada dele; este crate define apenas o **contrato** entre
//! client e servidor. Versionamento no handshake conforme ADR-0011.

use marvyr_domain_combat::Ammo;
use marvyr_domain_crafting::recipe::StationKind;
use marvyr_domain_items::{EquipmentSlot, Quality, Rarity};
use marvyr_domain_ships::ShipKind;
use marvyr_domain_world::RiskTier;
use marvyr_shared::ids::{ItemDefinitionId, ItemInstanceId};
use serde::{Deserialize, Serialize};

/// Versão atual do protocolo. Qualquer mudança incompatível deve incrementar
/// este número (semântica: quebra de contrato = +1).
///
/// v2: combate — FireBroadside, ShipDestroyed e projéteis no snapshot.
/// v3: economia do naufrágio — cargo_weight no ShipState, ciclo de vida de
///     Wreck (WreckSpawned/WreckRemoved) e loot (LootWreck/LootResult).
/// v4: geografia de risco (MF-017) — ZoneChanged com tier e nome da zona;
///     o servidor é quem calcula a zona real (PRD §10), a UI representa.
/// v5: economia de recursos (MF-018/019) — nós visíveis (NodesSnapshot no
///     hello, NodeUpdated em toda mudança), coleta GatherNode/GatherResult.
/// v6: crafting (MF-021/022) — catálogo de receitas no hello
///     (RecipesSnapshot), intents CraftItem e veredito CraftResult.
/// v7: mercado regional (MF-023..026) — catálogo de itens e carteira no
///     hello, intents de storage (Z/X), sell/cancel/buy de orders, e
///     veredito MarketResult.
/// v8: A1-P0 — identidade estável (MF-035): `ClientHello.identity` carrega o
///     token persistente do jogador; a conexão nunca é dona de nada. E AOI
///     (MF-031): `WorldSnapshot` passa a ser construído por destinatário e
///     inclui wrecks visíveis (`WreckState`) — WreckSpawned/WreckRemoved
///     saem do protocolo (visibilidade de wreck vem do snapshot, com TTL no
///     client).
/// v9: A1-P1 — dock/undock (MF-036): intents `Dock`/`Undock` e veredito
///     `DockResult` (com estado resultante `docked`). Atracar é um ESTADO
///     explícito: serviços de porto (storage, craft, mercado, construção)
///     exigem `Docked`; movimento/tiro/coleta/saque exigem `AtSea`.
/// v10: A1-P2 — loadout (MF-039): intents `EquipItem`/`UnequipItem`,
///     veredito `LoadoutResult` e `LoadoutSnapshot` no handshake. E o
///     `ShipState` passa a carregar os stats AUTORITATIVOS (hp, max_hp,
///     velocidade máxima, dano e alcance) — o client nunca deriva stats.
/// v11: MF-056B — `ShipState.kind` é a identidade visual autoritativa do
///     casco; o client escolhe sprite, nunca stats ou regras.
/// v12: MF-056H — `ShipState.cargo_capacity` expõe o limite autoritativo do
///     porão para a UI.
/// v13: MF-059 — portais (`PortalsUpdate`), guilda e contratos, clima,
///      munição e `ShipState.sail_hp`/`ammo`.
/// v14: MF-060 — mar vivo: `ShipState.faction`/`notoriety_tier`,
///      `ReputationUpdate` e `WorldEvent`.
/// v15: MV-061 — Public Alpha. Handshake CONGELADO: `ClientHello` e
///      `ServerWelcome` são as duas primeiras mensagens registradas (ids de
///      rede 0 e 1) e só ganham campos no FIM — assim um client de qualquer
///      versão futura ainda decodifica a recusa e mostra "versão
///      incompatível". `ServerWelcome.reason` explica a recusa; `identity`
///      carrega o JWT do `marvyr-auth`. Gameplay: dano de leme, reparo no
///      mar, abordagem, tripulação, eventos de mundo, mapas do tesouro e
///      ilhas ocultas.
/// v16: MV-062 — onboarding. `LootResult`/`GatherResult`/`CraftResult`
///      ganham `reason` (motivo PT-BR da recusa, vazio no sucesso) e o
///      client reporta `OnboardingProgress` (telemetria, nunca concede nada).
/// v17: MV-065 — mundo procedural. `WorldSeed` (registrada no fim) chega
///      logo após o `ServerWelcome` aceito; o client monta o mesmo mapa.
/// v18: MV-066 — zonas, arenas e cosméticos. `PortalsUpdate.arenas` traz a
///      semente do miolo de cada cerração aberta. Cosméticos: `ShipState`
///      ganha `sail_cosmetic`/`flag_cosmetic`, `CosmeticsSnapshot` e
///      `WearCosmetic` (registradas no fim).
/// v19: MV-067 — Renome (`RenownUpdate`) e Rosa dos Ventos
///      (`TalentsSnapshot`, `AllocateTalent`, `RespecTalents`; `ActionKind`
///      ganha `Talent`), registrados no fim.
/// v20: sem moeda. `WalletUpdated` sai do protocolo; o mercado vira
///      escambo (`OrderLine`/`CreateSellOrder` com `ask_*`, `BuySellOrder`
///      só com o número, tudo ou nada); a guilda troca pelo recurso do
///      porto (`GuildPrices.payouts`, `GuildPriceLine.per_ten`); contrato
///      paga recurso (`ContractLine.reward_*`); `ReputationUpdate` perde o
///      `bounty`; `PortStorageSnapshot.elsewhere` mostra o que está
///      guardado nos outros portos.
/// v21: tiro automático em 360°. `FireBroadside` sai (o servidor dispara
///      sozinho); `ShipState` ganha `black_flag` e `fire_target`;
///      `SetBlackFlag` e `LockTarget` registrados no fim.
/// v22: afixos de equipamento. `CraftItem.rarity`; `CraftResult.quality`;
///      `RecipeEntry.magic`/`rare` (custo por raridade); `StorageLine` e
///      `LoadoutLine` com `instance`/`quality` (peça a peça); `EquipItem`
///      escolhe a peça por `instance`.
/// v23: `ShipState.aura` (0-3), o brilho de poder do equipamento raro.
/// v24: `SocketGem`/`UnsocketGem` e `Quality.gems` (gemas de suporte).
/// v25: `UseFlask` e `ShipState.flasks` (frascos de bordo).
/// v26: `TreasureHint.rarity`/`mods` e `Quality.map_mods` (mapas com
/// modificadores).
/// v27: `LoadoutLine.synergies` e `LoadoutSnapshot.sets` (sinergia de gemas).
/// v28: `StorageDeposit`/`StorageWithdraw` (mover um item entre porão e
/// armazém).
/// v29: `ApplyOrb`/`OrbResult` (orbes de ofício).
pub const PROTOCOL_VERSION: u16 = 29;

/// Rótulo de versão da build (`MARVYR_VERSION_LABEL` no build de release,
/// senão a versão do Cargo). Client e servidor mostram no log e no HUD.
pub const VERSION_LABEL: &str = match option_env!("MARVYR_VERSION_LABEL") {
    Some(label) => label,
    None => env!("CARGO_PKG_VERSION"),
};

/// Commit da build (`MARVYR_BUILD_SHA`), `dev` fora do pipeline.
pub const BUILD_SHA: &str = match option_env!("MARVYR_BUILD_SHA") {
    Some(sha) => sha,
    None => "dev",
};

/// Id de protocolo do netcode (lightyear). Constante ENTRE versões: a
/// incompatibilidade de versão é detectada no `ClientHello`, onde o client
/// ainda consegue mostrar a mensagem certa — não no transporte.
pub const NETCODE_PROTOCOL_ID: u64 = 0x4D41_5256_5952_0001;

/// Chave do connect token do netcode. Vai no binário do client (auth
/// `Manual`), então NÃO é segredo: o controle de acesso real é o JWT do
/// `ClientHello`. Só separa o Marvyr de outro tráfego netcode.
pub const NETCODE_KEY: [u8; 32] = *b"marvyr-public-alpha-netcode-key!";

/// Tamanho máximo aceito para `ClientHello.identity` (JWT cabe folgado).
pub const MAX_IDENTITY_LEN: usize = 2048;

/// Recusas que mandam o client de volta ao login (sessão ausente/vencida).
/// Ficam no protocolo porque o client decide a tela comparando o texto.
pub const REASON_NO_SESSION: &str = "Sessão ausente. Faça login novamente.";
pub const REASON_BAD_SESSION: &str = "Sessão expirada ou inválida. Faça login novamente.";

/// Primeira mensagem do client após conectar (ADR-0011). `identity` é o
/// token persistente do jogador (MF-035): o servidor resolve token →
/// CharacterId; ClientId/conexão é só transporte da sessão.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ClientHello {
    pub protocol_version: u16,
    pub identity: String,
}

impl ClientHello {
    /// Hello da versão atual com o token de identidade do jogador.
    pub fn current(identity: impl Into<String>) -> Self {
        Self {
            protocol_version: PROTOCOL_VERSION,
            identity: identity.into(),
        }
    }
}

/// Seed do mundo (v17, MV-065): o client monta o mesmo
/// `WorldMap::from_seed(seed)` do servidor — nada de geografia na rede.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct WorldSeed {
    pub seed: u64,
}

/// Cosméticos do capitão (v18, MV-066): o que ele possui e o que está
/// usando, em códigos do catálogo (`0` = nenhum). Chega no handshake e a
/// cada troca.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct CosmeticsSnapshot {
    pub owned: Vec<u8>,
    pub sail: u8,
    pub flag: u8,
}

/// Vestir um cosmético (v18): `slot` 0 = vela, 1 = bandeira; `code` 0 tira.
/// Só atracado e só o que o capitão possui — o servidor decide.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct WearCosmetic {
    pub slot: u8,
    pub code: u8,
}

/// Renome do capitão (v19, MV-067): o total e onde ele está no nível. Chega
/// ao conectar (`gained` 0) e a cada feito, com o motivo (PT-BR).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct RenownUpdate {
    pub total: u64,
    pub level: u32,
    /// Renome dentro do nível atual e o tamanho do nível (0 = nível máximo).
    pub into: u64,
    pub span: u64,
    pub gained: u32,
    pub reason: String,
}

/// Rosa dos Ventos (v19, MV-067): ids dos talentos aprendidos. Os pontos
/// saem do nível de Renome (`points_for_level`). Chega ao conectar e a
/// cada mudança.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct TalentsSnapshot {
    pub allocated: Vec<String>,
}

/// Aprender um talento (em qualquer lugar; o servidor valida pontos e pai).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct AllocateTalent {
    pub node: String,
}

/// Esquecer todos os talentos, pagando ouro (só atracado).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct RespecTalents;

/// Resposta do servidor. Conexão recusada (`accepted == false`) é encerrada
/// logo em seguida; `reason` é texto para o jogador (vazio quando aceito).
/// Layout congelado desde o v15 (ver [`PROTOCOL_VERSION`]).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ServerWelcome {
    pub protocol_version: u16,
    pub accepted: bool,
    pub reason: String,
}

impl ServerWelcome {
    pub fn accepted() -> Self {
        Self {
            protocol_version: PROTOCOL_VERSION,
            accepted: true,
            reason: String::new(),
        }
    }

    pub fn rejected(reason: impl Into<String>) -> Self {
        Self {
            protocol_version: PROTOCOL_VERSION,
            accepted: false,
            reason: reason.into(),
        }
    }
}

/// O servidor atribui um navio ao jogador aceito (janela com visão própria).
/// O `kind` alimenta a UI de loadout para filtrar itens compatíveis (MF-039).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct AssignShip {
    pub ship_id: u32,
    pub kind: ShipKind,
}

/// Intenção de navegação do jogador (PRD §63: ShipInput).
/// Campos espelham `MotionInput` de `domain-ships`; o servidor é quem
/// valida e aplica (Pilar 4).
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub struct ShipInput {
    /// [0, 1] — velas.
    pub throttle: f32,
    /// [-1, 1] — leme; positivo = bombordo (anti-horário).
    pub turn: f32,
}

/// Estado autoritativo de um navio no tick do snapshot (PRD §64: ShipState).
/// Desde o v10 carrega os STATS do servidor (MF-039): o client exibe, nunca
/// calcula — equipar vela/casco/canhão aparece aqui no próximo snapshot.
/// v10+: os cooldowns de bordo são campos de display do client; a verdade
/// continua no servidor (`BroadsideBattery`).
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub struct ShipState {
    pub ship_id: u32,
    /// Tipo autoritativo para apresentação. Nunca participa de regras no client.
    pub kind: marvyr_domain_ships::ShipKind,
    pub x: f32,
    pub y: f32,
    /// Radianos, 0 = +X, anti-horário (convenção de `domain-ships`).
    pub heading: f32,
    /// m/s (atual).
    pub speed: f32,
    /// Peso atual da carga (unidades de peso; o limite é do navio).
    pub cargo_weight: u32,
    pub hp: u32,
    pub max_hp: u32,
    /// m/s (máximo com o loadout atual).
    pub max_speed: f32,
    pub weapon_damage: u32,
    pub weapon_range: f32,
    #[serde(default)]
    pub port_cooldown_secs: f32,
    #[serde(default)]
    pub starboard_cooldown_secs: f32,
    /// MF-044: NPC é não-player. Aditivo, default false.
    #[serde(default)]
    pub is_npc: bool,
    /// MF-056H: limite de peso do porão com o loadout atual. Aditivo,
    /// default 0 (cliente antigo não quebra).
    #[serde(default)]
    pub cargo_capacity: u32,
    /// MF-059: integridade do pano (0..100); rasgado, o navio anda menos.
    #[serde(default = "full_sails")]
    pub sail_hp: f32,
    /// MF-059: munição carregada (tecla C). Default bala redonda.
    #[serde(default)]
    pub ammo: Ammo,
    /// Bandeira do navio (jogador, pirata, marinha, mercador NPC). Aditivo;
    /// só apresentação e HUD — regras de facção vivem no servidor.
    #[serde(default)]
    pub faction: Faction,
    /// Faixa de notoriedade do capitão (0 Honrado, 1 Suspeito, 2 Procurado).
    /// Todos veem quem é procurado. Aditivo, default 0.
    #[serde(default)]
    pub notoriety_tier: u8,
    /// v15: integridade do leme (0..100); avariado, o navio gira menos.
    #[serde(default = "full_sails")]
    pub rudder_hp: f32,
    /// v15: tripulação a bordo e capacidade do casco.
    #[serde(default)]
    pub crew: u16,
    #[serde(default)]
    pub crew_max: u16,
    /// v15: tripulação trabalhando no reparo em mar.
    #[serde(default)]
    pub repairing: bool,
    /// v15: progresso da escavação de tesouro (0 = não está cavando).
    #[serde(default)]
    pub dig_progress: f32,
    /// v18: cosméticos à mostra (código do catálogo `COSMETICS`; 0 = padrão
    /// do casco). Só aparência — nenhum stat vem daqui.
    #[serde(default)]
    pub sail_cosmetic: u8,
    #[serde(default)]
    pub flag_cosmetic: u8,
    /// v21: Bandeira Negra (`FLAG_*`). Todos veem.
    #[serde(default)]
    pub black_flag: u8,
    /// v21: navio na mira do tiro automático (retícula do client).
    #[serde(default)]
    pub fire_target: Option<u32>,
    /// v23: aura de poder (0-3) pela raridade do equipamento instalado.
    #[serde(default)]
    pub aura: u8,
    /// v25: frascos de bordo (cargas, efeito ligado, a bordo).
    #[serde(default)]
    pub flasks: FlaskWire,
}

/// v25: os quatro frascos na ordem de `FlaskKind::ALL`. Bit `i` de `active`
/// = efeito ligado (todos veem o brilho); de `aboard` = frasco no porão.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct FlaskWire {
    pub charges: [u8; 4],
    pub active: u8,
    pub aboard: u8,
}

fn full_sails() -> f32 {
    100.0
}

/// v15: liga/desliga o reparo em mar (tecla K). O servidor só repara com
/// o navio parado, fora de combate e com Madeira no porão.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct SetRepair {
    pub active: bool,
}

/// v15: tentativa de abordagem (tecla H) — encostado no alvo avariado ou
/// parado; o resultado depende das tripulações.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct BoardShip {
    pub target_ship_id: u32,
}

/// v15: contratar marujos no porto (atracado), pagando ouro.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct HireCrew {
    pub count: u16,
}

/// v15: cavar no ponto do mapa do tesouro (tecla J), parado sobre ele.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct DigTreasure;

/// v15: ação de mar/porto que recebeu veredito.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum ActionKind {
    Repair,
    Board,
    HireCrew,
    Dig,
    Talent,
    /// v21: Bandeira Negra e alvo travado.
    Gunnery,
}

/// v15: veredito das ações novas (texto para o toast do HUD).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ActionResult {
    pub action: ActionKind,
    pub success: bool,
    pub reason: String,
}

/// v15: tipo de evento de mundo em curso.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum SeaEventKind {
    /// Tempestade que também racha casco.
    Tempest,
    /// Comboio NPC com ouro e escolta pesada.
    TreasureFleet,
    /// Monstro marinho que ataca navios.
    Kraken,
    /// Maré rica em recurso raro, disputada.
    ContestedTide,
}

/// v15: evento de mundo visível para todos (área e tempo restante).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct SeaEventState {
    pub event_id: u32,
    pub kind: SeaEventKind,
    pub name: String,
    pub x: f32,
    pub y: f32,
    pub radius: f32,
    pub remaining_secs: f32,
}

/// v15: eventos de mundo em curso (~1 Hz, todos os clients).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct SeaEventsUpdate {
    pub events: Vec<SeaEventState>,
}

/// v15: onde os mapas do tesouro no porão apontam.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct TreasureHint {
    pub x: f32,
    pub y: f32,
    pub island: String,
    /// v26: raridade do mapa e os perigos que acordam ao cavar.
    #[serde(default)]
    pub rarity: marvyr_domain_items::Rarity,
    #[serde(default)]
    pub mods: Vec<marvyr_domain_items::MapMod>,
}

/// v15: pistas dos mapas do PRÓPRIO porão (~1 Hz, só para o dono).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct TreasureHints {
    pub hints: Vec<TreasureHint>,
}

/// v15: ilha oculta à vista (só aparece para quem chega perto).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct IslandState {
    pub island_id: u32,
    pub name: String,
    pub x: f32,
    pub y: f32,
    pub radius: f32,
}

/// v15: ilhas ocultas dentro do alcance de visão do navio (~1 Hz).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct IslandsInSight {
    pub islands: Vec<IslandState>,
}

/// v16: progresso do onboarding (telemetria de playtest). `step` 0 = boas-
/// vindas exibidas, 1..=6 = passos do guia concluídos; `skipped` quando o
/// jogador pula o guia. O servidor só registra — nunca concede nada.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct OnboardingProgress {
    pub step: u8,
    pub skipped: bool,
}

/// Troca de munição do próprio navio (MF-059). O servidor guarda e aplica
/// no próximo disparo; o veredito aparece em `ShipState.ammo`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct SelectAmmo {
    pub ammo: Ammo,
}

/// Célula de tempestade visível (MF-059).
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub struct StormState {
    pub storm_id: u32,
    pub x: f32,
    pub y: f32,
    pub radius: f32,
    /// 0..1 — fade in/out da célula.
    pub intensity: f32,
    /// v15: tempestade de evento (racha casco, não só pano).
    #[serde(default)]
    pub tempest: bool,
}

/// Clima do mar (MF-059), ~1 Hz para todos: vento global e tempestades.
/// `wind_dir` é para onde o vento sopra (radianos, convenção do heading).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct WeatherUpdate {
    pub wind_dir: f32,
    pub wind_strength: f32,
    pub storms: Vec<StormState>,
}

/// Facção de um navio no mar. `Player` é o default de wire.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default, Serialize, Deserialize)]
pub enum Faction {
    #[default]
    Player,
    Pirate,
    Navy,
    Merchant,
    /// v15: criatura marinha (kraken de evento).
    Monster,
}

/// Faixas de notoriedade no wire (`ShipState.notoriety_tier`).
pub const TIER_HONRADO: u8 = 0;
pub const TIER_SUSPEITO: u8 = 1;
pub const TIER_PROCURADO: u8 = 2;

/// `ShipState.black_flag` (v21).
pub const FLAG_LOWERED: u8 = 0;
pub const FLAG_HOISTING: u8 = 1;
pub const FLAG_RAISED: u8 = 2;

pub fn black_flag_wire(flag: marvyr_domain_combat::BlackFlag) -> u8 {
    match flag {
        marvyr_domain_combat::BlackFlag::Lowered => FLAG_LOWERED,
        marvyr_domain_combat::BlackFlag::Hoisting { .. } => FLAG_HOISTING,
        marvyr_domain_combat::BlackFlag::Raised { .. } => FLAG_RAISED,
    }
}

/// Reputação do PRÓPRIO capitão (só para o dono): notoriedade 0..1000 e
/// faixa.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct ReputationUpdate {
    pub notoriety: u32,
    pub tier: u8,
}

/// Tipo de evento do feed (cor/ícone no client).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum WorldEventKind {
    /// Afundamento, saque, recompensa recebida.
    Kill,
    /// Alarme: mercador atacado, marinha a caminho.
    Alert,
    /// Cabeça a prêmio / mudança de faixa de notoriedade.
    Bounty,
}

/// Linha do feed de eventos, só para jogadores envolvidos ou por perto.
/// Texto ASCII (a fonte padrão do client não tem acentos).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct WorldEvent {
    pub text: String,
    pub kind: WorldEventKind,
}

/// Instala um item do storage regional no slot dele (MF-039). Só atracado;
/// slot ocupado é swap — o antigo volta ao storage, nada é destruído.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct EquipItem {
    pub item: ItemDefinitionId,
    /// v22: a peça exata (afixos diferem); `None` = qualquer do tipo.
    #[serde(default)]
    pub instance: Option<ItemInstanceId>,
}

/// Desinstala o slot; o item volta ao storage da região onde está atracado.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct UnequipItem {
    pub slot: EquipmentSlot,
}

/// Uma linha do loadout do PRÓPRIO navio: o slot existe no casco? há item?
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct LoadoutLine {
    pub slot: EquipmentSlot,
    /// Display name do item equipado (vazio quando o slot está livre).
    pub item_name: String,
    pub equipped: bool,
    /// v22: raridade e afixos da peça instalada.
    #[serde(default)]
    pub quality: Option<Quality>,
    /// v24: encaixes de gema da peça (0 com o slot vazio).
    #[serde(default)]
    pub sockets: u8,
    /// v27: sinergias acesas nesta peça (ressonância, pares ligados).
    #[serde(default)]
    pub synergies: Vec<marvyr_domain_items::Synergy>,
}

/// Loadout completo do navio do observador, no hello e a cada troca.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct LoadoutSnapshot {
    pub slots: Vec<LoadoutLine>,
    /// v27: conjuntos do navio (mesma gema em três peças).
    #[serde(default)]
    pub sets: Vec<marvyr_domain_items::Synergy>,
}

/// Veredito de equipar/desequipar.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct LoadoutResult {
    pub success: bool,
    pub reason: String,
}

/// v21: içar (`raise`) ou arriar a Bandeira Negra (tecla R). O servidor
/// recusa atracado, em águas protegidas e arriar sem calma.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct SetBlackFlag {
    pub raise: bool,
}

/// v21: trava o tiro automático no navio mais próximo no alcance, mesmo
/// inocente (tecla Q); com alvo travado, solta.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct LockTarget;

/// v24: encaixa uma gema do armazém do porto na peça instalada no slot
/// (primeira vaga livre). Serviço de porto, como equipar.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct SocketGem {
    pub slot: EquipmentSlot,
    pub gem: marvyr_domain_items::GemKind,
}

/// v25: bebe o frasco (teclas 1-4 no mar). Sem dose ou já ligado: nada.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct UseFlask {
    pub kind: marvyr_domain_combat::FlaskKind,
}

/// v24: tira a gema do encaixe `index`; ela volta ao armazém do porto.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct UnsocketGem {
    pub slot: EquipmentSlot,
    pub index: u8,
}

/// Estado autoritativo de um projétil no tick do snapshot (PRD §20).
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub struct ProjectileState {
    pub projectile_id: u32,
    pub x: f32,
    pub y: f32,
    /// Direção de voo em radianos.
    pub heading: f32,
}

/// Navio afundou (PRD §21). O servidor retransmite a todos; a resolução de
/// loot acontece do lado do servidor (MF-013).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct ShipDestroyed {
    pub ship_id: u32,
}

/// Destroço visível no snapshot do destinatário (MF-031: visibilidade de
/// wreck é recorte de AOI como navios e projéteis).
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub struct WreckState {
    pub wreck_id: u32,
    pub x: f32,
    pub y: f32,
    /// Quantidade de pilhas de itens dentro do baú (para UI).
    pub stack_count: u32,
}

/// Snapshot do mundo **do ponto de vista do destinatário** (PRD §64, ADR-0009,
/// MF-031): só entidades dos chunks visíveis + anel de borda. Enviado a 20 Hz
/// (ADR-0008) por canal não-confiável.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct WorldSnapshot {
    pub tick: u64,
    pub ships: Vec<ShipState>,
    pub projectiles: Vec<ProjectileState>,
    pub wrecks: Vec<WreckState>,
}

/// Tipo de portal (MF-059): entrada/saída de cerração e sorvedouro.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum PortalKindWire {
    FogGate,
    FogExit,
    Whirlpool,
}

/// Portal visível no mundo. Poucos e globais: vão para todos, sem AOI.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub struct PortalState {
    pub portal_id: u32,
    pub kind: PortalKindWire,
    pub x: f32,
    pub y: f32,
    pub radius: f32,
    /// Segundos até sumir.
    pub expires_in_secs: f32,
    /// Travessias restantes; `None` = ilimitado.
    pub uses_left: Option<u32>,
}

/// Estado completo dos portais (MF-059), a cada segundo e quando mudam.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct PortalsUpdate {
    pub portals: Vec<PortalState>,
    /// Cerrações abertas: (slot, semente do miolo). O client sorteia os
    /// mesmos rochedos (`arena_layout`).
    pub arenas: Vec<(u8, u64)>,
}

/// Jogador quer saquear um wreck (PRD §27: precisa estar nele, com porão).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct LootWreck {
    pub wreck_id: u32,
}

/// Resultado da tentativa de saque (MF-015: atômico, capacity-aware).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct LootResult {
    pub wreck_id: u32,
    pub success: bool,
    /// v16: motivo da recusa para o jogador (vazio no sucesso).
    pub reason: String,
}

/// A zona real do navio mudou (PRD §10, MF-017). O servidor define a zona;
/// o client apenas representa. Enviado por canal confiável para o dono do
/// navio — no spawn (estado inicial) e a cada travessia de fronteira.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ZoneChanged {
    pub ship_id: u32,
    pub tier: RiskTier,
    /// Nome declarado da zona no mapa do servidor (ex.: "Rota da Costa").
    pub zone_name: String,
}

/// Estado visível de um nó de recurso (PRD §64: ResourceNodeUpdated,
/// MF-018). Nós são server-authoritative: o client desenha e pede.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct NodeState {
    /// Id numérico do nó (protocolo usa u32, como wrecks e navios).
    pub node_id: u32,
    pub x: f32,
    pub y: f32,
    /// Nome do recurso para a UI (display name do catálogo do servidor).
    pub resource_name: String,
    /// Unidades disponíveis agora (0 = esgotado, aguardando respawn).
    pub stock: u32,
    pub max_stock: u32,
}

/// Todos os nós do mundo, enviados no handshake — depois, só deltas.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct NodesSnapshot {
    pub nodes: Vec<NodeState>,
}

/// Um nó mudou (coleta de outro jogador ou respawn). Mesmo formato do
/// estado: o client substitui o que sabe.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct NodeUpdated {
    pub node: NodeState,
}

/// Jogador quer coletar um nó (PRD MF-019: perto, com estoque e porão).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct GatherNode {
    pub node_id: u32,
}

/// Resultado da tentativa de coleta (MF-019).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct GatherResult {
    pub node_id: u32,
    pub success: bool,
    /// Unidades efetivamente coletadas (0 quando falha).
    pub gathered: u32,
    /// v16: motivo da recusa para o jogador (vazio no sucesso).
    pub reason: String,
}

/// Uma receita do catálogo do servidor, pronta para exibição (MF-021/022).
/// O `recipe_id` é o índice numérico que o client devolve em `CraftItem`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct RecipeEntry {
    pub recipe_id: u32,
    pub display_name: String,
    /// Estação exigida (Workbench/Dock/None — §36).
    pub station: StationKind,
    /// "item" quando produz equipamento/recurso; "navio" quando o output é
    /// um ShipInstance construído no Dock (PRD §38: navio não é item).
    pub ship_build: bool,
    pub output_name: String,
    /// Quantidade produzida por craft (1 para equipamento).
    pub output_quantity: u32,
    /// Linhas de ingredientes já resolvidas para UI (nome + quantidade).
    pub ingredients: Vec<IngredientLine>,
    /// v22: custo da versão Mágica e da Rara (vazio = não tem raridade).
    #[serde(default)]
    pub magic: Vec<IngredientLine>,
    #[serde(default)]
    pub rare: Vec<IngredientLine>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct IngredientLine {
    pub name: String,
    pub quantity: u32,
}

/// Catálogo completo de receitas, enviado no handshake — estático na sessão.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct RecipesSnapshot {
    pub recipes: Vec<RecipeEntry>,
}

/// Jogador quer fabricar (PRD §63: CraftItem). O servidor valida estação,
/// ingredientes e porão — fail-closed (§37).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct CraftItem {
    pub recipe_id: u32,
    /// v22: raridade pedida (só equipamento).
    #[serde(default)]
    pub rarity: Rarity,
}

/// Resultado da tentativa de fabricação/construção.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct CraftResult {
    pub recipe_id: u32,
    pub success: bool,
    /// v16: motivo da recusa para o jogador (vazio no sucesso).
    pub reason: String,
    /// v22: afixos da peça fabricada (Mágica/Rara), para o client festejar.
    #[serde(default)]
    pub quality: Option<Quality>,
}

/// Linha do catálogo de itens (MF-023): id real para os intents, nome e
/// peso para a UI. Enviada no handshake.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ItemLine {
    pub id: ItemDefinitionId,
    pub name: String,
    pub weight: u32,
    /// Slot do equipamento, quando o item é equipável (MF-038).
    #[serde(default)]
    pub equipment_slot: Option<EquipmentSlot>,
}

/// Catálogo completo de itens do servidor, no handshake.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct CatalogSnapshot {
    pub items: Vec<ItemLine>,
}

/// Uma oferta de escambo visível (MF-025): "dou `quantity` de `item_name`
/// por `ask_quantity` de `ask_item_name`". `region` é o nome da região;
/// ofertas de regiões diferentes NUNCA cruzam (§44).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct OrderLine {
    pub order_num: u32,
    pub region: String,
    pub item_name: String,
    pub quantity: u32,
    pub ask_item_name: String,
    pub ask_quantity: u32,
    /// A order é deste client (habilita o cancelar).
    pub mine: bool,
}

/// Todas as orders abertas, enviadas no hello e a cada mudança (delta no
/// OrderUpdated seria o próximo passo; o slice reenvia o quadro inteiro).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct OrdersSnapshot {
    pub orders: Vec<OrderLine>,
}

/// Uma linha do storage regional do porto onde está atracado (post-review:
/// habilita a aba Loadout do PortScreen a oferecer "Equipar" via UI).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct StorageLine {
    pub item: ItemDefinitionId,
    pub item_name: String,
    pub quantity: u32,
    /// v22: equipamento vem peça a peça (id + afixos); recurso agrega.
    #[serde(default)]
    pub instance: Option<ItemInstanceId>,
    #[serde(default)]
    pub quality: Option<Quality>,
}

/// Snapshot do storage do porto onde o jogador acabou de dockar. Enviado
/// após `DockResult { success: true, docked: true }`. Sem ele, a aba Loadout
/// só permite desequipar (post-review: agora também permite equipar itens
/// compatíveis com os slots do casco).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PortStorageSnapshot {
    pub region: String,
    pub lines: Vec<StorageLine>,
    /// O que o jogador tem guardado nos OUTROS portos (o armazém é por
    /// porto): nome do porto e total de unidades.
    pub elsewhere: Vec<StoredElsewhere>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct StoredElsewhere {
    pub region: String,
    pub quantity: u32,
}

/// Atracar no porto onde está (MF-036). O servidor valida: dentro da área
/// do porto, devagar o bastante. Confiável: é uma decisão do jogador.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct Dock;

/// Desatracar (MF-036): volta ao ponto de atracação, mesmo HP, mesma carga.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct Undock;

/// Veredito de dock/undock (MF-036). `docked` é o ESTADO resultante — o
/// client não infere presença por texto.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct DockResult {
    pub success: bool,
    pub docked: bool,
    /// Texto de display (nome do porto ou motivo da recusa).
    pub reason: String,
}

/// Guarda TUDO do porão no storage regional do porto onde está (MF-023).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct StorageDepositAll;

/// Tira do storage tudo que couber de volta no porão (MF-023).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct StorageWithdrawAll;

/// v29: gasta um orbe do armazém na peça `target` do mesmo armazém.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct ApplyOrb {
    pub orb: marvyr_domain_items::OrbKind,
    pub target: ItemInstanceId,
}

/// v29: veredito do orbe; com sucesso, a peça como ficou (para a festa).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct OrbResult {
    pub success: bool,
    pub reason: String,
    pub orb: marvyr_domain_items::OrbKind,
    pub item_name: String,
    pub quality: Option<Quality>,
}

/// v28: guarda um tipo do porão (a peça `instance`, ou todas as pilhas).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct StorageDeposit {
    pub item: ItemDefinitionId,
    pub instance: Option<ItemInstanceId>,
}

/// v28: leva um tipo do armazém para o porão (o que couber).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct StorageWithdraw {
    pub item: ItemDefinitionId,
    pub instance: Option<ItemInstanceId>,
}

/// Cria oferta de escambo no porto onde está (MF-024/025): o item sai do
/// storage regional e entra em escrow atomicamente no servidor.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct CreateSellOrder {
    pub item: ItemDefinitionId,
    pub quantity: u32,
    pub ask_item: ItemDefinitionId,
    pub ask_quantity: u32,
}

/// Cancela sua oferta; o item volta do escrow pro storage.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct CancelSellOrder {
    pub order_num: u32,
}

/// Aceita uma oferta da região onde está (MF-025), tudo ou nada: o pedido
/// sai do seu storage local para o do vendedor, e a oferta entra no seu.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct BuySellOrder {
    pub order_num: u32,
}

/// Veredito de qualquer operação de storage/mercado. `reason` é texto de
/// display para HUD/log — a máquina de verdade vive no servidor.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct MarketResult {
    pub success: bool,
    pub reason: String,
}

/// Trocar com a Guilda Mercante (NPC) do porto onde está ATRACADO, a partir
/// do storage regional; ela paga com o recurso do porto. O servidor limita
/// ao estoque; item fora da tabela da guilda é recusado (fail-closed).
/// Veredito vem em `MarketResult`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct SellToGuild {
    pub item: ItemDefinitionId,
    pub quantity: u32,
}

/// Quanto 10 unidades de um item rendem do recurso de cada porto (mesma
/// ordem de `GuildPrices::ports`; 0 = a guilda de lá não aceita).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct GuildPriceLine {
    pub item: ItemDefinitionId,
    pub item_name: String,
    pub per_ten: Vec<u32>,
}

/// Câmbio da guilda em TODOS os portos (rota visível) + o que o navio leva
/// no porão (só leitura). Enviado ao atracado quando algo muda.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct GuildPrices {
    pub ports: Vec<String>,
    /// Recurso com que cada porto paga (mesma ordem de `ports`).
    pub payouts: Vec<String>,
    pub lines: Vec<GuildPriceLine>,
    pub cargo: Vec<StorageLine>,
}

/// Um contrato do Quadro (oferta ou ativo).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ContractLine {
    pub id: u32,
    pub title: String,
    pub reward_item: String,
    pub reward_quantity: u32,
    pub duration_secs: u32,
    /// Tempo restante (só faz sentido no contrato ativo).
    pub remaining_secs: u32,
    pub progress: u32,
    pub target: u32,
    /// Caça (abates) em vez de Entrega.
    pub hunt: bool,
}

/// Ofertas do porto atracado (vazio no mar) + seu contrato ativo.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ContractsSnapshot {
    pub offers: Vec<ContractLine>,
    pub active: Option<ContractLine>,
}

/// Aceitar uma oferta do Quadro (só atracado; 1 contrato ativo por vez).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct AcceptContract {
    pub id: u32,
}

/// Abandonar o contrato ativo (sem reembolso, sem multa).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct AbandonContract;

/// Veredito de contrato: aceite, abandono, conclusão ou expiração.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ContractResult {
    pub success: bool,
    pub reason: String,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn current_protocol_version_is_twenty_nine() {
        assert_eq!(PROTOCOL_VERSION, 29);
        assert_eq!(
            ClientHello::current("token").protocol_version,
            PROTOCOL_VERSION
        );
    }

    #[test]
    fn ship_state_carries_authoritative_stats() {
        let state = ShipState {
            ship_id: 1,
            kind: marvyr_domain_ships::ShipKind::SmallMerchant,
            x: 0.0,
            y: 0.0,
            heading: 0.0,
            speed: 30.0,
            cargo_weight: 38,
            hp: 140,
            max_hp: 140,
            max_speed: 36.0,
            weapon_damage: 30,
            weapon_range: 55.0,
            port_cooldown_secs: 3.5,
            starboard_cooldown_secs: 1.25,
            is_npc: false,
            cargo_capacity: 100,
            sail_hp: 100.0,
            ammo: Ammo::Round,
            faction: Faction::Player,
            notoriety_tier: 0,
            rudder_hp: 100.0,
            crew: 0,
            crew_max: 0,
            repairing: false,
            dig_progress: 0.0,
            sail_cosmetic: 0,
            flag_cosmetic: 0,
            black_flag: 0,
            fire_target: None,
            aura: 0,
            flasks: Default::default(),
        };
        let bytes = bincode::serialize(&state).unwrap();
        let decoded = bincode::deserialize::<ShipState>(&bytes).unwrap();
        assert_eq!(decoded, state);
        assert_eq!(decoded.port_cooldown_secs, 3.5);
        assert_eq!(decoded.starboard_cooldown_secs, 1.25);
        assert_eq!(decoded.cargo_capacity, 100);
    }

    #[test]
    fn ship_state_roundtrips_is_npc_flag() {
        for is_npc in [false, true] {
            let state = ShipState {
                ship_id: 3,
                kind: marvyr_domain_ships::ShipKind::Patrol,
                x: 1.0,
                y: 2.0,
                heading: 0.5,
                speed: 0.0,
                cargo_weight: 0,
                hp: 70,
                max_hp: 70,
                max_speed: 40.0,
                weapon_damage: 25,
                weapon_range: 55.0,
                port_cooldown_secs: 0.0,
                starboard_cooldown_secs: 0.0,
                is_npc,
                cargo_capacity: 70,
                sail_hp: 100.0,
                ammo: Ammo::Round,
                faction: Faction::Navy,
                notoriety_tier: 0,
                rudder_hp: 100.0,
                crew: 0,
                crew_max: 0,
                repairing: false,
                dig_progress: 0.0,
                sail_cosmetic: 0,
                flag_cosmetic: 0,
                black_flag: 0,
                fire_target: None,
                aura: 0,
                flasks: Default::default(),
            };
            let bytes = bincode::serialize(&state).unwrap();
            let decoded = bincode::deserialize::<ShipState>(&bytes).unwrap();
            assert_eq!(decoded, state);
            assert_eq!(decoded.is_npc, is_npc);
            assert_eq!(decoded.cargo_capacity, 70);
        }
    }

    #[test]
    fn reputation_and_world_event_roundtrip() {
        let update = ReputationUpdate {
            notoriety: 320,
            tier: TIER_PROCURADO,
        };
        let bytes = bincode::serialize(&update).unwrap();
        assert_eq!(
            bincode::deserialize::<ReputationUpdate>(&bytes).unwrap(),
            update
        );
        let event = WorldEvent {
            text: String::from("CABECA A PRECO: 640g"),
            kind: WorldEventKind::Bounty,
        };
        let bytes = bincode::serialize(&event).unwrap();
        assert_eq!(bincode::deserialize::<WorldEvent>(&bytes).unwrap(), event);
    }

    #[test]
    fn loadout_messages_roundtrip() {
        let equip = EquipItem {
            item: ItemDefinitionId::new(),
            instance: Some(ItemInstanceId::new()),
        };
        let bytes = bincode::serialize(&equip).unwrap();
        assert_eq!(bincode::deserialize::<EquipItem>(&bytes).unwrap(), equip);

        let unequip = UnequipItem {
            slot: EquipmentSlot::Sail,
        };
        let bytes = bincode::serialize(&unequip).unwrap();
        assert_eq!(
            bincode::deserialize::<UnequipItem>(&bytes).unwrap(),
            unequip
        );

        let snapshot = LoadoutSnapshot {
            slots: vec![
                LoadoutLine {
                    slot: EquipmentSlot::Hull,
                    item_name: String::from("Casco Reforçado"),
                    equipped: true,
                    quality: marvyr_domain_items::roll_quality(Rarity::Rare, 1),
                    sockets: 0,
                    synergies: Vec::new(),
                },
                LoadoutLine {
                    slot: EquipmentSlot::Sail,
                    item_name: String::new(),
                    equipped: false,
                    quality: None,
                    sockets: 0,
                    synergies: Vec::new(),
                },
            ],
            sets: Vec::new(),
        };
        let bytes = bincode::serialize(&snapshot).unwrap();
        assert_eq!(
            bincode::deserialize::<LoadoutSnapshot>(&bytes).unwrap(),
            snapshot
        );
    }

    #[test]
    fn dock_messages_roundtrip() {
        let dock = Dock;
        let bytes = bincode::serialize(&dock).unwrap();
        assert_eq!(bincode::deserialize::<Dock>(&bytes).unwrap(), dock);

        let undock = Undock;
        let bytes = bincode::serialize(&undock).unwrap();
        assert_eq!(bincode::deserialize::<Undock>(&bytes).unwrap(), undock);

        let refused = DockResult {
            success: false,
            docked: false,
            reason: String::from("veloz demais para atracar"),
        };
        let bytes = bincode::serialize(&refused).unwrap();
        let decoded: DockResult = bincode::deserialize(&bytes).unwrap();
        assert!(!decoded.success);
        assert!(!decoded.docked);
    }

    #[test]
    fn port_storage_snapshot_roundtrips_through_bincode() {
        let message = PortStorageSnapshot {
            region: String::from("Porto da Serra"),
            lines: vec![
                StorageLine {
                    item: ItemDefinitionId::new(),
                    item_name: String::from("Madeira"),
                    quantity: 25,
                    instance: None,
                    quality: None,
                },
                StorageLine {
                    item: ItemDefinitionId::new(),
                    item_name: String::from("Casco Reforçado"),
                    quantity: 1,
                    instance: Some(ItemInstanceId::new()),
                    quality: marvyr_domain_items::roll_quality(Rarity::Magic, 2),
                },
            ],
            elsewhere: vec![StoredElsewhere {
                region: String::from("Porto da Mina"),
                quantity: 100,
            }],
        };
        let bytes = bincode::serialize(&message).unwrap();
        assert_eq!(
            bincode::deserialize::<PortStorageSnapshot>(&bytes).unwrap(),
            message
        );
    }

    /// MF-043 estendeu `ShipState` com `port_cooldown_secs` e
    /// `starboard_cooldown_secs`. Em Alpha, cliente e servidor são deploy
    /// juntos, então o caminho "cliente antigo ↔ servidor novo" não
    /// acontece. Este teste documenta o limite: bytes truncados (simulando
    /// servidor antigo falando com cliente novo) falham de forma
    /// recuperável, sem panic nem corrupção silenciosa.
    #[test]
    fn ship_state_truncated_bytes_fail_cleanly_not_silently() {
        let full = ShipState {
            ship_id: 1,
            kind: marvyr_domain_ships::ShipKind::SmallMerchant,
            x: 0.0,
            y: 0.0,
            heading: 0.0,
            speed: 0.0,
            cargo_weight: 0,
            hp: 100,
            max_hp: 100,
            max_speed: 0.0,
            weapon_damage: 0,
            weapon_range: 0.0,
            port_cooldown_secs: 2.5,
            starboard_cooldown_secs: 1.5,
            is_npc: false,
            cargo_capacity: 100,
            sail_hp: 100.0,
            ammo: Ammo::Round,
            faction: Faction::Player,
            notoriety_tier: 0,
            rudder_hp: 100.0,
            crew: 0,
            crew_max: 0,
            repairing: false,
            dig_progress: 0.0,
            sail_cosmetic: 0,
            flag_cosmetic: 0,
            black_flag: 0,
            fire_target: None,
            aura: 0,
            flasks: Default::default(),
        };
        let bytes = bincode::serialize(&full).expect("encode");
        // Trunca 8 bytes (dois f32): simula cliente novo lendo servidor antigo.
        let truncated = &bytes[..bytes.len() - 8];
        let decoded: Result<ShipState, _> = bincode::deserialize(truncated);
        assert!(
            decoded.is_err(),
            "truncation must surface as Err, not panic"
        );
    }

    #[test]
    fn client_hello_roundtrips() {
        let message = ClientHello::current("jogador-abc");
        let bytes = bincode::serialize(&message).unwrap();
        let decoded: ClientHello = bincode::deserialize(&bytes).unwrap();
        assert_eq!(decoded, message);
        assert_eq!(decoded.identity, "jogador-abc");
    }

    #[test]
    fn server_welcome_rejects_old_version() {
        let message = ServerWelcome {
            protocol_version: 99,
            accepted: false,
            reason: String::from("versão incompatível"),
        };
        let bytes = bincode::serialize(&message).unwrap();
        let decoded: ServerWelcome = bincode::deserialize(&bytes).unwrap();
        assert!(!decoded.accepted);
        assert_ne!(decoded.protocol_version, PROTOCOL_VERSION);
    }

    #[test]
    fn ship_input_roundtrips_and_clamps_are_server_side() {
        let message = ShipInput {
            throttle: 0.75,
            turn: -0.5,
        };
        let bytes = bincode::serialize(&message).unwrap();
        let decoded: ShipInput = bincode::deserialize(&bytes).unwrap();
        assert_eq!(decoded, message);
    }

    #[test]
    fn world_snapshot_roundtrips_with_aoi_entities() {
        let message = WorldSnapshot {
            tick: 42,
            ships: vec![
                ShipState {
                    ship_id: 1,
                    kind: marvyr_domain_ships::ShipKind::SmallMerchant,
                    x: 12.5,
                    y: -3.25,
                    heading: 0.1,
                    speed: 4.0,
                    cargo_weight: 36,
                    hp: 100,
                    max_hp: 100,
                    max_speed: 30.0,
                    weapon_damage: 20,
                    weapon_range: 50.0,
                    port_cooldown_secs: 0.0,
                    starboard_cooldown_secs: 2.0,
                    is_npc: false,
                    cargo_capacity: 100,
                    sail_hp: 100.0,
                    ammo: Ammo::Round,
                    faction: Faction::Player,
                    notoriety_tier: 2,
                    rudder_hp: 100.0,
                    crew: 0,
                    crew_max: 0,
                    repairing: false,
                    dig_progress: 0.0,
                    sail_cosmetic: 0,
                    flag_cosmetic: 0,
                    black_flag: 0,
                    fire_target: None,
                    aura: 0,
                    flasks: Default::default(),
                },
                ShipState {
                    ship_id: 2,
                    kind: marvyr_domain_ships::ShipKind::Corsair,
                    x: 0.0,
                    y: 0.0,
                    heading: 3.0,
                    speed: 0.0,
                    cargo_weight: 0,
                    hp: 70,
                    max_hp: 70,
                    max_speed: 40.0,
                    weapon_damage: 25,
                    weapon_range: 55.0,
                    port_cooldown_secs: 0.0,
                    starboard_cooldown_secs: 0.0,
                    is_npc: false,
                    cargo_capacity: 40,
                    sail_hp: 100.0,
                    ammo: Ammo::Round,
                    faction: Faction::Pirate,
                    notoriety_tier: 0,
                    rudder_hp: 100.0,
                    crew: 0,
                    crew_max: 0,
                    repairing: false,
                    dig_progress: 0.0,
                    sail_cosmetic: 0,
                    flag_cosmetic: 0,
                    black_flag: 0,
                    fire_target: None,
                    aura: 0,
                    flasks: Default::default(),
                },
            ],
            projectiles: vec![ProjectileState {
                projectile_id: 7,
                x: 1.0,
                y: 2.0,
                heading: 1.5,
            }],
            wrecks: vec![WreckState {
                wreck_id: 9,
                x: 30.0,
                y: -10.0,
                stack_count: 2,
            }],
        };
        let bytes = bincode::serialize(&message).unwrap();
        let decoded: WorldSnapshot = bincode::deserialize(&bytes).unwrap();
        assert_eq!(decoded, message);
        assert_eq!(decoded.wrecks[0].stack_count, 2);
    }

    #[test]
    fn loot_messages_roundtrip() {
        let loot = LootWreck { wreck_id: 9 };
        let bytes = bincode::serialize(&loot).unwrap();
        assert_eq!(bincode::deserialize::<LootWreck>(&bytes).unwrap(), loot);

        let result = LootResult {
            wreck_id: 9,
            success: false,
            reason: String::from("Longe demais do destroço: chegue mais perto."),
        };
        let bytes = bincode::serialize(&result).unwrap();
        assert_eq!(bincode::deserialize::<LootResult>(&bytes).unwrap(), result);
    }

    #[test]
    fn black_flag_and_lock_roundtrip() {
        for raise in [true, false] {
            let bytes = bincode::serialize(&SetBlackFlag { raise }).unwrap();
            assert_eq!(
                bincode::deserialize::<SetBlackFlag>(&bytes).unwrap(),
                SetBlackFlag { raise }
            );
        }
        let bytes = bincode::serialize(&LockTarget).unwrap();
        assert_eq!(
            bincode::deserialize::<LockTarget>(&bytes).unwrap(),
            LockTarget
        );
    }

    #[test]
    fn ship_destroyed_roundtrips() {
        let message = ShipDestroyed { ship_id: 3 };
        let bytes = bincode::serialize(&message).unwrap();
        let decoded: ShipDestroyed = bincode::deserialize(&bytes).unwrap();
        assert_eq!(decoded, message);
    }

    #[test]
    fn zone_changed_roundtrips_with_tier_and_name() {
        for tier in [
            marvyr_domain_world::RiskTier::Protected,
            marvyr_domain_world::RiskTier::Frontier,
            marvyr_domain_world::RiskTier::Lawless,
        ] {
            let message = ZoneChanged {
                ship_id: 7,
                tier,
                zone_name: String::from("Rota da Costa"),
            };
            let bytes = bincode::serialize(&message).unwrap();
            let decoded: ZoneChanged = bincode::deserialize(&bytes).unwrap();
            assert_eq!(decoded, message);
        }
    }

    #[test]
    fn node_messages_roundtrip() {
        let state = NodeState {
            node_id: 4,
            x: -430.0,
            y: 20.0,
            resource_name: String::from("Madeira"),
            stock: 50,
            max_stock: 60,
        };
        let snapshot = NodesSnapshot {
            nodes: vec![
                state.clone(),
                NodeState {
                    node_id: 5,
                    x: 0.0,
                    y: 900.0,
                    resource_name: String::from("Coral Negro"),
                    stock: 0,
                    max_stock: 30,
                },
            ],
        };
        let bytes = bincode::serialize(&snapshot).unwrap();
        let decoded: NodesSnapshot = bincode::deserialize(&bytes).unwrap();
        assert_eq!(decoded, snapshot);
        assert_eq!(decoded.nodes[1].stock, 0);

        let updated = NodeUpdated { node: state };
        let bytes = bincode::serialize(&updated).unwrap();
        assert_eq!(
            bincode::deserialize::<NodeUpdated>(&bytes).unwrap(),
            updated
        );

        let gather = GatherNode { node_id: 4 };
        let bytes = bincode::serialize(&gather).unwrap();
        assert_eq!(bincode::deserialize::<GatherNode>(&bytes).unwrap(), gather);

        let result = GatherResult {
            node_id: 4,
            success: true,
            gathered: 10,
            reason: String::new(),
        };
        let bytes = bincode::serialize(&result).unwrap();
        assert_eq!(
            bincode::deserialize::<GatherResult>(&bytes).unwrap(),
            result
        );
    }

    #[test]
    fn craft_messages_roundtrip_with_station_and_ingredients() {
        let entry = RecipeEntry {
            recipe_id: 3,
            display_name: String::from("Corsair"),
            station: marvyr_domain_crafting::recipe::StationKind::Dock,
            ship_build: true,
            output_name: String::from("Corsair"),
            output_quantity: 1,
            ingredients: vec![
                IngredientLine {
                    name: String::from("Minério"),
                    quantity: 40,
                },
                IngredientLine {
                    name: String::from("Coral Negro"),
                    quantity: 10,
                },
            ],
            magic: Vec::new(),
            rare: vec![IngredientLine {
                name: String::from("Coral Negro"),
                quantity: 2,
            }],
        };
        let snapshot = RecipesSnapshot {
            recipes: vec![entry],
        };
        let bytes = bincode::serialize(&snapshot).unwrap();
        let decoded: RecipesSnapshot = bincode::deserialize(&bytes).unwrap();
        assert_eq!(decoded, snapshot);
        assert_eq!(
            decoded.recipes[0].station,
            marvyr_domain_crafting::recipe::StationKind::Dock
        );

        let intent = CraftItem {
            recipe_id: 3,
            rarity: Rarity::Rare,
        };
        let bytes = bincode::serialize(&intent).unwrap();
        assert_eq!(bincode::deserialize::<CraftItem>(&bytes).unwrap(), intent);

        let result = CraftResult {
            recipe_id: 3,
            success: false,
            reason: String::from("Faltam materiais: 5 Minério."),
            quality: None,
        };
        let bytes = bincode::serialize(&result).unwrap();
        assert_eq!(bincode::deserialize::<CraftResult>(&bytes).unwrap(), result);
    }

    #[test]
    fn onboarding_progress_roundtrips() {
        let progress = OnboardingProgress {
            step: 3,
            skipped: true,
        };
        let bytes = bincode::serialize(&progress).unwrap();
        assert_eq!(
            bincode::deserialize::<OnboardingProgress>(&bytes).unwrap(),
            progress
        );
    }
}
