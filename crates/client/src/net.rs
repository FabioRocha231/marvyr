//! Networking do client (PRD MF-006, ADR-0002/0003).
//!
//! O client é uma janela sobre a verdade do servidor: envia `ShipInput`
//! (intenção) e desenha o `WorldSnapshot` (realidade). Nada de física aqui —
//! client prediction/interpolação fina entram depois (Phase 2).
//!
//! Nota: canais e registros de mensagens devem ser um espelho exato do
//! `server/src/net.rs`.

use std::collections::HashMap;
use std::net::{IpAddr, Ipv4Addr, SocketAddr};
use std::time::Duration;

use bevy::ecs::prelude::*;
use bevy::prelude::*;
use lightyear::prelude::client::*;
use lightyear::prelude::*;
use marvyr_domain_items::EquipmentSlot;
use marvyr_domain_ships::ShipKind;
use marvyr_protocol::{
    AssignShip, BuySellOrder, CancelSellOrder, CatalogSnapshot, ClientHello, CraftItem,
    CraftResult, CreateSellOrder, Dock, DockResult, EquipItem, GatherNode, GatherResult,
    LoadoutResult, LoadoutSnapshot, LockTarget, LootResult, LootWreck, MarketResult, NodeUpdated,
    NodesSnapshot, OrdersSnapshot, PortStorageSnapshot, RecipesSnapshot, ServerWelcome,
    SetBlackFlag, ShipDestroyed, ShipInput, StorageDepositAll, StorageWithdrawAll, Undock,
    UnequipItem, WorldSnapshot, ZoneChanged, PROTOCOL_VERSION,
};

/// Socket local em todas as interfaces (MV-061: servidor remoto). Porta 0:
/// o SO escolhe a efêmera — permite vários clients na mesma máquina.
const CLIENT_ADDR: SocketAddr = SocketAddr::new(IpAddr::V4(Ipv4Addr::UNSPECIFIED), 0);

/// Config de netcode (UDP) para um servidor já resolvido. No browser vira
/// só o valor inicial do plugin: a conexão de verdade usa
/// [`web_netcode_config`].
pub fn netcode_config(server_addr: SocketAddr) -> NetConfig {
    #[cfg(not(target_arch = "wasm32"))]
    let transport = ClientTransport::UdpSocket(CLIENT_ADDR);
    #[cfg(target_arch = "wasm32")]
    let transport = ClientTransport::Dummy;
    netcode_with(server_addr, transport)
}

/// Browser: WebTransport (QUIC) com o certificado do servidor fixado pelo
/// hash SHA-256 em hex que o `marvyr-auth` publica.
#[cfg(target_arch = "wasm32")]
pub fn web_netcode_config(server_addr: SocketAddr, certificate_digest: String) -> NetConfig {
    netcode_with(
        server_addr,
        ClientTransport::WebTransportClient {
            client_addr: CLIENT_ADDR,
            server_addr,
            certificate_digest,
        },
    )
}

/// O `client_id` é aleatório: dois jogadores em máquinas diferentes nunca
/// colidem (o id de processo colidia).
fn netcode_with(server_addr: SocketAddr, transport: ClientTransport) -> NetConfig {
    NetConfig::Netcode {
        auth: Authentication::Manual {
            server_addr,
            client_id: rand_client_id(),
            private_key: marvyr_protocol::NETCODE_KEY,
            protocol_id: marvyr_protocol::NETCODE_PROTOCOL_ID,
        },
        io: IoConfig {
            transport,
            ..default()
        },
        config: NetcodeConfig {
            client_timeout_secs: 10,
            ..default()
        },
    }
}

fn rand_client_id() -> u64 {
    uuid::Uuid::new_v4().as_u64_pair().0
}
const SIM_HZ: f64 = 30.0;

/// Espelho do canal confiável do servidor.
#[derive(Channel)]
pub struct ReliableChannel;

/// Espelho do canal não-confiável do servidor.
#[derive(Channel)]
pub struct UnreliableChannel;

fn shared_config() -> SharedConfig {
    SharedConfig {
        server_replication_send_interval: Duration::from_secs_f64(1.0 / SIM_HZ),
        client_replication_send_interval: Duration::from_secs_f64(1.0 / SIM_HZ),
        tick: TickConfig {
            tick_duration: Duration::from_secs_f64(1.0 / SIM_HZ),
        },
    }
}

/// Navio atribuído a este client (destacado no visual).
#[derive(Resource, Debug, Clone, Copy, Default)]
pub struct MyShip(pub Option<u32>);

/// Tipo do casco do PRÓPRIO navio, vindo do `AssignShip` (MF-039: a UI de
/// loadout usa os slots aceitos para filtrar itens do storage).
#[derive(Resource, Debug, Clone, Copy, Default)]
pub struct KnownShipKind(pub Option<ShipKind>);

/// Configuração de rede injetada pelos testes. Quando ausente, o plugin usa
/// UDP e a identidade persistente normal do client.
#[derive(Resource, Default)]
pub struct ClientNetOverride(pub Option<NetConfig>);

/// Identidade enviada no `ClientHello`: o JWT da conta (login) ou, em dev
/// sem serviço de contas, o token anônimo persistente. Os testes injetam.
#[derive(Resource, Default)]
pub struct ClientIdentity(pub Option<String>);

/// Input fixo usado pelos testes para dirigir um client simulado pelo canal
/// de rede real, em vez de depender do teclado.
#[derive(Resource, Default)]
pub struct ShipInputOverride(pub Option<ShipInput>);

pub struct ClientNetPlugin;

impl Plugin for ClientNetPlugin {
    fn build(&self, app: &mut App) {
        let overridden = app
            .world()
            .get_resource::<ClientNetOverride>()
            .and_then(|override_config| override_config.0.clone());
        // Sem override, o endereço real só existe depois do login/DNS
        // (`session::start_connection` regrava o `ClientConfig`).
        let net_config = overridden.clone().unwrap_or_else(|| {
            netcode_config(SocketAddr::new(IpAddr::V4(Ipv4Addr::LOCALHOST), 5000))
        });
        app.init_resource::<crate::session::ConnectionStatus>();
        if overridden.is_some() {
            // Testes: transporte e identidade injetados; conecta no boot.
            app.insert_resource(crate::session::ConnectImmediately);
            app.add_systems(Startup, connect_now);
        }
        app.init_resource::<ClientNetOverride>();
        app.init_resource::<ClientIdentity>();
        app.init_resource::<ShipInputOverride>();
        app.init_resource::<MyShip>();
        app.init_resource::<KnownShipKind>();
        app.init_resource::<crate::nodes::KnownNodes>();
        app.init_resource::<crate::market::KnownCatalog>();
        app.add_plugins(ClientPlugins::new(ClientConfig {
            shared: shared_config(),
            net: net_config,
            ..default()
        }));
        app.add_channel::<ReliableChannel>(ChannelSettings {
            mode: ChannelMode::OrderedReliable(ReliableSettings::default()),
            ..default()
        });
        app.add_channel::<UnreliableChannel>(ChannelSettings {
            mode: ChannelMode::UnorderedUnreliable,
            ..default()
        });
        // Handshake PRIMEIRO (ids de rede 0 e 1, congelados desde o v15):
        // espelho exato do servidor.
        app.register_message::<ClientHello>(ChannelDirection::ClientToServer);
        app.register_message::<ServerWelcome>(ChannelDirection::ServerToClient);
        app.register_message::<ShipInput>(ChannelDirection::ClientToServer);
        app.register_message::<Dock>(ChannelDirection::ClientToServer);
        app.register_message::<Undock>(ChannelDirection::ClientToServer);
        app.register_message::<EquipItem>(ChannelDirection::ClientToServer);
        app.register_message::<UnequipItem>(ChannelDirection::ClientToServer);
        app.register_message::<marvyr_protocol::SelectAmmo>(ChannelDirection::ClientToServer);
        app.register_message::<LootWreck>(ChannelDirection::ClientToServer);
        app.register_message::<GatherNode>(ChannelDirection::ClientToServer);
        app.register_message::<CraftItem>(ChannelDirection::ClientToServer);
        app.register_message::<StorageDepositAll>(ChannelDirection::ClientToServer);
        app.register_message::<StorageWithdrawAll>(ChannelDirection::ClientToServer);
        app.register_message::<CreateSellOrder>(ChannelDirection::ClientToServer);
        app.register_message::<CancelSellOrder>(ChannelDirection::ClientToServer);
        app.register_message::<BuySellOrder>(ChannelDirection::ClientToServer);
        app.register_message::<AssignShip>(ChannelDirection::ServerToClient);
        app.register_message::<DockResult>(ChannelDirection::ServerToClient);
        app.register_message::<LoadoutSnapshot>(ChannelDirection::ServerToClient);
        app.register_message::<LoadoutResult>(ChannelDirection::ServerToClient);
        app.register_message::<WorldSnapshot>(ChannelDirection::ServerToClient);
        app.register_message::<marvyr_protocol::PortalsUpdate>(ChannelDirection::ServerToClient);
        app.register_message::<ShipDestroyed>(ChannelDirection::ServerToClient);
        app.register_message::<LootResult>(ChannelDirection::ServerToClient);
        app.register_message::<ZoneChanged>(ChannelDirection::ServerToClient);
        app.register_message::<NodesSnapshot>(ChannelDirection::ServerToClient);
        app.register_message::<NodeUpdated>(ChannelDirection::ServerToClient);
        app.register_message::<GatherResult>(ChannelDirection::ServerToClient);
        app.register_message::<RecipesSnapshot>(ChannelDirection::ServerToClient);
        app.register_message::<CraftResult>(ChannelDirection::ServerToClient);
        app.register_message::<CatalogSnapshot>(ChannelDirection::ServerToClient);
        app.register_message::<OrdersSnapshot>(ChannelDirection::ServerToClient);
        app.register_message::<PortStorageSnapshot>(ChannelDirection::ServerToClient);
        app.register_message::<MarketResult>(ChannelDirection::ServerToClient);
        app.register_message::<marvyr_protocol::SellToGuild>(ChannelDirection::ClientToServer);
        app.register_message::<marvyr_protocol::AcceptContract>(ChannelDirection::ClientToServer);
        app.register_message::<marvyr_protocol::AbandonContract>(ChannelDirection::ClientToServer);
        app.register_message::<marvyr_protocol::GuildPrices>(ChannelDirection::ServerToClient);
        app.register_message::<marvyr_protocol::ContractsSnapshot>(
            ChannelDirection::ServerToClient,
        );
        app.register_message::<marvyr_protocol::ContractResult>(ChannelDirection::ServerToClient);
        app.register_message::<marvyr_protocol::WeatherUpdate>(ChannelDirection::ServerToClient);
        app.register_message::<marvyr_protocol::ReputationUpdate>(ChannelDirection::ServerToClient);
        app.register_message::<marvyr_protocol::WorldEvent>(ChannelDirection::ServerToClient);
        app.register_message::<marvyr_protocol::SetRepair>(ChannelDirection::ClientToServer);
        app.register_message::<marvyr_protocol::BoardShip>(ChannelDirection::ClientToServer);
        app.register_message::<marvyr_protocol::HireCrew>(ChannelDirection::ClientToServer);
        app.register_message::<marvyr_protocol::DigTreasure>(ChannelDirection::ClientToServer);
        app.register_message::<marvyr_protocol::ActionResult>(ChannelDirection::ServerToClient);
        app.register_message::<marvyr_protocol::SeaEventsUpdate>(ChannelDirection::ServerToClient);
        app.register_message::<marvyr_protocol::TreasureHints>(ChannelDirection::ServerToClient);
        app.register_message::<marvyr_protocol::IslandsInSight>(ChannelDirection::ServerToClient);
        // v16 (MV-062): SEMPRE no fim — espelho exato do servidor.
        app.register_message::<marvyr_protocol::OnboardingProgress>(
            ChannelDirection::ClientToServer,
        );
        // v17 (MV-065): SEMPRE no fim, espelho do servidor.
        app.register_message::<marvyr_protocol::WorldSeed>(ChannelDirection::ServerToClient);
        // v18 (MV-066): SEMPRE no fim, espelho do servidor.
        app.register_message::<marvyr_protocol::CosmeticsSnapshot>(
            ChannelDirection::ServerToClient,
        );
        app.register_message::<marvyr_protocol::WearCosmetic>(ChannelDirection::ClientToServer);
        // v19 (MV-067): SEMPRE no fim, espelho do servidor.
        app.register_message::<marvyr_protocol::RenownUpdate>(ChannelDirection::ServerToClient);
        app.register_message::<marvyr_protocol::TalentsSnapshot>(ChannelDirection::ServerToClient);
        app.register_message::<marvyr_protocol::AllocateTalent>(ChannelDirection::ClientToServer);
        app.register_message::<marvyr_protocol::RespecTalents>(ChannelDirection::ClientToServer);
        // v21: tiro automático — Bandeira Negra e alvo travado.
        app.register_message::<marvyr_protocol::SetBlackFlag>(ChannelDirection::ClientToServer);
        app.register_message::<marvyr_protocol::LockTarget>(ChannelDirection::ClientToServer);
        // v24: gemas de suporte.
        app.register_message::<marvyr_protocol::SocketGem>(ChannelDirection::ClientToServer);
        app.register_message::<marvyr_protocol::UnsocketGem>(ChannelDirection::ClientToServer);
        // v25: frascos de bordo.
        app.register_message::<marvyr_protocol::UseFlask>(ChannelDirection::ClientToServer);
        // v28: mover um item entre porão e armazém.
        app.register_message::<marvyr_protocol::StorageDeposit>(ChannelDirection::ClientToServer);
        app.register_message::<marvyr_protocol::StorageWithdraw>(ChannelDirection::ClientToServer);
        // v29: orbes de ofício.
        app.register_message::<marvyr_protocol::ApplyOrb>(ChannelDirection::ClientToServer);
        app.register_message::<marvyr_protocol::OrbResult>(ChannelDirection::ServerToClient);
        // v35: Diário de Bordo.
        app.register_message::<marvyr_protocol::ProgressSnapshot>(ChannelDirection::ServerToClient);
        // v39: pesca.
        app.register_message::<marvyr_protocol::CastLine>(ChannelDirection::ClientToServer);
        // v43: temporadas.
        app.register_message::<marvyr_protocol::SeasonBoard>(ChannelDirection::ServerToClient);
        // v44: caçadas.
        app.register_message::<marvyr_protocol::BountyBoard>(ChannelDirection::ServerToClient);
        app.add_event::<PlayerNotice>();
        app.init_resource::<crate::ship::DestroyedShips>();
        app.init_resource::<KnownWrecks>();
        app.init_resource::<MyDocked>();
        app.init_resource::<MyCosmetics>();
        app.init_resource::<SailLevel>();
        // Intenção contínua (leme/pano) vai no tick fixo; comandos de tecla
        // única ficam no Update — `just_pressed` vale um frame de render e o
        // FixedUpdate a 30 Hz pula frames, engolindo tiros e atracações.
        app.add_systems(FixedUpdate, (send_hello_on_connect, send_ship_input));
        app.add_systems(
            Update,
            (
                update_sail_level,
                send_dock_input,
                send_loadout_input,
                send_gunnery_input,
                send_loot_input,
                send_gather_input,
            ),
        );
        app.add_systems(
            Update,
            (
                handle_handshake,
                receive_world_seed,
                receive_cosmetics,
                handle_dock_result,
                handle_loadout_result,
                handle_ship_destroyed,
                reset_on_disconnect,
                handle_loot_result,
            ),
        );
    }
}

/// Wrecks conhecidos pelo client (posições para saque e visuais). A fonte
/// agora é o snapshot AOI (MF-031) — `ship.rs` reconstrói a cada quadro.
#[derive(Resource, Debug, Default)]
pub struct KnownWrecks(pub HashMap<u32, Vec2>);

/// Presença do PRÓPRIO navio conforme o servidor (MF-036): o veredito
/// `DockResult.docked` é a verdade; o client não infere por posição.
#[derive(Resource, Debug, Default)]
pub struct MyDocked(pub bool);

/// Raio de saque usado pelo HUD e pelo atalho F (38 m: 40 do servidor com
/// folga para o lerp visual).
pub const LOOT_RADIUS_SQ: f32 = 38.0 * 38.0;
/// Raio de coleta usado pelo HUD e pelo atalho G (45 do servidor, com folga).
pub const GATHER_RADIUS_SQ: f32 = 43.0 * 43.0;

/// E alterna atracar/desatracar (MF-036). Dev tooling (§39):
/// MARVYR_AUTODOCK=1 tenta atracar sozinho até conseguir — smoke da
/// rotina de porto sem digitar.
fn send_dock_input(
    keys: Res<ButtonInput<KeyCode>>,
    my_docked: Res<MyDocked>,
    time: Res<Time>,
    mut autodock_timer: Local<f32>,
    mut docked_for: Local<f32>,
    mut undocked_once: Local<bool>,
    mut connection_manager: ResMut<ConnectionManager>,
) {
    // Dev (§39): MARVYR_AUTOUNDOCK=<s> sai do porto uma vez depois de <s>
    // atracado (equipar/fabricar e ver o navio no mar sem teclado).
    let autoundock: Option<f32> = std::env::var("MARVYR_AUTOUNDOCK")
        .ok()
        .and_then(|secs| secs.parse().ok());
    if let (Some(after), true, false) = (autoundock, my_docked.0, *undocked_once) {
        *docked_for += time.delta_secs();
        if *docked_for >= after {
            *undocked_once = true;
            let _ = connection_manager.send_message::<ReliableChannel, _>(&Undock);
            return;
        }
    }
    if keys.just_pressed(KeyCode::KeyE) {
        if my_docked.0 {
            info!("desatracando");
            let _ = connection_manager.send_message::<ReliableChannel, _>(&Undock);
        } else {
            info!("atracando");
            let _ = connection_manager.send_message::<ReliableChannel, _>(&Dock);
        }
        return;
    }
    if std::env::var_os("MARVYR_AUTODOCK").is_some() && !my_docked.0 && !*undocked_once {
        *autodock_timer += time.delta_secs();
        if *autodock_timer >= 1.5 {
            *autodock_timer = 0.0;
            let _ = connection_manager.send_message::<ReliableChannel, _>(&Dock);
        }
    }
}

/// T/Y/U equipam Casco/Velas/Canhão do storage; Shift+T/Y/U desequipam os
/// slots Hull/Sail/Weapon (MF-039, dev keys — a tela de porto do P3 substitui
/// isto). MARVYR_AUTOEQUIP=1 instala os três em sequência para o smoke.
fn send_loadout_input(
    keys: Res<ButtonInput<KeyCode>>,
    known_catalog: Res<crate::market::KnownCatalog>,
    time: Res<Time>,
    mut auto_timer: Local<f32>,
    mut auto_step: Local<usize>,
    mut connection_manager: ResMut<ConnectionManager>,
) {
    const DEV_EQUIPMENT: [(&str, EquipmentSlot); 3] = [
        ("Casco Reforçado", EquipmentSlot::Hull),
        ("Velas de Corrida", EquipmentSlot::Sail),
        ("Canhão de Bronze", EquipmentSlot::Weapon),
    ];

    let shift = keys.pressed(KeyCode::ShiftLeft) || keys.pressed(KeyCode::ShiftRight);
    for (index, key) in [KeyCode::KeyT, KeyCode::KeyY, KeyCode::KeyU]
        .into_iter()
        .enumerate()
    {
        if !keys.just_pressed(key) {
            continue;
        }
        let (name, slot) = DEV_EQUIPMENT[index];
        if shift {
            info!(?slot, "desequipando slot");
            let _ = connection_manager.send_message::<ReliableChannel, _>(&UnequipItem { slot });
        } else if let Some(line) = known_catalog.0.get(name) {
            info!(item = name, "equipando do storage");
            let _ = connection_manager.send_message::<ReliableChannel, _>(&EquipItem {
                item: line.id,
                instance: None,
            });
        } else {
            warn!(item = name, "item fora do catálogo conhecido");
        }
    }

    if std::env::var_os("MARVYR_AUTOEQUIP").is_some() {
        *auto_timer += time.delta_secs();
        if *auto_timer >= 1.0 {
            *auto_timer = 0.0;
            if *auto_step < DEV_EQUIPMENT.len() {
                let (name, _) = DEV_EQUIPMENT[*auto_step];
                if let Some(line) = known_catalog.0.get(name) {
                    let _ = connection_manager.send_message::<ReliableChannel, _>(&EquipItem {
                        item: line.id,
                        instance: None,
                    });
                }
            }
            *auto_step = (*auto_step + 1) % (DEV_EQUIPMENT.len() + 1);
        }
    }
}

/// O servidor decide a presença; o client espelha o estado e mostra o motivo.
fn handle_dock_result(
    mut events: EventReader<ClientReceiveMessage<DockResult>>,
    mut my_docked: ResMut<MyDocked>,
) {
    for event in events.read() {
        let result = event.message();
        if result.success {
            info!(docked = result.docked, reason = %result.reason, "doca: {}", result.reason);
        } else {
            warn!(reason = %result.reason, "doca recusou: {}", result.reason);
        }
        my_docked.0 = result.docked;
    }
}

/// Token anônimo persistente (MF-035) para dev SEM serviço de contas: a
/// MESMA identidade sobrevive a restart de client. Ordem: `MARVYR_IDENTITY`
/// (testes/smoke) → `<dados do jogador>/identity` → gera e salva. Produção
/// usa o JWT do login; o servidor recusa token anônimo.
pub(crate) fn identity_token() -> String {
    if let Ok(token) = std::env::var("MARVYR_IDENTITY") {
        if !token.trim().is_empty() {
            return token;
        }
    }
    let path = crate::config::data_dir().join("identity");
    if let Ok(token) = std::fs::read_to_string(&path) {
        let token = token.trim().to_string();
        if !token.is_empty() {
            return token;
        }
    }
    let token = uuid::Uuid::new_v4().to_string();
    if let Some(parent) = path.parent() {
        let _ = std::fs::create_dir_all(parent);
    }
    if std::fs::write(&path, &token).is_ok() {
        info!(token = %token, "nova identidade de jogador criada");
    }
    token
}

/// Vereditos de loadout: só log (a verdade dos stats vem no ShipState).
fn handle_loadout_result(mut events: EventReader<ClientReceiveMessage<LoadoutResult>>) {
    for event in events.read() {
        let result = event.message();
        if result.success {
            info!(reason = %result.reason, "loadout: {}", result.reason);
        } else {
            warn!(reason = %result.reason, "loadout recusado: {}", result.reason);
        }
    }
}

/// Sessão caiu: o client não tem mais navio atribuído (o servidor mantém o
/// personagem vivo na janela de graça; o reconnect é um novo processo com
/// o mesmo token — `identity_token`).
fn reset_on_disconnect(
    mut disconnect: EventReader<DisconnectEvent>,
    mut my_ship: ResMut<MyShip>,
    mut ship_kind: ResMut<KnownShipKind>,
    mut status: ResMut<crate::session::ConnectionStatus>,
) {
    for _ in disconnect.read() {
        warn!("conexão perdida; personagem segue no servidor dentro da janela de graça");
        my_ship.0 = None;
        ship_kind.0 = None;
        status.on_disconnect();
    }
}

fn connect_now(mut commands: Commands, mut status: ResMut<crate::session::ConnectionStatus>) {
    commands.connect_client();
    *status = crate::session::ConnectionStatus::Connecting { since: 0.0 };
}

/// Handshake (ADR-0011): primeira mensagem após conectar é o hello com a
/// versão do protocolo e o token de identidade do jogador (MF-035).
fn send_hello_on_connect(
    mut connect: EventReader<ConnectEvent>,
    mut connection_manager: ResMut<ConnectionManager>,
    identity: Res<ClientIdentity>,
) {
    for _ in connect.read() {
        let Some(token) = identity.0.clone() else {
            warn!("conectado sem identidade de sessão; hello adiado");
            continue;
        };
        info!("conectado; enviando ClientHello com identidade");
        let _ = connection_manager.send_message::<ReliableChannel, _>(&ClientHello::current(token));
    }
}

/// Pano armado (MF-058): W iça um nível, S recolhe um. O navio mantém o
/// seguimento sem o jogador segurar tecla — navegar é escolher o pano e
/// governar o leme, não apertar W por dois minutos.
#[derive(Resource, Debug, Default, Clone, Copy, PartialEq, Eq)]
pub struct SailLevel(pub u8);

impl SailLevel {
    pub const MAX: u8 = 3;

    pub fn throttle(self) -> f32 {
        f32::from(self.0) / f32::from(Self::MAX)
    }

    pub fn label(self) -> &'static str {
        match self.0 {
            0 => "Velas recolhidas",
            1 => "Pano de manobra",
            2 => "Meio pano",
            _ => "Pano cheio",
        }
    }
}

/// W/S mudam o nível de pano; atracado, o pano é recolhido.
fn update_sail_level(
    keys: Res<ButtonInput<KeyCode>>,
    docked: Res<MyDocked>,
    mut sail: ResMut<SailLevel>,
) {
    if docked.0 {
        sail.0 = 0;
        return;
    }
    if keys.just_pressed(KeyCode::KeyW) || keys.just_pressed(KeyCode::ArrowUp) {
        sail.0 = (sail.0 + 1).min(SailLevel::MAX);
    }
    if keys.just_pressed(KeyCode::KeyS) || keys.just_pressed(KeyCode::ArrowDown) {
        sail.0 = sail.0.saturating_sub(1);
    }
}

/// Intenção de navegação local — o servidor valida e aplica (Pilar 4).
fn send_ship_input(
    keys: Res<ButtonInput<KeyCode>>,
    sail: Res<SailLevel>,
    override_input: Res<ShipInputOverride>,
    mut connection_manager: ResMut<ConnectionManager>,
) {
    let input = match override_input.0 {
        Some(input) => input,
        None => {
            // Dev tooling (PRD §39): MARVYR_AUTOSAIL=1 navega a pano cheio.
            let autosail = std::env::var_os("MARVYR_AUTOSAIL").is_some();
            let throttle = if autosail { 1.0 } else { sail.throttle() };
            let left = keys.pressed(KeyCode::KeyA) || keys.pressed(KeyCode::ArrowLeft);
            let right = keys.pressed(KeyCode::KeyD) || keys.pressed(KeyCode::ArrowRight);
            let turn = (left as i32 - right as i32) as f32;
            ShipInput { throttle, turn }
        }
    };
    let _ = connection_manager.send_message::<UnreliableChannel, _>(&input);
}

/// v21: o servidor dispara sozinho. Q trava/solta o alvo (inclusive
/// inocente); R iça ou arria a Bandeira Negra conforme o estado atual.
fn send_gunnery_input(
    keys: Res<ButtonInput<KeyCode>>,
    my_ship: Res<MyShip>,
    visuals: Query<&crate::ship::ShipVisual>,
    mut connection_manager: ResMut<ConnectionManager>,
) {
    if keys.just_pressed(KeyCode::KeyQ) {
        let _ = connection_manager.send_message::<ReliableChannel, _>(&LockTarget);
    }
    if keys.just_pressed(KeyCode::KeyR) {
        let lowered = visuals
            .iter()
            .find(|visual| Some(visual.target.ship_id) == my_ship.0)
            .map_or(true, |visual| {
                visual.target.black_flag == marvyr_protocol::FLAG_LOWERED
            });
        let _ =
            connection_manager.send_message::<ReliableChannel, _>(&SetBlackFlag { raise: lowered });
    }
}

fn autofire_enabled() -> bool {
    std::env::var_os("MARVYR_AUTOFIRE").is_some()
}

/// Navio afundou: remove o visual e memoriza o id (snapshots antigos ainda
/// podem trazer o casco; ids não são reutilizados — o respawn tem id novo).
fn handle_ship_destroyed(
    mut events: EventReader<ClientReceiveMessage<ShipDestroyed>>,
    mut destroyed: ResMut<crate::ship::DestroyedShips>,
    mut commands: Commands,
    visuals: Query<(Entity, &crate::ship::ShipVisual)>,
) {
    for event in events.read() {
        let ship_id = event.message().ship_id;
        warn!(ship_id, "navio destruído no horizonte");
        destroyed.0.insert(ship_id);
        // O casco não some: afunda (animação em `ship::animate_sinking`).
        for (entity, visual) in &visuals {
            if visual.target.ship_id == ship_id {
                // try_insert: um soluço > TTL pode ter expirado o visual.
                commands
                    .entity(entity)
                    .remove::<crate::ship::ShipVisual>()
                    .try_insert(crate::ship::Sinking::of(ship_id));
            }
        }
    }
}

/// Aviso curto para o feed do HUD (MV-062): motivo de recusa vindo do
/// servidor (saque, coleta, fabricação).
#[derive(Event, Debug, Clone, PartialEq, Eq)]
pub struct PlayerNotice(pub String);

impl PlayerNotice {
    /// Só recusas com motivo viram aviso; sucesso e motivo vazio, não.
    pub fn from_refusal(success: bool, reason: &str) -> Option<Self> {
        (!success && !reason.is_empty()).then(|| Self(reason.to_owned()))
    }
}

/// Relata o progresso do onboarding ao servidor (telemetria MV-062).
pub fn send_onboarding(connection_manager: &mut ConnectionManager, step: u8, skipped: bool) {
    let _ = connection_manager
        .send_message::<ReliableChannel, _>(&marvyr_protocol::OnboardingProgress { step, skipped });
}

fn handle_loot_result(
    mut events: EventReader<ClientReceiveMessage<LootResult>>,
    mut notices: EventWriter<PlayerNotice>,
) {
    for event in events.read() {
        let result = event.message();
        if result.success {
            info!(
                wreck_id = result.wreck_id,
                "saque concluído: carga no porão"
            );
        } else {
            warn!(wreck_id = result.wreck_id, reason = %result.reason, "saque recusado pelo servidor");
        }
        notices.send_batch(PlayerNotice::from_refusal(result.success, &result.reason));
    }
}

/// F saqueia o wreck mais próximo (PRD §27). No modo dev (PRD §39), o
/// autofire também saqueia sozinho para o smoke do loop econômico.
fn send_loot_input(
    keys: Res<ButtonInput<KeyCode>>,
    time: Res<Time>,
    my_ship: Res<MyShip>,
    known_wrecks: Res<KnownWrecks>,
    visuals: Query<&crate::ship::ShipVisual>,
    mut autofire_timer: Local<f32>,
    mut connection_manager: ResMut<ConnectionManager>,
) {
    let manual = keys.just_pressed(KeyCode::KeyF);
    let mut auto = false;
    if autofire_enabled() {
        *autofire_timer += time.delta_secs();
        if *autofire_timer >= 2.0 {
            *autofire_timer = 0.0;
            auto = true;
        }
    }
    if !manual && !auto {
        return;
    }

    let Some(my_id) = my_ship.0 else { return };
    let Some(my_visual) = visuals.iter().find(|v| v.target.ship_id == my_id) else {
        return;
    };
    let mine = Vec2::new(my_visual.target.x, my_visual.target.y);

    let nearest = known_wrecks
        .0
        .iter()
        .filter(|(_, pos)| mine.distance_squared(**pos) <= LOOT_RADIUS_SQ)
        .min_by(|a, b| {
            let da = mine.distance_squared(*a.1);
            let db = mine.distance_squared(*b.1);
            da.total_cmp(&db)
        })
        .map(|(id, _)| *id);

    if let Some(wreck_id) = nearest {
        if manual {
            info!(wreck_id, "saqueando wreck");
        }
        let _ = connection_manager.send_message::<ReliableChannel, _>(&LootWreck { wreck_id });
    }
}

/// G coleta o node mais próximo com estoque (PRD MF-019). Dev tooling
/// (§39): MARVYR_AUTOGATHER=1 coleta sozinho — smoke do loop de recursos.
fn send_gather_input(
    keys: Res<ButtonInput<KeyCode>>,
    time: Res<Time>,
    my_ship: Res<MyShip>,
    known_nodes: Res<crate::nodes::KnownNodes>,
    visuals: Query<&crate::ship::ShipVisual>,
    mut auto_timer: Local<f32>,
    mut connection_manager: ResMut<ConnectionManager>,
) {
    let manual = keys.just_pressed(KeyCode::KeyG);
    let mut auto = false;
    if autogather_enabled() {
        *auto_timer += time.delta_secs();
        if *auto_timer >= 1.0 {
            *auto_timer = 0.0;
            auto = true;
        }
    }
    if !manual && !auto {
        return;
    }

    let Some(my_id) = my_ship.0 else { return };
    let Some(my_visual) = visuals.iter().find(|v| v.target.ship_id == my_id) else {
        return;
    };
    let mine = Vec2::new(my_visual.target.x, my_visual.target.y);

    let nearest = known_nodes
        .0
        .iter()
        .filter(|(_, info)| info.stock > 0 && mine.distance_squared(info.pos) <= GATHER_RADIUS_SQ)
        .min_by(|a, b| {
            let da = mine.distance_squared(a.1.pos);
            let db = mine.distance_squared(b.1.pos);
            da.total_cmp(&db)
        })
        .map(|(id, _)| *id);

    if let Some(node_id) = nearest {
        if manual {
            info!(node_id, "coletando node");
        }
        let _ = connection_manager.send_message::<ReliableChannel, _>(&GatherNode { node_id });
    }
}

fn autogather_enabled() -> bool {
    std::env::var_os("MARVYR_AUTOGATHER").is_some()
}

/// Cosméticos do capitão (MV-066): o que possui e o que está usando.
#[derive(Resource, Debug, Default)]
pub struct MyCosmetics(pub Option<marvyr_protocol::CosmeticsSnapshot>);

fn receive_cosmetics(
    mut events: EventReader<ClientReceiveMessage<marvyr_protocol::CosmeticsSnapshot>>,
    mut mine: ResMut<MyCosmetics>,
) {
    if let Some(event) = events.read().last() {
        mine.0 = Some(event.message().clone());
    }
}

/// MV-065: a seed do servidor vira o mapa do client (mesmo gerador).
fn receive_world_seed(
    mut commands: Commands,
    mut seeds: EventReader<ClientReceiveMessage<marvyr_protocol::WorldSeed>>,
    world: Option<Res<crate::world::ClientWorld>>,
) {
    let Some(seed) = seeds.read().last().map(|event| event.message().seed) else {
        return;
    };
    if world.is_some_and(|world| world.0.features().seed == seed) {
        return;
    }
    info!(seed, "mundo do servidor recebido");
    commands.insert_resource(crate::world::ClientWorld(
        marvyr_domain_world::WorldMap::from_seed(seed),
    ));
}

fn handle_handshake(
    mut welcome_events: EventReader<ClientReceiveMessage<ServerWelcome>>,
    mut assign_events: EventReader<ClientReceiveMessage<AssignShip>>,
    mut my_ship: ResMut<MyShip>,
    mut ship_kind: ResMut<KnownShipKind>,
    mut status: ResMut<crate::session::ConnectionStatus>,
) {
    for event in welcome_events.read() {
        let welcome = event.message();
        if welcome.accepted {
            info!(
                server_protocol = welcome.protocol_version,
                "handshake aceito pelo servidor"
            );
            *status = crate::session::ConnectionStatus::InGame;
        } else {
            error!(
                server_protocol = welcome.protocol_version,
                our_protocol = PROTOCOL_VERSION,
                reason = %welcome.reason,
                "servidor recusou a conexão"
            );
            *status = crate::session::ConnectionStatus::from_rejection(welcome);
        }
    }
    for event in assign_events.read() {
        let message = event.message();
        my_ship.0 = Some(message.ship_id);
        ship_kind.0 = Some(message.kind);
        info!(
            ship_id = message.ship_id,
            kind = ?message.kind,
            "navio atribuído a este client"
        );
    }
}
