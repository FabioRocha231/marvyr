//! Som do mar (MF-059). Efeitos pontuais nascem dos [`SeaEvent`] (derivados
//! do snapshot em `juice.rs`) e tocam espacializados: pan estéreo pelo
//! `SpatialListener` da câmera e volume com queda linear até silenciar a
//! [`HEARING_RANGE`] metros. Ambiente (ondas, vento, rangido), UI (clique,
//! moedas, sino de zona PvP) e música em loop (O liga/desliga).

use bevy::audio::{SpatialScale, Volume};
use bevy::prelude::*;
use marvyr_domain_world::RiskTier;

use crate::juice::SeaEvent;
use crate::market::MarketFeedback;
use crate::net::{MyDocked, MyShip};
use crate::ship::ShipVisual;
use crate::ui::UiButton;
use crate::zone::CurrentZone;

/// Volume geral do jogo (todo som passa por aqui).
pub const MASTER_VOLUME: f32 = 0.7;
const MUSIC_VOLUME: f32 = 0.22;
const WAVES_VOLUME: f32 = 0.35;
const WIND_VOLUME: f32 = 0.45;

/// A partir daqui (metros da câmera) não se ouve mais nada.
pub const HEARING_RANGE: f32 = 600.0;
/// Distância entre os "ouvidos" da câmera, em metros de mundo: quanto
/// menor, mais forte o pan de um som fora do centro.
pub const EAR_GAP: f32 = 300.0;
/// O rodio atenua por 1/d² acima de 1 unidade. Com esta escala todo o
/// alcance audível fica abaixo de 1 e a queda é só a nossa, linear.
const SPATIAL_SCALE: SpatialScale = SpatialScale::new_2d(1.0 / (HEARING_RANGE + 100.0));
const _: () = assert!(HEARING_RANGE * SPATIAL_SCALE.0.x < 1.0);

/// Ganho por distância: 1 colado na câmera, 0 em [`HEARING_RANGE`].
pub fn distance_gain(distance: f32) -> f32 {
    (1.0 - distance / HEARING_RANGE).clamp(0.0, 1.0)
}

#[derive(Resource)]
struct SoundHandles {
    cannon: Handle<AudioSource>,
    hull_hit: Handle<AudioSource>,
    splash: Handle<AudioSource>,
    sinking: Handle<AudioSource>,
    creak: Handle<AudioSource>,
    click: Handle<AudioSource>,
    coins: Handle<AudioSource>,
    bell: Handle<AudioSource>,
}

#[derive(Component)]
struct Music;

#[derive(Component)]
struct WindLoop;

/// Música ligada/desligada (tecla O; M é a carta, N cancela ordem).
#[derive(Resource)]
pub struct MusicEnabled(pub bool);

fn setup_audio(mut commands: Commands, server: Res<AssetServer>) {
    commands.insert_resource(SoundHandles {
        cannon: server.load("external/oga-battle-at-sea/cannon_fire_1.ogg"),
        hull_hit: server.load("external/oga-battle-at-sea/cannon_hit_ship_short.ogg"),
        splash: server.load("external/oga-battle-at-sea/cannon_miss_1.ogg"),
        sinking: server.load("external/oga-battle-at-sea/ship_destroyed_short.ogg"),
        creak: server.load("external/kenney-rpg-audio/creak1.ogg"),
        click: server.load("external/kenney-interface-sounds/click_002.ogg"),
        coins: server.load("external/kenney-rpg-audio/handleCoins.ogg"),
        bell: server.load("external/kenney-impact-sounds/impactBell_heavy_000.ogg"),
    });
    let looped = |volume: f32| PlaybackSettings::LOOP.with_volume(Volume::new(volume));
    commands.spawn((
        AudioPlayer::new(server.load("external/oga-beach-ocean-waves/waves_loop.ogg")),
        looped(WAVES_VOLUME * MASTER_VOLUME),
    ));
    commands.spawn((
        AudioPlayer::new(server.load("external/oga-short-wind/wind_loop.ogg")),
        looped(0.0),
        WindLoop,
    ));
    commands.spawn((
        AudioPlayer::new(
            server.load("external/oga-sailors-chant/oga_jam_menu_music_loopable_0.ogg"),
        ),
        looped(MUSIC_VOLUME * MASTER_VOLUME),
        Music,
    ));
}

/// Pseudo-aleatório barato em [0, 1) (só para variar tom e volume).
fn jitter(seed: f32) -> f32 {
    ((seed * 12.9898).sin() * 43_758.547).fract().abs()
}

/// Efeito pontual no mundo: espacializado e atenuado pela distância.
fn play_at(
    commands: &mut Commands,
    sound: &Handle<AudioSource>,
    at: Vec2,
    listener: Vec2,
    volume: f32,
    seed: f32,
) {
    let gain = distance_gain(listener.distance(at)) * volume * MASTER_VOLUME;
    if gain <= 0.01 {
        return;
    }
    let r = jitter(seed + at.x * 0.37 + at.y);
    commands.spawn((
        AudioPlayer::new(sound.clone()),
        PlaybackSettings::DESPAWN
            .with_spatial(true)
            .with_spatial_scale(SPATIAL_SCALE)
            .with_volume(Volume::new(gain * (0.85 + 0.15 * r)))
            .with_speed(0.92 + 0.16 * r),
        Transform::from_translation(at.extend(0.0)),
    ));
}

/// Som de interface: sem posição no mundo.
fn play_ui(commands: &mut Commands, sound: &Handle<AudioSource>, volume: f32) {
    play_ui_pitched(commands, sound, volume, 1.0);
}

/// Como `play_ui`, com tom próprio (o sino agudo vira o "tim" da gema).
fn play_ui_pitched(commands: &mut Commands, sound: &Handle<AudioSource>, volume: f32, speed: f32) {
    commands.spawn((
        AudioPlayer::new(sound.clone()),
        PlaybackSettings::DESPAWN
            .with_volume(Volume::new(volume * MASTER_VOLUME))
            .with_speed(speed),
    ));
}

/// v25: frasco bebido — gole (borrifo agudo) e, na Fúria, o estalo do clique.
fn flask_sounds(
    mut commands: Commands,
    sounds: Option<Res<SoundHandles>>,
    mut events: EventReader<crate::flasks::FlaskDrunk>,
) {
    let Some(sounds) = sounds else { return };
    for event in events.read().filter(|event| event.mine) {
        play_ui_pitched(&mut commands, &sounds.splash, 0.7, 1.7);
        if event.kind == marvyr_domain_combat::FlaskKind::Fury {
            play_ui_pitched(&mut commands, &sounds.click, 0.8, 0.7);
        }
    }
}

/// v24: gema encaixada tine (sino agudo + clique); tirada chacoalha.
fn gem_sounds(
    mut commands: Commands,
    sounds: Option<Res<SoundHandles>>,
    mut events: EventReader<crate::gems::GemSound>,
) {
    let Some(sounds) = sounds else { return };
    for event in events.read() {
        if event.socketed {
            play_ui_pitched(&mut commands, &sounds.bell, 0.55, 2.2);
            play_ui_pitched(&mut commands, &sounds.click, 0.8, 1.4);
        } else {
            play_ui_pitched(&mut commands, &sounds.coins, 0.7, 1.3);
        }
    }
}

fn play_sea_events(
    mut commands: Commands,
    time: Res<Time>,
    sounds: Option<Res<SoundHandles>>,
    mut events: EventReader<SeaEvent>,
    camera: Query<&Transform, With<SpatialListener>>,
) {
    let Some(sounds) = sounds else { return };
    let listener = camera
        .get_single()
        .map(|t| t.translation.truncate())
        .unwrap_or_default();
    let seed = time.elapsed_secs();
    for event in events.read() {
        match *event {
            SeaEvent::Salvo { at, own } => {
                let volume = if own { 1.0 } else { 0.8 };
                play_at(&mut commands, &sounds.cannon, at, listener, volume, seed);
            }
            SeaEvent::HullHit { at, own, .. } => {
                let volume = if own { 1.0 } else { 0.75 };
                play_at(&mut commands, &sounds.hull_hit, at, listener, volume, seed);
            }
            SeaEvent::Splash { at } => {
                play_at(&mut commands, &sounds.splash, at, listener, 0.55, seed);
            }
            SeaEvent::Sunk { at, .. } => {
                play_at(&mut commands, &sounds.sinking, at, listener, 1.0, seed);
            }
        }
    }
}

/// Vento nas velas e rangido do casco crescem com o seguimento.
#[allow(clippy::too_many_arguments)]
fn ship_ambience(
    mut commands: Commands,
    time: Res<Time>,
    sounds: Option<Res<SoundHandles>>,
    my_ship: Res<MyShip>,
    docked: Res<MyDocked>,
    ships: Query<&ShipVisual>,
    wind: Query<&AudioSink, With<WindLoop>>,
    mut next_creak: Local<f32>,
) {
    let speed_ratio = if docked.0 {
        0.0
    } else {
        ships
            .iter()
            .find(|v| Some(v.target.ship_id) == my_ship.0)
            .map(|v| (v.target.speed / v.target.max_speed.max(1.0)).clamp(0.0, 1.0))
            .unwrap_or(0.0)
    };
    if let Ok(sink) = wind.get_single() {
        // Aproxima devagar: sem degrau audível quando a vela muda.
        let target = WIND_VOLUME * MASTER_VOLUME * speed_ratio;
        let k = 1.0 - (-2.0 * time.delta_secs()).exp();
        sink.set_volume(sink.volume() + (target - sink.volume()) * k);
    }
    let now = time.elapsed_secs();
    if speed_ratio > 0.1 && now >= *next_creak {
        if let Some(sounds) = sounds {
            play_ui(&mut commands, &sounds.creak, 0.25 + 0.35 * speed_ratio);
        }
        *next_creak = now + 3.0 + 4.0 * jitter(now);
    }
}

fn ui_sounds(
    mut commands: Commands,
    sounds: Option<Res<SoundHandles>>,
    buttons: Query<&Interaction, (Changed<Interaction>, With<UiButton>)>,
    market: Res<MarketFeedback>,
    craft: Res<crate::port_screen::CraftFeedback>,
    zone: Res<CurrentZone>,
    mut last_tier: Local<Option<RiskTier>>,
) {
    let Some(sounds) = sounds else { return };
    if buttons.iter().any(|i| *i == Interaction::Pressed) {
        play_ui(&mut commands, &sounds.click, 0.6);
    }
    // Troca, depósito ou oferta aceita pelo servidor: som de negócio fechado.
    if market.is_changed() && market.0.as_ref().is_some_and(|result| result.success) {
        play_ui(&mut commands, &sounds.coins, 0.8);
    }
    // v22: peça com afixo — moedas no Mágico, sino e moedas no Raro.
    if craft.is_changed() {
        match craft.0.as_ref().and_then(|result| result.quality.as_ref()) {
            Some(quality) if quality.rarity == marvyr_domain_items::Rarity::Rare => {
                play_ui(&mut commands, &sounds.bell, 0.8);
                play_ui(&mut commands, &sounds.coins, 0.9);
            }
            Some(_) => play_ui(&mut commands, &sounds.coins, 0.9),
            None => {}
        }
    }
    let tier = zone.0.as_ref().map(|z| z.tier);
    let entering_pvp =
        tier.is_some_and(RiskTier::is_pvp) && !last_tier.is_some_and(RiskTier::is_pvp);
    if entering_pvp {
        play_ui(&mut commands, &sounds.bell, 0.7);
    }
    *last_tier = tier;
}

fn toggle_music(
    keys: Res<ButtonInput<KeyCode>>,
    docked: Res<MyDocked>,
    mut enabled: ResMut<MusicEnabled>,
    music: Query<&AudioSink, With<Music>>,
) {
    // Atracado há campos de texto no mercado: O é letra, não atalho.
    if docked.0 || !keys.just_pressed(KeyCode::KeyO) {
        return;
    }
    enabled.0 = !enabled.0;
    if let Ok(sink) = music.get_single() {
        sink.set_volume(if enabled.0 {
            MUSIC_VOLUME * MASTER_VOLUME
        } else {
            0.0
        });
    }
    info!(music = enabled.0, "música");
}

pub struct SoundPlugin;

impl Plugin for SoundPlugin {
    fn build(&self, app: &mut App) {
        app.insert_resource(MusicEnabled(true))
            .add_systems(Startup, setup_audio)
            .add_systems(
                Update,
                (
                    play_sea_events,
                    ship_ambience,
                    ui_sounds,
                    gem_sounds,
                    flask_sounds,
                    toggle_music,
                ),
            );
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn volume_falls_linearly_to_silence() {
        assert_eq!(distance_gain(0.0), 1.0);
        assert!((distance_gain(HEARING_RANGE / 2.0) - 0.5).abs() < 1e-6);
        assert_eq!(distance_gain(HEARING_RANGE), 0.0);
        assert_eq!(distance_gain(HEARING_RANGE * 3.0), 0.0);
    }

    #[test]
    fn jitter_stays_in_unit_range() {
        for i in 0..500 {
            let r = jitter(i as f32 * 0.731);
            assert!((0.0..1.0).contains(&r));
        }
    }
}
