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
/// v30: `WreckState.best_rarity` (feixe de luz do destroço pela raridade).
/// v31: `ShipState.elite` (afixos de pirata de elite, bitmask).
/// v32: Maré Sangrenta — `SeaEventKind::BloodTide`, `ActionKind::CursedChest`
/// e `SeaEventState.chests`.
/// v33: aspectos lendários — `Quality.aspect` e `OrbKind::Seal` (Selo).
/// v34: chefe de mundo — `SeaEventKind::WorldBoss` e o bit de chefe
/// (`elite::BOSS`, 0x80) em `ShipState.elite`.
/// v35: Diário de Bordo — `ProgressSnapshot` (metas do dia e da semana e
/// recompensas esperando o porto), registrada no fim.
/// v36: Fúria do Mar — `ShipState.fury` (afundamentos seguidos).
/// v37: veio dourado — `NodeState.golden` (nó que rende 5x).
/// v38: Carga Amaldiçoada — `SeaEventKind::CursedCargo` (quem carrega,
/// visto por todos) e `ActionKind::CursedCargo` (entrega no porto).
/// v39: pesca — `CastLine` (registrada no fim) e `ActionKind::FishCast`,
/// `FishBite` e `Fish`.
/// v40: o Abismo — `SeaEventKind::Abyss`, `ActionKind::Abyss` e
/// `ProgressSnapshot.abyss_best`.
/// v41: Livro de Bordo — `ProgressSnapshot.found` e `ShipState.title`
/// (título conquistado, catálogo `TITLES`).
/// v42: maestria de casco — `ProgressSnapshot.mastery` (experiência por
/// casco); o título de mestre entra no mesmo `ShipState.title`.
/// v43: temporadas — `SeasonBoard` (registrada no fim) e
/// `ProgressSnapshot.season_points`/`crowns`.
/// v44: caçadas — `BountyBoard` (registrada no fim).
/// v45: influência de porto — `SeasonBoard.lords` (Senhor de cada porto
/// disputado) e `ProgressSnapshot.influence`.
/// v46: faróis de jogador — `LighthousesUpdate` e `RaiseLighthouse`
/// (registradas no fim) e `ActionKind::Lighthouse`.
/// v47: moral da tripulação — `ShipState.morale` e `ActionKind::Morale`.
/// v48: correntes marítimas da semana — `SeaCurrents` (registrada no fim).
/// v49: escassez viva — `GuildPriceLine.scarce` (item em falta por porto).
/// v50: histórico do navio — `ProgressSnapshot.history` e `ShipLogCard`
/// (registrada no fim).
/// v51: mensagem na garrafa — `ThrowBottle`, `PickBottle`, `BottlesUpdate`
/// e `BottleRead` (registradas no fim) e `ActionKind::Bottle`.
/// v52: frete entre jogadores — `FreightBoard`, `PostFreight`,
/// `AcceptFreight` e `CancelFreight` (registradas no fim).
/// v53: peça exata no escambo — `CreateSellOrder.instance` e
/// `OrderLine.quality`; `TreasureHint.bonus_pct` (o bônus do mapa vem do
/// servidor); `fury::PER_POINT_PCT` compartilhado.
/// v54: combate ativo — `CombatAction` (registrada no fim: salva mirada,
/// abalroar, leque e barril), `ShipState.ram_cooldown_secs` e
/// `.skill_cooldowns`, `ProjectileState.kind` (bala ou barril).
/// v55: quem é quem e party — `ShipState.npc_kind`, `ShipNames`,
/// `TakeoverRequest` (derrubar a outra sessão), `PartyInvite`,
/// `PartyAnswer`, `PartyLeave` e `PartyUpdate` (registradas no fim).
/// v56: inimigos com mecânica — `ShipState.telegraph` (tiro pesado
/// anunciado da elite) e `npc_kind` 11-13 (brulote, artilheiro, calafate).
/// v57: gemas que mudam a skill — `ShipState.skill_variants` e
/// `ProjectileState.kind` 2 (bala incendiária).
/// v58: cascos novos — `ShipKind` Brig, Galleon e Bombard no fio, e os
/// títulos de maestria deles (códigos 9-11).
/// v59: oficiais de bordo — `ShipState.officers` e `HireOfficer`
/// (registrada no fim).
/// v60: fortaleza pirata — `npc_kind` 14 (o client desenha pedra, não casco).
/// v61: abordagem em duelo de táticas — `BoardTactic` e `MeleeUpdate`
/// (registradas no fim).
/// v62: companhias e guerra de território — `CreateCompany`,
/// `CompanyInvite`, `CompanyAnswer`, `LeaveCompany` e `CompanyUpdate`
/// (registradas no fim).
pub const PROTOCOL_VERSION: u16 = 62;

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
/// O capitão já está em mar por outra sessão (o client oferece derrubá-la).
pub const REASON_ALREADY_AT_SEA: &str =
    "Seu capitão já está conectado em outra sessão. Feche o outro jogo e tente de novo.";
/// v55: esta sessão foi derrubada por um login do mesmo capitão.
pub const REASON_TAKEN_OVER: &str = "Seu capitão entrou por outro jogo; esta sessão foi encerrada.";
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

/// v35: uma meta do Diário de Bordo. `template` é PT-BR com `{0}` = alvo
/// (o client traduz com `trf`).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct GoalLine {
    pub template: String,
    pub target: u32,
    pub progress: u32,
    pub reward_item: String,
    pub reward_quantity: u32,
    pub weekly: bool,
}

/// v35: o Diário do capitão — metas de hoje, a da semana e o que já foi
/// cumprido e espera o próximo porto. Chega ao conectar e a cada avanço.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ProgressSnapshot {
    pub goals: Vec<GoalLine>,
    pub unpaid: Vec<(String, u32)>,
    /// v40: camada mais funda que o capitão já venceu no Abismo.
    #[serde(default)]
    pub abyss_best: u32,
    /// v41: entradas do Livro de Bordo já registradas.
    #[serde(default)]
    pub found: Vec<String>,
    /// v42: experiência de maestria por casco (nome do casco, PT-BR).
    #[serde(default)]
    pub mastery: Vec<(String, u32)>,
    /// v43: pontos na temporada em curso e coroas já levadas.
    #[serde(default)]
    pub season_points: u32,
    #[serde(default)]
    pub crowns: u32,
    /// v45: influência do capitão por porto disputado nesta semana.
    #[serde(default)]
    pub influence: Vec<(String, u32)>,
    /// v50: folha de serviço por casco: (casco, viagens, afundados, metros).
    #[serde(default)]
    pub history: Vec<(String, u32, u32, u32)>,
}

/// v50: a folha de serviço do navio de outro capitão, mandada a quem trava
/// o alvo nele. Só vaidade.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ShipLogCard {
    pub ship_id: u32,
    pub captain: String,
    pub hull: String,
    pub voyages: u32,
    pub sinks: u32,
    pub meters: u32,
    /// Código de `TITLES` (0 = nenhum).
    pub title: u8,
}

/// v44: uma cabeça a prêmio — o capitão Procurado, a zona onde está e o
/// que a coroa paga (bruto) a quem afundá-lo.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct BountyLine {
    pub captain: String,
    pub zone: String,
    pub reward: Vec<(String, u32)>,
}

/// v44: quadro de cabeças a prêmio (~10 s, todo mundo).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct BountyBoard {
    pub entries: Vec<BountyLine>,
}

/// v43: a temporada em curso para todo mundo (~10 s): tema, dias até
/// virar e os dez capitães com mais pontos.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SeasonBoard {
    pub season: u32,
    pub theme: String,
    pub theme_description: String,
    pub days_left: u32,
    pub crown_at: u32,
    pub top: Vec<(String, u32)>,
    /// v45: Senhor de cada porto disputado: (porto, capitão, influência).
    #[serde(default)]
    pub lords: Vec<(String, String, u32)>,
}

/// v46: farol erguido por um capitão. Clareia a noite em volta, aparece
/// na carta e rende Renome a quem ergueu quando outro capitão passa.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct LighthouseLine {
    pub id: u32,
    pub x: f32,
    pub y: f32,
    /// Horas até apagar (reforçar soma).
    pub hours_left: u32,
    pub builder: String,
}

/// v46: todos os faróis acesos (no connect e a cada mudança).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct LighthousesUpdate {
    pub list: Vec<LighthouseLine>,
}

/// v46: erguer um farol aqui (perto da costa) ou, colado num farol aceso,
/// reforçá-lo. O servidor decide qual e cobra do porão.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct RaiseLighthouse;

/// v46: geometria do farol, a mesma no servidor e no bilhete do client.
/// v53: Fúria do Mar — o servidor aplica, o HUD mostra o mesmo número.
pub mod fury {
    /// Butim a mais (%) por ponto de fúria.
    pub const PER_POINT_PCT: u32 = 10;
}

pub mod lighthouse {
    /// Até esta distância da terra dá para erguer (m).
    pub const COAST: f32 = 120.0;
    /// Distância mínima entre dois faróis (m).
    pub const SPACING: f32 = 500.0;
    /// Raio da luz (m).
    pub const LIGHT: f32 = 450.0;
    /// Colado no farol (m): o bilhete vira "reforçar".
    pub const TEND: f32 = 90.0;
}

/// v48: as correntes da semana, cada uma (x0, y0, x1, y1) no sentido em que
/// empurra. Chega ao conectar e na virada da semana.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct SeaCurrents {
    pub lanes: Vec<(f32, f32, f32, f32)>,
}

/// v51: frases da garrafa, montadas de três peças prontas (sem texto
/// livre: nada a moderar). O client traduz peça por peça.
pub mod bottle {
    pub const OPENINGS: [&str; 6] = [
        "Cuidado com",
        "Achei",
        "Procuro",
        "Evite",
        "Vale a pena",
        "Boa sorte com",
    ];
    pub const SUBJECTS: [&str; 10] = [
        "piratas",
        "o Kraken",
        "um veio dourado",
        "Peixe-Lanterna",
        "a tempestade",
        "um tesouro",
        "um farol",
        "carga amaldiçoada",
        "a Marinha",
        "companhia",
    ];
    pub const PLACES: [&str; 8] = [
        "ao norte",
        "ao sul",
        "a leste",
        "a oeste",
        "aqui perto",
        "no porto",
        "no mar sem lei",
        "na corrente",
    ];

    /// As três peças da frase (índices fora do catálogo: `None`).
    pub fn pieces(words: [u8; 3]) -> Option<[&'static str; 3]> {
        Some([
            *OPENINGS.get(usize::from(words[0]))?,
            *SUBJECTS.get(usize::from(words[1]))?,
            *PLACES.get(usize::from(words[2]))?,
        ])
    }
}

/// v51: jogar uma garrafa ao mar com a frase `words` (índices de
/// [`bottle`]).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct ThrowBottle {
    pub words: [u8; 3],
}

/// v51: pescar a garrafa mais perto (o servidor escolhe).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct PickBottle;

/// v51: garrafas boiando (id, x, y) — todo mundo vê.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct BottlesUpdate {
    pub list: Vec<(u32, f32, f32)>,
}

/// v51: a frase da garrafa pescada e quem a jogou.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct BottleRead {
    pub words: [u8; 3],
    pub author: String,
}

/// v52: um frete no quadro. Quantidades em unidades; a caução é no item
/// da carga.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct FreightLine {
    pub num: u32,
    pub origin: String,
    pub dest: String,
    pub cargo_item: String,
    pub cargo_qty: u32,
    pub reward_item: String,
    pub reward_qty: u32,
    pub collateral: u32,
    pub poster: String,
    /// Anunciado por quem recebe o quadro.
    pub mine: bool,
    /// Quem recebe o quadro é o transportador.
    pub carrying: bool,
    pub in_transit: bool,
    pub minutes_left: u32,
}

/// v52: quadro de fretes para quem está atracado: os abertos deste porto
/// e os seus (anunciados ou levando) em qualquer lugar. `ports` são os
/// destinos possíveis.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct FreightBoard {
    pub lines: Vec<FreightLine>,
    pub ports: Vec<String>,
}

/// v52: anunciar um frete daqui para `dest` (nome do porto): carga e prêmio
/// saem do armazém deste porto; a caução é no item da carga.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PostFreight {
    pub dest: String,
    pub cargo_item: ItemDefinitionId,
    pub cargo_qty: u32,
    pub reward_item: ItemDefinitionId,
    pub reward_qty: u32,
    pub collateral: u32,
}

/// v52: levar o frete `num` (a caução sai do seu armazém daqui).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct AcceptFreight {
    pub num: u32,
}

/// v52: desistir do seu frete ainda sem transportador.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct CancelFreight {
    pub num: u32,
}

/// Ticks do servidor (30 Hz) num dia inteiro: 20 minutos.
pub const DAY_TICKS: u64 = 36_000;

/// Quão noite é no tick dado (0 dia claro, 1 meia-noite). O dia começa no
/// tick 0; a noite ocupa cerca de um terço do ciclo. v47: o servidor usa
/// para a moral da tripulação; o client, para o véu.
pub fn night_of(tick: u64) -> f32 {
    let phase = (tick % DAY_TICKS) as f32 / DAY_TICKS as f32;
    let n = 0.5 - 0.5 * (phase * std::f32::consts::TAU).cos();
    ((n - 0.75) / 0.25).clamp(0.0, 1.0)
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
    /// v31: afixos de elite do NPC (`marvyr_domain_combat::elite`; 0 = comum).
    #[serde(default)]
    pub elite: u8,
    /// v36: Fúria do Mar (0-10) — afundamentos seguidos sem atracar.
    #[serde(default)]
    pub fury: u8,
    /// v41: título à mostra (código de `TITLES`; 0 = nenhum). Só aparência.
    #[serde(default)]
    pub title: u8,
    /// v47: moral da tripulação (0..100). Baixa, o navio anda menos.
    #[serde(default = "full_morale")]
    pub morale: u8,
    /// v54: recarga do abalroar (s) e abalroando agora (rastro no client).
    #[serde(default)]
    pub ram_cooldown_secs: f32,
    #[serde(default)]
    pub ramming: bool,
    /// v54: recarga das skills (leque, barril), em segundos.
    #[serde(default)]
    pub skill_cooldowns: [f32; 2],
    /// v55: tipo do NPC para a placa (0 = jogador; `npc_kind_name`).
    #[serde(default)]
    pub npc_kind: u8,
    /// v56: tiro pesado anunciado — (x, y, progresso 0..1 até cair).
    #[serde(default)]
    pub telegraph: Option<(f32, f32, f32)>,
    /// v57: variantes das skills pelas gemas (leque, barril, abalroar;
    /// `SkillVariants::wire`).
    #[serde(default)]
    pub skill_variants: [u8; 3],
    /// v59: oficiais a bordo (máscara: 1 artilheiro, 2 contramestre,
    /// 4 cirurgião).
    #[serde(default)]
    pub officers: u8,
}

/// v56: raio do tiro pesado anunciado da elite (regra no servidor, círculo
/// no client).
pub const HEAVY_SHOT_RADIUS: f32 = 65.0;
/// v56: raio da explosão do brulote (regra no servidor, anel no client).
pub const FIRESHIP_BLAST_RADIUS: f32 = 70.0;

/// v55: nome da placa de um NPC pelo `ShipState.npc_kind`.
/// v60: `npc_kind` da fortaleza pirata (o client desenha pedra).
pub const NPC_KIND_FORT: u8 = 14;

pub fn npc_kind_name(code: u8) -> Option<&'static str> {
    Some(match code {
        1 => "Corsário",
        2 => "Chalupa Pirata",
        3 => "Navio da Marinha",
        4 => "Mercador",
        5 => "Kraken",
        6 => "Galeão do Tesouro",
        7 => "Escolta da Coroa",
        8 => "Guardião do Tesouro",
        9 => "Saqueador da Maré",
        10 => "Leviatã",
        11 => "Brulote",
        12 => "Artilheiro Pirata",
        13 => "Calafate Pirata",
        14 => "Fortaleza Pirata",
        _ => return None,
    })
}

/// v55: nome do capitão de cada navio de jogador (a cada 2 s, para todos).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ShipNames {
    pub names: Vec<(u32, String)>,
}

/// v55: mandada ANTES do `ClientHello` (mesmo canal ordenado): se o
/// capitão já estiver em mar por outra sessão, derruba a outra.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct TakeoverRequest;

/// v55: convida o capitão deste navio para a party.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct PartyInvite {
    pub target_ship_id: u32,
}

/// v55: responde ao convite pendente.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct PartyAnswer {
    pub accept: bool,
}

/// v55: sai da party.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct PartyLeave;

/// v55: um companheiro de party.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PartyMember {
    /// 0 = sem navio no mar agora.
    pub ship_id: u32,
    pub name: String,
    pub hp: u32,
    pub max_hp: u32,
}

/// v55: a party de quem recebe (vazia = sozinho) e o convite pendente.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct PartyUpdate {
    pub members: Vec<PartyMember>,
    pub invite_from: Option<String>,
}

/// v59: contratar oficial no porto (bit de `ShipState.officers`), pago com
/// recurso do armazém.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct HireOfficer {
    pub officer: u8,
}

/// v62: fundar uma companhia (atracado; a tag sai das iniciais do nome).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct CreateCompany {
    pub name: String,
}

/// v62: convidar para a companhia um capitão atracado no mesmo porto.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct CompanyInvite {
    pub target_ship_id: u32,
}

/// v62: responder ao convite de companhia.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct CompanyAnswer {
    pub accept: bool,
}

/// v62: sair da companhia (o último a sair a desfaz).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct LeaveCompany;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct CompanyMember {
    pub name: String,
    pub online: bool,
    pub leader: bool,
}

/// v62: frente de guerra de um porto disputado.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct WarFront {
    pub port: String,
    pub x: f32,
    pub y: f32,
    /// Quem manda no porto nesta semana ("" = ninguém).
    pub holder: String,
    /// Janela de guerra aberta agora.
    pub open: bool,
    /// Segundos até fechar (aberta) ou abrir (fechada).
    pub secs: f32,
    /// Influência da minha companhia (ou minha, sem companhia) e a do
    /// primeiro colocado que não sou eu.
    pub mine: u32,
    pub rival: u32,
}

/// v62: minha companhia, o convite pendente, quem pode ser convidado aqui
/// e as frentes de guerra.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct CompanyUpdate {
    /// Vazio = sem companhia.
    pub name: String,
    pub tag: String,
    pub leader: bool,
    pub members: Vec<CompanyMember>,
    /// Portos que a companhia segura nesta semana.
    pub ports: Vec<String>,
    /// Companhia que me convidou.
    pub invite_from: Option<String>,
    /// Capitães sem companhia atracados no mesmo porto: (navio, nome).
    pub docked_here: Vec<(u32, String)>,
    pub wars: Vec<WarFront>,
}

/// v61: tática da rodada no duelo de abordagem (1 Assalto, 2 Mosquete,
/// 3 Muralha).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct BoardTactic {
    pub tactic: u8,
}

/// v61: estado do duelo de abordagem visto por um dos lados (`active`
/// falso = acabou; o painel fecha).
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct MeleeUpdate {
    pub active: bool,
    /// Eu abordo (verdadeiro) ou defendo.
    pub attacker: bool,
    /// Rodada atual (1..=3) e segundos para escolher.
    pub round: u8,
    pub secs_left: f32,
    pub my_crew: u16,
    pub their_crew: u16,
    pub my_wins: u8,
    pub their_wins: u8,
    /// O que o NPC anuncia para esta rodada (0 = nada; jogador não anuncia).
    pub hint: u8,
    /// Minha escolha nesta rodada (0 = ainda nenhuma).
    pub my_pick: u8,
    /// Última rodada: (minha, deles, 1 venci / 0 empate / -1 perdi).
    pub last: Option<(u8, u8, i8)>,
    /// Fim: o navio rendeu (visto do atacante) ou segurou.
    pub captured: bool,
}

fn full_morale() -> u8 {
    100
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

/// v39: tecla de pesca (Espaço): lança a linha ou, com o peixe mordendo,
/// puxa. O servidor decide o tempo da mordida e a janela.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct CastLine;

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
    /// v32: Baú Maldito aberto (ou recusado) na Maré Sangrenta.
    CursedChest,
    /// v38: Carga Amaldiçoada entregue no porto.
    CursedCargo,
    /// v39: linha lançada (a boia vai para a água).
    FishCast,
    /// v39: o peixe mordeu — puxe já.
    FishBite,
    /// v39: fim da pescaria (peixe no porão, ou escapou).
    Fish,
    /// v40: descida no Abismo (camada vencida, ou o Abismo cuspiu).
    Abyss,
    /// v46: farol erguido ou reforçado (ou recusado).
    Lighthouse,
    /// v47: a tripulação comeu (sucesso) ou desanimou (aviso).
    Morale,
    /// v51: garrafa jogada ou pescada (ou recusada).
    Bottle,
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
    /// v32: Maré Sangrenta (ondas de elite, cinzas e baús malditos).
    BloodTide,
    /// v34: Leviatã, chefe de mundo agendado (contagem e depois o monstro).
    WorldBoss,
    /// v38: navio levando Carga Amaldiçoada (posição do navio, sem prazo).
    CursedCargo,
    /// v40: a Boca do Abismo (sempre) e cada descida em curso.
    Abyss,
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
    /// v32: Baús Malditos ainda fechados (só na Maré Sangrenta).
    #[serde(default)]
    pub chests: Vec<(f32, f32)>,
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
    /// v53: bônus do tesouro (%) que o servidor paga com esses perigos.
    #[serde(default)]
    pub bonus_pct: u32,
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

/// v54: ação de combate do capitão. `aim_x/aim_y` = ponto do mar sob o
/// cursor (ou o alvo do auto no controle); o servidor valida recarga,
/// águas e alcance, e decide o bordo.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub struct CombatAction {
    pub kind: marvyr_domain_combat::CombatActionKind,
    pub aim_x: f32,
    pub aim_y: f32,
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
    /// v54: 0 bala, 1 barril incendiário, 2 bala incendiária (v57;
    /// `ProjectileKind::wire`).
    #[serde(default)]
    pub kind: u8,
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
    /// v30: raridade da melhor peça do baú (0 Normal, 1 Mágica, 2 Rara):
    /// a cor do feixe de luz.
    pub best_rarity: u8,
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
    /// v37: veio dourado — rende 5x até esgotar (todos veem o brilho).
    #[serde(default)]
    pub golden: bool,
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
    /// v53: raridade e afixos da peça oferecida (equipamento).
    #[serde(default)]
    pub quality: Option<Quality>,
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
    /// v53: a peça exata do armazém (equipamento difere por afixos).
    #[serde(default)]
    pub instance: Option<ItemInstanceId>,
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
    /// v49: em falta em cada porto (mesma ordem de `per_ten`): entregar
    /// rende Renome.
    #[serde(default)]
    pub scarce: Vec<bool>,
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
    fn current_protocol_version_is_sixty_two() {
        assert_eq!(PROTOCOL_VERSION, 62);
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
            elite: 0,
            fury: 0,
            title: 0,
            morale: 100,
            ram_cooldown_secs: 0.0,
            ramming: false,
            skill_cooldowns: [0.0; 2],
            npc_kind: 0,
            telegraph: None,
            skill_variants: [0; 3],
            officers: 0,
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
                elite: 0,
                fury: 0,
                title: 0,
                morale: 100,
                ram_cooldown_secs: 0.0,
                ramming: false,
                skill_cooldowns: [0.0; 2],
                npc_kind: 0,
                telegraph: None,
                skill_variants: [0; 3],
                officers: 0,
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
            elite: 0,
            fury: 0,
            title: 0,
            morale: 100,
            ram_cooldown_secs: 0.0,
            ramming: false,
            skill_cooldowns: [0.0; 2],
            npc_kind: 0,
            telegraph: None,
            skill_variants: [0; 3],
            officers: 0,
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
                    elite: 0,
                    fury: 0,
                    title: 0,
                    morale: 100,
                    ram_cooldown_secs: 0.0,
                    ramming: false,
                    skill_cooldowns: [0.0; 2],
                    npc_kind: 0,
                    telegraph: None,
                    skill_variants: [0; 3],
                    officers: 0,
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
                    elite: 0,
                    fury: 0,
                    title: 0,
                    morale: 100,
                    ram_cooldown_secs: 0.0,
                    ramming: false,
                    skill_cooldowns: [0.0; 2],
                    npc_kind: 0,
                    telegraph: None,
                    skill_variants: [0; 3],
                    officers: 0,
                },
            ],
            projectiles: vec![ProjectileState {
                projectile_id: 7,
                x: 1.0,
                y: 2.0,
                heading: 1.5,
                kind: 0,
            }],
            wrecks: vec![WreckState {
                wreck_id: 9,
                x: 30.0,
                y: -10.0,
                stack_count: 2,
                best_rarity: 0,
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
            golden: false,
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
                    golden: false,
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
