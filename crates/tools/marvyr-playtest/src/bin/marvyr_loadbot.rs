//! Teste de carga: N capitães sem janela (o mesmo `ClientNetPlugin` do
//! jogo, por UDP) entram num servidor já rodando e navegam com leme e velas
//! aleatórios. No fim, quantos entraram e o ritmo de snapshots que cada um
//! recebeu — servidor engasgando aparece como snapshot rareando.
//!
//! Uso: `marvyr_loadbot [bots=20] [segundos=60]`, com `MARVYR_PORT` (5094
//! por padrão; nunca a 5077, que pode ser o servidor local do usuário).
//! O servidor precisa de `MARVYR_ALLOW_ANON=1` (cada bot usa um token
//! anônimo próprio).

use std::net::{Ipv4Addr, SocketAddr};
use std::time::{Duration, Instant};

use bevy::input::InputPlugin;
use bevy::prelude::*;
use bevy::state::app::StatesPlugin;
use lightyear::prelude::ClientReceiveMessage;
use marvyr_client::net::{
    netcode_config, ClientIdentity, ClientNetOverride, ClientNetPlugin, MyShip, ShipInputOverride,
};
use marvyr_protocol::{ShipInput, WorldSnapshot};

/// Snapshots por segundo que o servidor promete (abaixo disso, engasgou).
const EXPECTED_HZ: f64 = 20.0;

#[derive(Resource, Default)]
struct Pulse {
    snapshots: u32,
    worst_gap: Duration,
    last: Option<Instant>,
}

fn count_snapshots(
    mut events: EventReader<ClientReceiveMessage<WorldSnapshot>>,
    mut pulse: ResMut<Pulse>,
) {
    for _ in events.read() {
        let now = Instant::now();
        if let Some(last) = pulse.last {
            pulse.worst_gap = pulse.worst_gap.max(now - last);
        }
        pulse.last = Some(now);
        pulse.snapshots += 1;
    }
}

/// Leme e velas trocam a cada poucos segundos (semente por bot).
fn steer(time: Res<Time>, mut input: ResMut<ShipInputOverride>, mut state: Local<(f32, u64)>) {
    state.0 -= time.delta_secs();
    if state.0 > 0.0 {
        return;
    }
    let (clock, seed) = &mut *state;
    if *seed == 0 {
        *seed = uuid::Uuid::new_v4().as_u64_pair().0 | 1;
    }
    let mut next = || {
        *seed ^= *seed << 13;
        *seed ^= *seed >> 7;
        *seed ^= *seed << 17;
        (*seed >> 40) as f32 / (1u64 << 24) as f32
    };
    *clock = 2.0 + next() * 4.0;
    input.0 = Some(ShipInput {
        throttle: 0.4 + next() * 0.6,
        turn: next() * 2.0 - 1.0,
    });
}

fn bot(server: SocketAddr, index: usize) -> App {
    let mut app = App::new();
    app.add_plugins((MinimalPlugins, InputPlugin, StatesPlugin));
    app.insert_resource(Time::<Fixed>::from_hz(30.0));
    app.insert_resource(ClientNetOverride(Some(netcode_config(server))));
    app.insert_resource(ClientIdentity(Some(format!(
        "loadbot-{index}-{}",
        uuid::Uuid::new_v4().simple()
    ))));
    app.insert_resource(ShipInputOverride::default());
    app.init_resource::<Pulse>();
    app.add_plugins(ClientNetPlugin);
    app.add_systems(Update, (count_snapshots, steer));
    app.finish();
    app.cleanup();
    app
}

fn main() {
    let mut args = std::env::args().skip(1);
    let bots: usize = args.next().and_then(|a| a.parse().ok()).unwrap_or(20);
    let secs: u64 = args.next().and_then(|a| a.parse().ok()).unwrap_or(60);
    let port: u16 = std::env::var("MARVYR_PORT")
        .ok()
        .and_then(|p| p.parse().ok())
        .unwrap_or(5094);
    assert_ne!(port, 5077, "5077 pode ser o servidor local do usuário");
    let server = SocketAddr::from((Ipv4Addr::LOCALHOST, port));
    println!("marvyr_loadbot: {bots} bots contra {server} por {secs}s");

    let mut apps: Vec<App> = (0..bots).map(|i| bot(server, i)).collect();
    let started = Instant::now();
    let frame = Duration::from_secs_f64(1.0 / 60.0);
    while started.elapsed() < Duration::from_secs(secs) {
        let tick = Instant::now();
        for app in &mut apps {
            app.update();
        }
        if let Some(rest) = frame.checked_sub(tick.elapsed()) {
            std::thread::sleep(rest);
        }
    }

    let elapsed = started.elapsed().as_secs_f64();
    let mut joined = 0;
    let mut rates = Vec::new();
    let mut worst = Duration::ZERO;
    for app in &apps {
        let world = app.world();
        if world.resource::<MyShip>().0.is_some() {
            joined += 1;
        }
        let pulse = world.resource::<Pulse>();
        rates.push(f64::from(pulse.snapshots) / elapsed);
        worst = worst.max(pulse.worst_gap);
    }
    rates.sort_by(f64::total_cmp);
    let median = rates.get(rates.len() / 2).copied().unwrap_or(0.0);
    let lowest = rates.first().copied().unwrap_or(0.0);
    println!("entraram: {joined}/{bots}");
    println!("snapshots/s: mediana {median:.1}, pior bot {lowest:.1} (esperado ~{EXPECTED_HZ})");
    println!("maior intervalo sem snapshot: {} ms", worst.as_millis());
    let healthy = joined == bots && lowest >= EXPECTED_HZ * 0.8;
    println!("{}", if healthy { "OK" } else { "SERVIDOR ENGASGOU" });
    std::process::exit(if healthy { 0 } else { 1 });
}
