pub mod affixes;
pub mod assets;
pub mod audio;
pub mod blood_tide;
pub mod camera;
pub mod chart;
pub mod config;
pub mod crafting;
pub mod fishing;
pub mod flasks;
pub mod gems;
pub mod guild;
pub mod help;
pub mod hud;
pub mod i18n;
pub mod i18n_extra;
pub mod input;
pub mod inventory;
pub mod juice;
pub mod lighthouse;
pub mod logbook;
pub mod market;
pub mod net;
pub mod nodes;
pub mod onboarding;
pub mod playtest;
pub mod plugin;
pub mod port_screen;
pub mod portals;
pub mod renown;
pub mod seafaring;
pub mod session;
pub mod ship;
pub mod talents;
pub mod ui;
pub mod vfx;
pub mod wanted_hud;
pub mod weather;
pub mod world;
pub mod world_boss;
pub mod zone;

pub use plugin::ClientPlugin;

use bevy::asset::AssetPlugin;
use bevy::prelude::*;

/// Window app shared by `marvyr-client` and the playtest binary.
pub fn windowed_app() -> App {
    install_crash_log();
    let mut app = App::new();
    app.add_plugins(
        DefaultPlugins
            .set(AssetPlugin {
                file_path: asset_root(),
                // Nenhum asset tem `.meta`; no browser a busca dele cai no
                // fallback do servidor (index.html) e quebra o carregamento.
                meta_check: bevy::asset::AssetMetaCheck::Never,
                ..default()
            })
            .set(ImagePlugin::default_nearest())
            .set(WindowPlugin {
                primary_window: Some(Window {
                    title: format!("Marvyr {}", marvyr_protocol::VERSION_LABEL),
                    resolution: dev_window_size().into(),
                    // Captura de dev não rouba o foco (nem o teclado) de
                    // quem está usando a máquina.
                    focused: std::env::var_os("MARVYR_SHOT").is_none(),
                    // Só vale no browser: o canvas ocupa a página toda.
                    fit_canvas_to_parent: true,
                    ..default()
                }),
                ..default()
            }),
    );
    app.add_plugins(ClientPlugin);
    if let Some(shots) = playtest::DevScreenshotPlugin::from_env() {
        app.add_plugins(shots);
    }
    app
}

/// Tamanho lógico da janela; MARVYR_WINDOW=1366x768 testa telas de notebook.
fn dev_window_size() -> (f32, f32) {
    std::env::var("MARVYR_WINDOW")
        .ok()
        .and_then(|value| {
            let (w, h) = value.split_once('x')?;
            Some((w.trim().parse().ok()?, h.trim().parse().ok()?))
        })
        .unwrap_or((1280.0, 720.0))
}

fn asset_root() -> String {
    config::asset_root().to_string_lossy().into_owned()
}

/// Panic no build de release some sem rastro (Finder não mostra stderr):
/// grava mensagem + backtrace em `<data_dir>/crash.log` para o jogador
/// anexar ao reportar. Mantém o hook padrão (stderr) em seguida.
fn install_crash_log() {
    let default_hook = std::panic::take_hook();
    std::panic::set_hook(Box::new(move |info| {
        use std::io::Write;
        let dir = config::data_dir();
        let _ = std::fs::create_dir_all(&dir);
        if let Ok(mut file) = std::fs::OpenOptions::new()
            .create(true)
            .append(true)
            .open(dir.join("crash.log"))
        {
            let _ = writeln!(
                file,
                "=== Marvyr {} ({}) ===\n{info}\n{}\n",
                marvyr_protocol::VERSION_LABEL,
                marvyr_protocol::BUILD_SHA,
                std::backtrace::Backtrace::force_capture()
            );
        }
        default_hook(info);
    }));
}
