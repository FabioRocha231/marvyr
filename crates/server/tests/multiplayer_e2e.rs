//! MF-047: network-layer E2E for two simulated clients against one in-process
//! server. Unlike `e2e.rs`, protocol messages travel through the lightyear
//! wire format over local channels.

use std::time::Duration;

use bevy::input::InputPlugin;
use bevy::prelude::{App, EventReader, MinimalPlugins, Plugin, ResMut, Resource, Update};
use bevy::state::app::StatesPlugin;
use bevy::time::{Fixed, Real, Time, TimeUpdateStrategy};
use bevy::utils::Instant;
use crossbeam_channel::{Receiver, Sender};
use lightyear::prelude::client::{
    Authentication, ClientTransport, ConnectionManager as ClientConnectionManager,
    IoConfig as ClientIoConfig, NetConfig, NetcodeConfig as ClientNetcodeConfig,
};
use lightyear::prelude::server::ServerTransport;
use lightyear::prelude::ClientReceiveMessage;
use lightyear::transport::LOCAL_SOCKET;
use marvyr_client::net::{
    ClientIdentity, ClientNetOverride, ClientNetPlugin, ReliableChannel, ShipInputOverride,
};
use marvyr_domain_combat::BroadsideBattery;
use marvyr_protocol::{
    AssignShip, GatherNode, GatherResult, LoadoutSnapshot, LockTarget, OnboardingProgress,
    ServerWelcome, ShipDestroyed, ShipInput, ShipState, WorldSnapshot,
};
use marvyr_server::net::{ServerNetPlugin, ServerShip, ServerTransportOverride};
use marvyr_server::plugin::ServerPlugin;

const CLIENT_A_ID: u64 = 101;
const CLIENT_B_ID: u64 = 102;

fn tick() -> Duration {
    Duration::from_secs_f64(1.0 / 30.0)
}

#[derive(Resource, Default)]
struct Recorded {
    welcome: Option<ServerWelcome>,
    assign: Option<AssignShip>,
    snapshots: Vec<WorldSnapshot>,
    destroyed: Vec<ShipDestroyed>,
    gathers: Vec<GatherResult>,
    loadouts: Vec<LoadoutSnapshot>,
}

impl Recorded {
    fn latest_snapshot(&self) -> Option<&WorldSnapshot> {
        self.snapshots.last()
    }

    fn contains_ship(&self, ship_id: u32) -> bool {
        self.latest_snapshot()
            .is_some_and(|snapshot| snapshot.ships.iter().any(|ship| ship.ship_id == ship_id))
    }

    fn latest_ship(&self, ship_id: u32) -> Option<ShipState> {
        self.snapshots.iter().rev().find_map(|snapshot| {
            snapshot
                .ships
                .iter()
                .copied()
                .find(|ship| ship.ship_id == ship_id)
        })
    }
}

struct RecordPlugin;

impl Plugin for RecordPlugin {
    fn build(&self, app: &mut App) {
        app.init_resource::<Recorded>();
        app.add_systems(
            Update,
            (
                record_welcome,
                record_assign,
                record_snapshot,
                record_destroyed,
                record_gather,
                record_loadout,
            ),
        );
    }
}

fn record_welcome(
    mut events: EventReader<ClientReceiveMessage<ServerWelcome>>,
    mut recorded: ResMut<Recorded>,
) {
    for event in events.read() {
        recorded.welcome = Some(event.message().clone());
    }
}

fn record_assign(
    mut events: EventReader<ClientReceiveMessage<AssignShip>>,
    mut recorded: ResMut<Recorded>,
) {
    for event in events.read() {
        recorded.assign = Some(*event.message());
    }
}

fn record_snapshot(
    mut events: EventReader<ClientReceiveMessage<WorldSnapshot>>,
    mut recorded: ResMut<Recorded>,
) {
    for event in events.read() {
        recorded.snapshots.push(event.message().clone());
    }
}

fn record_destroyed(
    mut events: EventReader<ClientReceiveMessage<ShipDestroyed>>,
    mut recorded: ResMut<Recorded>,
) {
    for event in events.read() {
        recorded.destroyed.push(*event.message());
    }
}

fn record_gather(
    mut events: EventReader<ClientReceiveMessage<GatherResult>>,
    mut recorded: ResMut<Recorded>,
) {
    for event in events.read() {
        recorded.gathers.push(event.message().clone());
    }
}

fn record_loadout(
    mut events: EventReader<ClientReceiveMessage<LoadoutSnapshot>>,
    mut recorded: ResMut<Recorded>,
) {
    for event in events.read() {
        recorded.loadouts.push(event.message().clone());
    }
}

struct Harness {
    server_app: App,
    client_a: App,
    client_b: App,
    now: Instant,
}

impl Harness {
    fn new() -> Self {
        Self::with_identities("multiplayer-token-a", "multiplayer-token-b")
    }

    fn with_identities(identity_a: &str, identity_b: &str) -> Self {
        let (a_to_server, a_from_client) = crossbeam_channel::unbounded();
        let (a_to_client, a_from_server) = crossbeam_channel::unbounded();
        let (b_to_server, b_from_client) = crossbeam_channel::unbounded();
        let (b_to_client, b_from_server) = crossbeam_channel::unbounded();

        let client_a_config = client_net_config(CLIENT_A_ID, a_from_server, a_to_server);
        let client_b_config = client_net_config(CLIENT_B_ID, b_from_server, b_to_server);

        let mut server_app = App::new();
        server_app.add_plugins(MinimalPlugins);
        server_app.insert_resource(ServerTransportOverride(vec![
            ServerTransport::Channels {
                channels: vec![(LOCAL_SOCKET, a_from_client, a_to_client)],
            },
            ServerTransport::Channels {
                channels: vec![(LOCAL_SOCKET, b_from_client, b_to_client)],
            },
        ]));
        server_app.add_plugins(ServerPlugin);
        // Posições do teste são do mapa clássico (seed 0), não do gerado.
        server_app.insert_resource(marvyr_server::net::ServerWorldMap(
            marvyr_domain_world::WorldMap::vertical_slice().with_hidden_islands(),
        ));
        // MF-060: piratas e marinha nascem na Rota da Costa, bem onde o duelo
        // acontece — entram na briga e tornam o teste aleatório. Sem NPCs:
        // o teste mede AOI e dano entre jogadores.
        server_app.insert_resource(marvyr_server::npc::NpcSpawnConfig {
            count: 0,
            raider_positions: Vec::new(),
            navy_positions: Vec::new(),
            caravan_count: 0,
            ..Default::default()
        });
        // Portais com semente fixa: sorteio pelo relógio punha um redemoinho
        // no caminho de A de vez em quando.
        server_app.insert_resource(marvyr_server::portals::ServerPortals(
            marvyr_domain_world::PortalDirector::new(7, Default::default()),
        ));
        server_app.add_plugins(ServerNetPlugin);

        let mut client_a = build_client(client_a_config, identity_a);
        let mut client_b = build_client(client_b_config, identity_b);

        server_app.finish();
        server_app.cleanup();
        client_a.finish();
        client_a.cleanup();
        client_b.finish();
        client_b.cleanup();

        let now = Instant::now();
        for app in [&mut server_app, &mut client_a, &mut client_b] {
            app.insert_resource(TimeUpdateStrategy::ManualInstant(now));
            app.world_mut()
                .get_resource_mut::<Time<Real>>()
                .unwrap()
                .update_with_instant(now);
        }

        Self {
            server_app,
            client_a,
            client_b,
            now,
        }
    }

    fn frame_step(&mut self) {
        self.now += tick();
        for app in [&mut self.client_a, &mut self.client_b, &mut self.server_app] {
            app.insert_resource(TimeUpdateStrategy::ManualInstant(self.now));
            app.update();
        }
    }

    fn run_frames(&mut self, frames: usize) {
        for _ in 0..frames {
            self.frame_step();
        }
    }

    fn run_until(&mut self, max_frames: usize, ready: impl Fn(&Harness) -> bool) -> bool {
        for _ in 0..max_frames {
            if ready(self) {
                return true;
            }
            self.frame_step();
        }
        ready(self)
    }

    fn recorded_a(&self) -> &Recorded {
        self.client_a.world().resource::<Recorded>()
    }

    fn recorded_b(&self) -> &Recorded {
        self.client_b.world().resource::<Recorded>()
    }

    fn wait_for_handshake(&mut self) {
        let ready = self.run_until(300, |harness| {
            harness.recorded_a().assign.is_some() && harness.recorded_b().assign.is_some()
        });
        assert!(ready, "both clients should receive AssignShip");
        assert_eq!(self.recorded_a().welcome, Some(ServerWelcome::accepted()));
        assert!(!self.recorded_a().loadouts.is_empty());
    }

    fn ship_ids(&self) -> (u32, u32) {
        (
            self.recorded_a().assign.unwrap().ship_id,
            self.recorded_b().assign.unwrap().ship_id,
        )
    }

    fn set_input_a(&mut self, input: ShipInput) {
        self.client_a
            .world_mut()
            .insert_resource(ShipInputOverride(Some(input)));
    }

    fn stop_input_a(&mut self) {
        self.set_input_a(ShipInput {
            throttle: 0.0,
            turn: 0.0,
        });
    }

    fn send_a<M: lightyear::prelude::Message>(&mut self, message: &M) {
        self.client_a
            .world_mut()
            .resource_mut::<ClientConnectionManager>()
            .send_message::<ReliableChannel, _>(message)
            .expect("client can queue message");
    }

    /// v21: o tiro é automático; contra um inocente, A trava o alvo (Q).
    fn lock_a(&mut self) {
        self.send_a(&LockTarget);
    }

    fn prepare_ships(&mut self, a_id: u32, b_id: u32) {
        set_ship_position(&mut self.server_app, a_id, 300.0, 0.0, 0.0);
        set_ship_position(&mut self.server_app, b_id, 250.0, 0.0, 0.0);
        // Espera a condição em vez de um número fixo de frames: em CI lento
        // 30 frames às vezes não bastavam para o snapshot chegar.
        let seen = self.run_until(300, |harness| harness.recorded_b().contains_ship(a_id));
        assert!(seen, "B should see A inside the AOI");
    }

    fn move_a_out_of_aoi(&mut self, a_id: u32) {
        self.set_input_a(ShipInput {
            throttle: 1.0,
            turn: 0.0,
        });
        let left = self.run_until(400, |harness| !harness.recorded_b().contains_ship(a_id));
        assert!(left, "A should leave B's AOI");
        self.run_frames(20);
        assert!(
            !self.recorded_b().contains_ship(a_id),
            "A should stay absent from B's AOI"
        );
        self.stop_input_a();
    }

    fn return_a_to_aoi(&mut self, a_id: u32) {
        set_ship_heading(&mut self.server_app, a_id, std::f32::consts::PI);
        self.set_input_a(ShipInput {
            throttle: 1.0,
            turn: 0.0,
        });
        let returned = self.run_until(400, |harness| harness.recorded_b().contains_ship(a_id));
        assert!(returned, "A should reappear in B's AOI");
        self.stop_input_a();
    }
}

fn client_net_config(
    client_id: u64,
    from_server: Receiver<Vec<u8>>,
    to_server: Sender<Vec<u8>>,
) -> NetConfig {
    NetConfig::Netcode {
        auth: Authentication::Manual {
            server_addr: LOCAL_SOCKET,
            client_id,
            private_key: marvyr_protocol::NETCODE_KEY,
            protocol_id: marvyr_protocol::NETCODE_PROTOCOL_ID,
        },
        config: ClientNetcodeConfig::default(),
        io: ClientIoConfig::from_transport(ClientTransport::LocalChannel {
            recv: from_server,
            send: to_server,
        }),
    }
}

fn build_client(net_config: NetConfig, identity: &str) -> App {
    let mut app = App::new();
    app.add_plugins((MinimalPlugins, InputPlugin, StatesPlugin));
    app.insert_resource(Time::<Fixed>::from_hz(30.0));
    app.insert_resource(ClientNetOverride(Some(net_config)));
    app.insert_resource(ClientIdentity(Some(identity.to_owned())));
    app.add_plugins(ClientNetPlugin);
    app.add_plugins(RecordPlugin);
    app
}

fn set_ship_position(app: &mut App, ship_id: u32, x: f32, y: f32, heading: f32) {
    let world = app.world_mut();
    let mut query = world.query::<&mut ServerShip>();
    for mut ship in query.iter_mut(world) {
        if ship.ship_id == ship_id {
            ship.motion.x = x;
            ship.motion.y = y;
            ship.motion.heading = heading;
            ship.motion.speed = 0.0;
        }
    }
}

fn set_ship_heading(app: &mut App, ship_id: u32, heading: f32) {
    let world = app.world_mut();
    let mut query = world.query::<&mut ServerShip>();
    for mut ship in query.iter_mut(world) {
        if ship.ship_id == ship_id {
            ship.motion.heading = heading;
        }
    }
}

fn set_ship_hp(app: &mut App, ship_id: u32, hp: u32) {
    let world = app.world_mut();
    let mut query = world.query::<&mut ServerShip>();
    for mut ship in query.iter_mut(world) {
        if ship.ship_id == ship_id {
            ship.hp = hp;
        }
    }
}

fn reset_ship_battery(app: &mut App, ship_id: u32) {
    let world = app.world_mut();
    let mut query = world.query::<&mut ServerShip>();
    for mut ship in query.iter_mut(world) {
        if ship.ship_id == ship_id {
            ship.battery = BroadsideBattery::default();
        }
    }
}

#[test]
fn multiplayer_two_clients_receive_each_other_in_aoi() {
    let mut harness = Harness::new();
    harness.wait_for_handshake();
    let (a_id, b_id) = harness.ship_ids();
    harness.run_frames(30);

    assert!(
        harness.recorded_b().contains_ship(a_id),
        "B should receive A's ShipState in WorldSnapshot"
    );
    assert!(
        harness.recorded_a().contains_ship(b_id),
        "A should receive B's ShipState in WorldSnapshot"
    );
}

#[test]
fn multiplayer_client_outside_aoi_not_in_snapshot() {
    let mut harness = Harness::new();
    harness.wait_for_handshake();
    let (a_id, b_id) = harness.ship_ids();
    harness.prepare_ships(a_id, b_id);
    harness.move_a_out_of_aoi(a_id);
}

#[test]
fn multiplayer_client_returns_to_aoi_reappears() {
    let mut harness = Harness::new();
    harness.wait_for_handshake();
    let (a_id, b_id) = harness.ship_ids();
    harness.prepare_ships(a_id, b_id);
    harness.move_a_out_of_aoi(a_id);
    harness.return_a_to_aoi(a_id);

    assert!(
        harness.recorded_b().contains_ship(a_id),
        "A should be visible again after sailing back"
    );
}

#[test]
fn multiplayer_broadside_damages_other_client_ship() {
    let mut harness = Harness::new();
    harness.wait_for_handshake();
    let (a_id, b_id) = harness.ship_ids();
    harness.prepare_ships(a_id, b_id);
    set_ship_position(
        &mut harness.server_app,
        a_id,
        300.0,
        0.0,
        std::f32::consts::FRAC_PI_2,
    );
    harness.run_frames(10);

    let hp_before = harness.recorded_b().latest_ship(b_id).map(|ship| ship.hp);
    assert!(
        hp_before.is_some(),
        "B should have a ShipState before the hit"
    );

    harness.lock_a();
    let damaged = harness.run_until(60, |harness| {
        harness
            .recorded_b()
            .latest_ship(b_id)
            .is_some_and(|ship| ship.hp < hp_before.unwrap())
    });
    assert!(damaged, "B should receive reduced hp in WorldSnapshot");
}

#[test]
fn multiplayer_full_scenario_a_b_fire_damage() {
    let mut harness = Harness::new();
    harness.wait_for_handshake();
    let (a_id, b_id) = harness.ship_ids();
    harness.prepare_ships(a_id, b_id);

    harness.move_a_out_of_aoi(a_id);
    harness.return_a_to_aoi(a_id);

    set_ship_position(
        &mut harness.server_app,
        a_id,
        300.0,
        0.0,
        std::f32::consts::FRAC_PI_2,
    );
    harness.run_frames(10);
    assert!(
        harness.recorded_b().contains_ship(a_id),
        "A must be in B's AOI before the broadside"
    );

    let hp_before = harness.recorded_b().latest_ship(b_id).unwrap().hp;
    harness.lock_a();
    let damaged = harness.run_until(60, |harness| {
        harness
            .recorded_b()
            .latest_ship(b_id)
            .is_some_and(|ship| ship.hp < hp_before)
    });
    assert!(damaged, "full scenario should apply broadside damage");

    set_ship_hp(&mut harness.server_app, b_id, 1);
    reset_ship_battery(&mut harness.server_app, a_id);
    harness.run_frames(2);
    let loadouts_before = harness.recorded_b().loadouts.len();
    // O alvo segue travado: a próxima salva sai sozinha.
    let destroyed = harness.run_until(60, |harness| {
        harness
            .recorded_b()
            .destroyed
            .iter()
            .any(|message| message.ship_id == b_id)
    });
    assert!(destroyed, "full scenario should observe ShipDestroyed");

    // O casco do respawn nasce vazio e o client fica sabendo: sem isso a UI
    // mostrava como equipado o que afundou junto.
    let refreshed = harness.run_until(60, |harness| {
        harness.recorded_b().loadouts.len() > loadouts_before
    });
    assert!(refreshed, "respawn deve reenviar o loadout");
    let latest = harness.recorded_b().loadouts.last().unwrap();
    assert!(
        latest.slots.iter().all(|line| !line.equipped),
        "navio novo não tem nada instalado: {latest:?}"
    );
}

fn with_ship(app: &mut App, ship_id: u32, change: impl FnOnce(&mut ServerShip)) {
    let world = app.world_mut();
    let mut query = world.query::<&mut ServerShip>();
    if let Some(mut ship) = query.iter_mut(world).find(|ship| ship.ship_id == ship_id) {
        change(&mut ship);
    }
}

fn read_ship<T>(app: &mut App, ship_id: u32, read: impl FnOnce(&ServerShip) -> T) -> Option<T> {
    let world = app.world_mut();
    let mut query = world.query::<&ServerShip>();
    query
        .iter(world)
        .find(|ship| ship.ship_id == ship_id)
        .map(read)
}

fn dev_items(app: &App) -> &marvyr_server::net::DevItems {
    app.world().resource::<marvyr_server::net::DevItems>()
}

/// Põe Madeira no porão: o navio nasce vazio (nada de recurso no spawn).
fn load_timber(app: &mut App, ship_id: u32, quantity: u32) {
    let dev = dev_items(app);
    let (catalog, timber) = (dev.catalog.clone(), dev.timber);
    with_ship(app, ship_id, |ship| {
        ship.hold
            .insert(
                &catalog,
                marvyr_domain_items::ItemInstance::new_resource(
                    marvyr_shared::ids::ItemInstanceId::new(),
                    timber,
                    quantity,
                ),
            )
            .expect("madeira cabe no porão vazio");
    });
}

fn quantity_of(ship: &ServerShip, item: marvyr_shared::ids::ItemDefinitionId) -> u32 {
    ship.hold
        .items()
        .iter()
        .filter(|custody| custody.instance.definition == item)
        .map(|custody| custody.instance.quantity)
        .sum()
}

/// MV-061: hello sem sessão recebe o MOTIVO e não ganha navio.
#[test]
fn hello_without_session_is_rejected_with_reason() {
    let mut harness = Harness::with_identities("multiplayer-token-a", "   ");
    let rejected = harness.run_until(300, |harness| harness.recorded_b().welcome.is_some());
    assert!(rejected, "B deveria receber ServerWelcome");
    let welcome = harness.recorded_b().welcome.clone().unwrap();
    assert!(!welcome.accepted);
    assert_eq!(welcome.reason, marvyr_server::session::REASON_NO_SESSION);
    harness.run_frames(30);
    assert!(
        harness.recorded_b().assign.is_none(),
        "recusado não ganha navio"
    );
    assert!(harness.recorded_a().assign.is_some(), "A entra normalmente");
}

/// MV-061: navio avariado e colado é tomado por abordagem; a carga passa
/// INTEIRA para o wreck (sem a perda do afundamento a tiro).
#[test]
fn boarding_captures_a_crippled_ship_with_all_its_cargo() {
    let mut harness = Harness::new();
    harness.wait_for_handshake();
    let (a_id, b_id) = harness.ship_ids();
    harness.prepare_ships(a_id, b_id);
    set_ship_position(&mut harness.server_app, b_id, 280.0, 0.0, 0.0);
    load_timber(&mut harness.server_app, b_id, 15);
    let timber = dev_items(&harness.server_app).timber;
    let cargo_before = read_ship(&mut harness.server_app, b_id, |ship| {
        quantity_of(ship, timber)
    })
    .expect("B existe");
    assert!(cargo_before > 0, "B carrega madeira");

    let mut captured = false;
    for _ in 0..8 {
        with_ship(&mut harness.server_app, a_id, |ship| {
            ship.sea.crew = 8;
            ship.sea.board_cooldown = 0.0;
        });
        with_ship(&mut harness.server_app, b_id, |ship| {
            ship.hp = 5;
            ship.sea.crew = 0;
        });
        harness.send_a(&marvyr_protocol::BoardShip {
            target_ship_id: b_id,
        });
        captured = harness.run_until(30, |harness| {
            harness
                .recorded_b()
                .destroyed
                .iter()
                .any(|message| message.ship_id == b_id)
        });
        if captured {
            break;
        }
    }
    assert!(captured, "abordagem com 8 contra 0 deveria render o navio");
    harness.run_frames(5);
    let world = harness.server_app.world_mut();
    let mut wrecks = world.query::<&marvyr_server::net::ServerWreck>();
    let in_wreck: u32 = wrecks
        .iter(world)
        .flat_map(|wreck| wreck.chest.items().to_vec())
        .filter(|custody| custody.instance.definition == timber)
        .map(|custody| custody.instance.quantity)
        .sum();
    assert_eq!(
        in_wreck, cargo_before,
        "carga do navio rendido passa inteira"
    );
}

/// MV-061: tripulação no reparo, parada e fora de combate, troca Madeira
/// do porão por casco.
#[test]
fn repair_at_sea_spends_timber_to_restore_hull() {
    let mut harness = Harness::new();
    harness.wait_for_handshake();
    let (a_id, _) = harness.ship_ids();
    load_timber(&mut harness.server_app, a_id, 15);
    let timber = dev_items(&harness.server_app).timber;
    let (max_hp, timber_before) = read_ship(&mut harness.server_app, a_id, |ship| {
        (ship.stats.max_hp, quantity_of(ship, timber))
    })
    .unwrap();
    set_ship_hp(&mut harness.server_app, a_id, max_hp / 2);
    let damaged = harness.run_until(60, |harness| {
        harness
            .recorded_a()
            .latest_ship(a_id)
            .is_some_and(|ship| ship.hp == max_hp / 2)
    });
    assert!(damaged, "snapshot deveria mostrar o casco avariado");
    harness.send_a(&marvyr_protocol::SetRepair { active: true });
    let repaired = harness.run_until(200, |harness| {
        harness
            .recorded_a()
            .latest_ship(a_id)
            .is_some_and(|ship| ship.hp > max_hp / 2)
    });
    assert!(repaired, "casco deveria subir com o reparo");
    let timber_after = read_ship(&mut harness.server_app, a_id, |ship| {
        quantity_of(ship, timber)
    })
    .unwrap();
    assert!(timber_after < timber_before, "reparo consome Madeira");
}

/// MV-061: mapa do tesouro + parado no X = recurso bruto no porão.
#[test]
fn digging_at_the_map_spot_trades_the_map_for_treasure() {
    let mut harness = Harness::new();
    harness.wait_for_handshake();
    let (a_id, _) = harness.ship_ids();
    let (map_item, pearl) = {
        let dev = dev_items(&harness.server_app);
        (dev.treasure_map, dev.abyssal_pearl)
    };
    let map_id = marvyr_shared::ids::ItemInstanceId(uuid::Uuid::from_u128(0));
    let classic = marvyr_domain_world::WorldMap::vertical_slice();
    let island =
        marvyr_domain_world::treasure::island_for_map(&classic.features().hidden_islands, 0)
            .expect("mapa clássico tem ilhas ocultas");
    {
        let catalog = dev_items(&harness.server_app).catalog.clone();
        with_ship(&mut harness.server_app, a_id, |ship| {
            ship.hold
                .insert(
                    &catalog,
                    marvyr_domain_items::ItemInstance::new_resource(map_id, map_item, 1),
                )
                .expect("mapa cabe");
        });
    }
    set_ship_position(
        &mut harness.server_app,
        a_id,
        island.dig_x,
        island.dig_y,
        0.0,
    );
    harness.run_frames(3);
    harness.send_a(&marvyr_protocol::DigTreasure);
    let dug = harness.run_until(400, |harness| {
        let server = &harness.server_app;
        let world = server.world();
        world
            .iter_entities()
            .filter_map(|entity| entity.get::<ServerShip>())
            .any(|ship| ship.ship_id == a_id && quantity_of(ship, pearl) > 0)
    });
    assert!(dug, "8 s cavando deveriam render pérolas");
    let still_has_map = read_ship(&mut harness.server_app, a_id, |ship| {
        quantity_of(ship, map_item)
    })
    .unwrap();
    assert_eq!(still_has_map, 0, "o mapa é consumido");
}

/// v26: mapa Raro — ao descer o escaler os guardiões e o Kraken nascem
/// (uma vez só), e o baú sai engordado pelo perigo.
#[test]
fn rare_map_wakes_its_perils_once_and_pays_for_them() {
    use marvyr_domain_items::MapMod;
    use marvyr_server::npc::NpcRole;
    let mut harness = Harness::new();
    harness.wait_for_handshake();
    let (a_id, _) = harness.ship_ids();
    let (map_item, pearl) = {
        let dev = dev_items(&harness.server_app);
        (dev.treasure_map, dev.abyssal_pearl)
    };
    let map_id = marvyr_shared::ids::ItemInstanceId(uuid::Uuid::from_u128(0));
    let classic = marvyr_domain_world::WorldMap::vertical_slice();
    let island =
        marvyr_domain_world::treasure::island_for_map(&classic.features().hidden_islands, 0)
            .expect("mapa clássico tem ilhas ocultas");
    let mods = vec![MapMod::Guarded, MapMod::Kraken, MapMod::Rumored];
    {
        let catalog = dev_items(&harness.server_app).catalog.clone();
        let map = marvyr_domain_items::ItemInstance {
            quality: Some(marvyr_domain_items::Quality {
                rarity: marvyr_domain_items::Rarity::Rare,
                affixes: Vec::new(),
                gems: Vec::new(),
                map_mods: mods.clone(),
                aspect: None,
            }),
            ..marvyr_domain_items::ItemInstance::new_resource(map_id, map_item, 1)
        };
        with_ship(&mut harness.server_app, a_id, |ship| {
            ship.hold.insert(&catalog, map).expect("mapa cabe");
        });
    }
    set_ship_position(
        &mut harness.server_app,
        a_id,
        island.dig_x,
        island.dig_y,
        0.0,
    );
    harness.run_frames(3);
    let guardians_before = count_npcs(&mut harness.server_app, NpcRole::Guardian);
    let krakens_before = count_npcs(&mut harness.server_app, NpcRole::Kraken);
    harness.send_a(&marvyr_protocol::DigTreasure);
    harness.run_frames(10);
    assert_eq!(
        count_npcs(&mut harness.server_app, NpcRole::Guardian),
        guardians_before + 2
    );
    assert_eq!(
        count_npcs(&mut harness.server_app, NpcRole::Kraken),
        krakens_before + 1
    );
    // Recomeçar a escavação não chama outra leva.
    harness.send_a(&marvyr_protocol::DigTreasure);
    harness.run_frames(10);
    assert_eq!(
        count_npcs(&mut harness.server_app, NpcRole::Guardian),
        guardians_before + 2
    );
    // Guardião atirando interrompe o escaler (é o perigo). Aqui eles vão
    // para o fundo e a escavação recomeça em paz para medir o baú.
    {
        let world = harness.server_app.world_mut();
        let perils: Vec<bevy::ecs::entity::Entity> = world
            .query::<(bevy::ecs::entity::Entity, &marvyr_server::npc::NpcShip)>()
            .iter(world)
            .filter(|(_, npc)| matches!(npc.role, NpcRole::Guardian | NpcRole::Kraken))
            .map(|(entity, _)| entity)
            .collect();
        for entity in perils {
            world.despawn(entity);
        }
    }
    harness.send_a(&marvyr_protocol::DigTreasure);
    let dug = harness.run_until(400, |harness| {
        let world = harness.server_app.world();
        world
            .iter_entities()
            .filter_map(|entity| entity.get::<ServerShip>())
            .any(|ship| ship.ship_id == a_id && quantity_of(ship, pearl) > 0)
    });
    assert!(dug, "a escavação termina");
    let pearls = read_ship(&mut harness.server_app, a_id, |ship| {
        quantity_of(ship, pearl)
    })
    .unwrap();
    assert_eq!(
        pearls,
        marvyr_domain_items::map_mod::scaled_treasure(3, &mods),
        "o perigo engorda o baú"
    );
}

fn count_npcs(app: &mut App, role: marvyr_server::npc::NpcRole) -> usize {
    let world = app.world_mut();
    let mut query = world.query::<&marvyr_server::npc::NpcShip>();
    query.iter(world).filter(|npc| npc.role == role).count()
}

/// MV-061: o diretor de eventos materializa o Kraken e a Frota do Tesouro
/// (galeão + duas escoltas); trocar de evento recolhe o anterior.
#[test]
fn sea_events_spawn_kraken_and_treasure_fleet() {
    use marvyr_server::npc::NpcRole;
    let mut harness = Harness::new();
    harness.wait_for_handshake();
    harness
        .server_app
        .world_mut()
        .resource_mut::<marvyr_server::seafaring::ServerSeaEvents>()
        .force(marvyr_domain_world::SeaEventKind::Kraken);
    harness.run_frames(3);
    assert_eq!(count_npcs(&mut harness.server_app, NpcRole::Kraken), 1);

    harness
        .server_app
        .world_mut()
        .resource_mut::<marvyr_server::seafaring::ServerSeaEvents>()
        .force(marvyr_domain_world::SeaEventKind::TreasureFleet);
    harness.run_frames(3);
    assert_eq!(count_npcs(&mut harness.server_app, NpcRole::Kraken), 0);
    assert_eq!(
        count_npcs(&mut harness.server_app, NpcRole::TreasureGalleon),
        1
    );
    assert_eq!(count_npcs(&mut harness.server_app, NpcRole::Escort), 2);
    let metrics = harness
        .server_app
        .world()
        .resource::<marvyr_server::net::Metrics>();
    assert_eq!(metrics.sea_events_started, 2);
}

/// MV-062: recusa de coleta chega com motivo legível, e a mensagem v16
/// anexada no fim do registro (`OnboardingProgress`) decodifica no servidor.
#[test]
fn failed_gather_carries_reason_and_onboarding_is_recorded() {
    let mut harness = Harness::new();
    harness.wait_for_handshake();

    harness.send_a(&OnboardingProgress {
        step: 0,
        skipped: false,
    });
    harness.send_a(&GatherNode { node_id: 9_999 });
    let answered = harness.run_until(120, |harness| !harness.recorded_a().gathers.is_empty());
    assert!(answered, "server should answer the gather intent");

    let result = &harness.recorded_a().gathers[0];
    assert!(!result.success);
    assert_eq!(result.gathered, 0);
    assert!(!result.reason.is_empty(), "refusal must explain itself");

    let metrics = harness
        .server_app
        .world()
        .resource::<marvyr_server::net::Metrics>();
    assert_eq!(metrics.onboarding.welcomed(), 1);
}

/// v32: Maré Sangrenta — baú recusa quem chega sem cinza, abre com 10
/// (cinza sai do porão, bruto raro entra), saqueadores chegam em ondas e
/// tudo afunda quando a maré baixa.
#[test]
fn blood_tide_chest_eats_ash_and_the_tide_takes_its_reavers() {
    use marvyr_domain_world::SeaEventKind;
    use marvyr_server::blood_tide::{BloodTide, ASH_PER_CHEST};
    use marvyr_server::npc::NpcRole;
    let mut harness = Harness::new();
    harness.wait_for_handshake();
    let (a_id, _) = harness.ship_ids();
    let (ash, pearl, catalog) = {
        let dev = dev_items(&harness.server_app);
        (dev.blood_ash, dev.abyssal_pearl, dev.catalog.clone())
    };
    harness
        .server_app
        .world_mut()
        .resource_mut::<marvyr_server::seafaring::ServerSeaEvents>()
        .force(SeaEventKind::BloodTide);
    harness.run_frames(5);
    let chests = harness
        .server_app
        .world()
        .resource::<BloodTide>()
        .chests
        .clone();
    assert_eq!(chests.len(), 3, "a maré sobe com três baús");
    let (cx, cy) = chests[0];

    // Sem cinza: o baú fica fechado.
    set_ship_position(&mut harness.server_app, a_id, cx, cy, 0.0);
    harness.run_frames(5);
    assert_eq!(
        harness
            .server_app
            .world()
            .resource::<BloodTide>()
            .chests
            .len(),
        3
    );

    // Com 12 cinzas: suga 10, paga pérolas, o baú some.
    with_ship(&mut harness.server_app, a_id, |ship| {
        let found = marvyr_domain_items::ItemInstance::new_resource(
            marvyr_shared::ids::ItemInstanceId::new(),
            ash,
            ASH_PER_CHEST + 2,
        );
        ship.hold.insert(&catalog, found).expect("cinza cabe");
    });
    set_ship_position(&mut harness.server_app, a_id, cx, cy, 0.0);
    harness.run_frames(5);
    assert_eq!(
        harness
            .server_app
            .world()
            .resource::<BloodTide>()
            .chests
            .len(),
        2
    );
    let (ash_left, pearls) = read_ship(&mut harness.server_app, a_id, |ship| {
        (quantity_of(ship, ash), quantity_of(ship, pearl))
    })
    .unwrap();
    assert_eq!(ash_left, 2, "só 10 cinzas saem do porão");
    assert!(pearls > 0, "o baú paga bruto raro");

    // Ondas: o primeiro saqueador chega (sempre elite).
    let came = harness.run_until(900, |harness| {
        let world = harness.server_app.world();
        world
            .iter_entities()
            .filter_map(|entity| entity.get::<marvyr_server::npc::NpcShip>())
            .any(|npc| npc.role == NpcRole::Reaver && npc.elite != 0)
    });
    assert!(came, "a maré traz saqueadores de elite");

    // Outro evento no lugar: a maré baixa e leva tudo.
    harness
        .server_app
        .world_mut()
        .resource_mut::<marvyr_server::seafaring::ServerSeaEvents>()
        .force(SeaEventKind::Tempest);
    harness.run_frames(5);
    assert_eq!(count_npcs(&mut harness.server_app, NpcRole::Reaver), 0);
    assert!(harness
        .server_app
        .world()
        .resource::<BloodTide>()
        .chests
        .is_empty());
}
