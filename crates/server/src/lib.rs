pub mod abyss;
pub mod active_combat;
pub mod aoi;
pub mod blood_tide;
pub mod bottle;
pub mod captains;
pub mod companies;
pub mod cosmetics;
pub mod crafting;
pub mod currents;
pub mod cursed_cargo;
pub mod fishing;
pub mod flasks;
pub mod flotsam;
pub mod fortress;
pub mod freight;
pub mod fury;
pub mod gems;
pub mod guild;
pub mod gunnery;
pub mod lighthouse;
pub mod loadout;
pub mod market;
pub mod melee;
pub mod mentor;
pub mod morale;
pub mod net;
pub mod nodes;
pub mod npc;
pub mod party;
pub mod persist;
mod playtest;
pub mod plugin;
pub mod portals;
pub mod progress;
pub mod renown;
pub mod reputation;
pub mod seafaring;
pub mod season;
pub mod session;
pub mod sets;
pub mod talents;
pub mod telemetry;
pub mod territory;
pub mod weather;
pub mod world_boss;

pub use plugin::ServerPlugin;

use bevy::app::TerminalCtrlCHandlerPlugin;
use bevy::prelude::*;
use tracing_subscriber::EnvFilter;

/// Headless server app shared by `marvyr-server` and the playtest child.
pub fn run_headless() {
    tracing_subscriber::fmt()
        .with_env_filter(
            EnvFilter::try_from_default_env()
                .unwrap_or_else(|_| EnvFilter::new("info,marvyr_server=info")),
        )
        .init();

    let auth = session::AuthConfig::from_env();
    let database = std::env::var("MARVYR_DATABASE_URL").ok();
    // Fail-closed (handoff MV-061): produção nunca sobe sem persistência.
    assert!(
        !auth.production || database.is_some(),
        "MARVYR_ENV=production exige MARVYR_DATABASE_URL"
    );
    tracing::info!(
        version = marvyr_protocol::VERSION_LABEL,
        build = marvyr_protocol::BUILD_SHA,
        protocol = marvyr_protocol::PROTOCOL_VERSION,
        listening = %net::server_addr(),
        persistence = database
            .as_deref()
            .map(session::redact_url)
            .unwrap_or_else(|| String::from("sem banco")),
        environment = if auth.production { "production" } else { "development" },
        login = if auth.jwt_secret.is_some() { "jwt" } else { "desligado" },
        anonymous = auth.allow_anon,
        max_clients = auth.max_clients,
        "Marvyr Server"
    );

    let mut app = App::new();
    // Sem `run_loop` o MinimalPlugins gira o loop sem pausa (130%+ de CPU
    // ocioso). 60 Hz de frame folga o FixedUpdate de 30 Hz e a rede.
    app.add_plugins(
        MinimalPlugins.set(bevy::app::ScheduleRunnerPlugin::run_loop(
            std::time::Duration::from_secs_f64(1.0 / 60.0),
        )),
    )
    .insert_resource(auth)
    .add_plugins(TerminalCtrlCHandlerPlugin)
    .add_plugins(ServerPlugin)
    .add_plugins(net::ServerNetPlugin);

    // MV-061: resumo de sessão e SIGTERM gracioso em toda execução.
    playtest::install(&mut app);
    app.run();
}
