//! Tempestades e munição no client (MF-059). O servidor decide pano e
//! munição; aqui só se desenha: aviso de tempestade + velas + munição com o
//! que ela faz (topo-direita), riscos de vento no mar (só ambiente — o vento
//! não mexe mais na navegação) e tempestades (mar escurecido, nuvens e
//! chuva). Tecla C pede a troca de munição.

use std::f32::consts::{PI, TAU};

use bevy::prelude::*;
use lightyear::prelude::client::*;
use lightyear::prelude::*;
use marvyr_domain_combat::Ammo;
use marvyr_domain_ships::SAIL_HP_MAX;
use marvyr_protocol::{SelectAmmo, StormState, WeatherUpdate};

use crate::assets::layers;
use crate::hud::SeaHud;
use crate::i18n::{tr, trf};
use crate::net::{MyDocked, MyShip, ReliableChannel};
use crate::ship::ShipVisual;
use crate::ui;

/// Último clima anunciado pelo servidor.
#[derive(Resource, Debug, Clone, Default)]
pub struct SeaWeather {
    pub wind_dir: f32,
    pub wind_strength: f32,
    pub storms: Vec<StormState>,
    pub known: bool,
}

impl SeaWeather {
    /// O ponto está dentro de alguma tempestade viva?
    pub fn in_storm(&self, at: Vec2) -> bool {
        self.storms
            .iter()
            .any(|s| s.intensity > 0.05 && at.distance(Vec2::new(s.x, s.y)) < s.radius)
    }
}

pub struct WeatherPlugin;

impl Plugin for WeatherPlugin {
    fn build(&self, app: &mut App) {
        app.init_resource::<SeaWeather>()
            .add_systems(Startup, (setup_weather_hud, setup_storm_assets))
            .add_systems(
                Update,
                (
                    receive_weather,
                    send_ammo_input,
                    update_weather_hud,
                    sync_storm_visuals,
                    animate_storms,
                    spawn_streaks,
                    update_streaks,
                ),
            );
    }
}

fn receive_weather(
    mut events: EventReader<ClientReceiveMessage<WeatherUpdate>>,
    mut weather: ResMut<SeaWeather>,
) {
    if let Some(event) = events.read().last() {
        let update = event.message();
        weather.wind_dir = update.wind_dir;
        weather.wind_strength = update.wind_strength;
        weather.storms = update.storms.clone();
        weather.known = true;
    }
}

/// C alterna bala/corrente. Pede ao servidor a PRÓXIMA da munição que o
/// servidor diz estar carregada. Dev: MARVYR_AMMO=chain pede corrente.
fn send_ammo_input(
    keys: Res<ButtonInput<KeyCode>>,
    docked: Res<MyDocked>,
    my_ship: Res<MyShip>,
    visuals: Query<&ShipVisual>,
    mut dev_sent: Local<bool>,
    mut connection_manager: ResMut<ConnectionManager>,
) {
    let Some(current) = my_state(&my_ship, &visuals).map(|s| s.ammo) else {
        return;
    };
    let dev_chain =
        !*dev_sent && std::env::var("MARVYR_AMMO").is_ok_and(|v| v.eq_ignore_ascii_case("chain"));
    let ammo = if keys.just_pressed(KeyCode::KeyC) && !docked.0 {
        current.next()
    } else if dev_chain {
        *dev_sent = true;
        Ammo::Chain
    } else {
        return;
    };
    info!(?ammo, "pedindo troca de municao");
    let _ = connection_manager.send_message::<ReliableChannel, _>(&SelectAmmo { ammo });
}

fn my_state<'a>(
    my_ship: &MyShip,
    visuals: &'a Query<&ShipVisual>,
) -> Option<&'a marvyr_protocol::ShipState> {
    let id = my_ship.0?;
    visuals
        .iter()
        .find(|v| v.target.ship_id == id)
        .map(|v| &v.target)
}

// ---------------------------------------------------------------- HUD

#[derive(Component, Debug, Clone, Copy, PartialEq, Eq)]
enum WeatherText {
    Storm,
    Sails,
    Ammo,
    AmmoHint,
}

#[derive(Component)]
struct SailFill;

fn setup_weather_hud(mut commands: Commands) {
    commands
        .spawn((
            ui::panel(Node {
                position_type: PositionType::Absolute,
                right: Val::Px(ui::MARGIN),
                top: Val::Px(ui::MARGIN),
                flex_direction: FlexDirection::Column,
                row_gap: Val::Px(6.0),
                min_width: Val::Px(190.0),
                ..default()
            }),
            SeaHud,
        ))
        .with_children(|panel| {
            panel.spawn((ui::text("", 13.0, ui::DANGER), WeatherText::Storm));
            panel
                .spawn(Node {
                    align_items: AlignItems::Center,
                    column_gap: Val::Px(8.0),
                    ..default()
                })
                .with_children(|row| {
                    row.spawn((
                        ui::text(crate::i18n::trf("VELAS {0}%", &["100"]), 11.0, ui::TEXT_DIM),
                        Node {
                            width: Val::Px(78.0),
                            ..default()
                        },
                        WeatherText::Sails,
                    ));
                    ui::spawn_bar(row, 96.0, ui::OK_GREEN, SailFill);
                });
            panel.spawn((ui::text("-", 12.0, ui::GOLD), WeatherText::Ammo));
            panel.spawn((ui::text("-", 10.0, ui::TEXT_DIM), WeatherText::AmmoHint));
        });
}

pub fn ammo_label(ammo: Ammo) -> &'static str {
    match ammo {
        Ammo::Round => "MUNIÇÃO: BALA",
        Ammo::Chain => "MUNIÇÃO: CORRENTE",
    }
}

/// O que a munição faz, em uma linha (espelho de `domain-combat::Ammo`).
pub fn ammo_hint(ammo: Ammo) -> &'static str {
    match ammo {
        Ammo::Round => "Dano cheio no casco, alcance longo",
        Ammo::Chain => "Rasga velas (inimigo fica lento), pouco casco, alcance curto",
    }
}

fn update_weather_hud(
    weather: Res<SeaWeather>,
    my_ship: Res<MyShip>,
    visuals: Query<&ShipVisual>,
    mut texts: Query<(&mut Text, &mut TextColor, &WeatherText)>,
    mut fill: Query<(&mut Node, &mut BackgroundColor), With<SailFill>>,
) {
    let state = my_state(&my_ship, &visuals);
    let storm = state.is_some_and(|s| weather.in_storm(Vec2::new(s.x, s.y)));
    for (mut text, mut color, kind) in &mut texts {
        let (value, tint) = match kind {
            WeatherText::Storm if storm => (tr("TEMPESTADE!"), ui::DANGER),
            WeatherText::Storm => (String::new(), ui::DANGER),
            WeatherText::Sails => {
                let pct = state.map_or(100.0, |s| s.sail_hp / SAIL_HP_MAX * 100.0);
                (trf("VELAS {0}%", &[&format!("{pct:.0}")]), ui::TEXT_DIM)
            }
            WeatherText::Ammo => {
                let ammo = state.map_or(Ammo::Round, |s| s.ammo);
                let tint = if ammo == Ammo::Chain {
                    ui::AMBER
                } else {
                    ui::GOLD
                };
                (format!("{} [C]", tr(ammo_label(ammo))), tint)
            }
            WeatherText::AmmoHint => {
                let ammo = state.map_or(Ammo::Round, |s| s.ammo);
                (tr(ammo_hint(ammo)), ui::TEXT_DIM)
            }
        };
        if text.0 != value {
            text.0 = value;
        }
        if color.0 != tint {
            color.0 = tint;
        }
    }
    let sail = state.map_or(1.0, |s| s.sail_hp / SAIL_HP_MAX);
    for (mut node, mut bg) in &mut fill {
        crate::hud::set_width(&mut node, ui::bar_width(sail));
        bg.set_if_neq(BackgroundColor(if sail > 0.6 {
            ui::OK_GREEN
        } else if sail > 0.3 {
            ui::AMBER
        } else {
            ui::DANGER
        }));
    }
}

// ------------------------------------------------------------ Tempestades

#[derive(Resource)]
struct StormAssets {
    disc: Handle<Mesh>,
    cloud: Handle<Mesh>,
}

#[derive(Component)]
struct StormVisual {
    id: u32,
    target: Vec2,
    radius: f32,
    shown: f32,
    goal: f32,
    sea: Handle<ColorMaterial>,
    clouds: Handle<ColorMaterial>,
}

#[derive(Component)]
struct StormCloud {
    anchor: Vec2,
    phase: f32,
}

const SEA_RINGS: usize = 5;
const CLOUDS: usize = 14;
const STORM_SEA_ALPHA: f32 = 0.12;
const STORM_CLOUD_ALPHA: f32 = 0.36;

fn setup_storm_assets(mut commands: Commands, mut meshes: ResMut<Assets<Mesh>>) {
    commands.insert_resource(StormAssets {
        disc: meshes.add(Circle::new(1.0).mesh().resolution(48)),
        // Poucos lados: nuvem "pixelada", no estilo dos sprites.
        cloud: meshes.add(Circle::new(1.0).mesh().resolution(9)),
    });
}

fn sync_storm_visuals(
    mut commands: Commands,
    weather: Res<SeaWeather>,
    assets: Option<Res<StormAssets>>,
    mut materials: ResMut<Assets<ColorMaterial>>,
    mut visuals: Query<&mut StormVisual>,
) {
    if !weather.is_changed() {
        return;
    }
    let Some(assets) = assets else { return };
    for mut visual in &mut visuals {
        match weather.storms.iter().find(|s| s.storm_id == visual.id) {
            Some(storm) => {
                visual.target = Vec2::new(storm.x, storm.y);
                visual.goal = storm.intensity;
            }
            None => visual.goal = 0.0,
        }
    }
    for storm in &weather.storms {
        if visuals.iter().any(|v| v.id == storm.storm_id) {
            continue;
        }
        info!(
            storm_id = storm.storm_id,
            x = storm.x,
            y = storm.y,
            "tempestade a vista"
        );
        spawn_storm(&mut commands, &assets, &mut materials, storm);
    }
}

fn spawn_storm(
    commands: &mut Commands,
    assets: &StormAssets,
    materials: &mut Assets<ColorMaterial>,
    storm: &StormState,
) {
    let sea = materials.add(Color::srgba(0.02, 0.04, 0.08, 0.0));
    let clouds = materials.add(Color::srgba(0.16, 0.18, 0.22, 0.0));
    let at = Vec2::new(storm.x, storm.y);
    commands
        .spawn((
            Transform::from_translation(at.extend(0.0)),
            Visibility::default(),
            StormVisual {
                id: storm.storm_id,
                target: at,
                radius: storm.radius,
                shown: 0.0,
                goal: storm.intensity,
                sea: sea.clone(),
                clouds: clouds.clone(),
            },
        ))
        .with_children(|root| {
            // Anéis empilhados: o centro fica mais escuro, a borda é suave.
            for i in 0..SEA_RINGS {
                let r = storm.radius * (0.6 + 0.1 * i as f32);
                root.spawn((
                    Mesh2d(assets.disc.clone()),
                    MeshMaterial2d(sea.clone()),
                    Transform::from_xyz(0.0, 0.0, layers::OCEAN + 0.3 + 0.01 * i as f32)
                        .with_scale(Vec3::new(r, r, 1.0)),
                ));
            }
            for i in 0..CLOUDS {
                let a = i as f32 / CLOUDS as f32 * TAU + i as f32 * 0.7;
                let d = storm.radius * (0.15 + 0.55 * ((i * 7 % 5) as f32 / 4.0));
                let size = storm.radius * (0.22 + 0.06 * (i % 3) as f32);
                root.spawn((
                    Mesh2d(assets.cloud.clone()),
                    MeshMaterial2d(clouds.clone()),
                    Transform::from_xyz(0.0, 0.0, layers::VFX + 1.0 + 0.01 * i as f32)
                        .with_scale(Vec3::new(size * 1.3, size, 1.0)),
                    StormCloud {
                        anchor: Vec2::from_angle(a) * d,
                        phase: a,
                    },
                ));
            }
        });
}

fn animate_storms(
    mut commands: Commands,
    time: Res<Time>,
    mut materials: ResMut<Assets<ColorMaterial>>,
    mut storms: Query<(Entity, &mut StormVisual, &mut Transform, &Children), Without<StormCloud>>,
    mut clouds: Query<(&StormCloud, &mut Transform), Without<StormVisual>>,
) {
    let dt = time.delta_secs();
    let t = time.elapsed_secs();
    for (entity, mut storm, mut transform, children) in &mut storms {
        // O servidor anuncia a 1 Hz: persegue suave, sem teleporte.
        let pos = transform.translation.truncate();
        let next = pos.lerp(storm.target, 1.0 - (-1.5 * dt).exp());
        transform.translation = next.extend(0.0);
        let step = dt / 2.0;
        storm.shown += (storm.goal - storm.shown).clamp(-step, step);
        if storm.goal <= 0.0 && storm.shown <= 0.0 {
            commands.entity(entity).despawn_recursive();
            continue;
        }
        let k = storm.shown;
        // get_mut marca o material como modificado e o Bevy refaz o bind
        // group: só escreve quando a cor muda de fato.
        let sea_alpha = STORM_SEA_ALPHA * k;
        if materials
            .get(&storm.sea)
            .is_some_and(|m| m.color.alpha() != sea_alpha)
        {
            if let Some(sea) = materials.get_mut(&storm.sea) {
                sea.color.set_alpha(sea_alpha);
            }
        }
        // Relâmpago: um clarão raro e curto nas nuvens.
        let flash = ((t * 0.37 + storm.id as f32 * 1.9).sin() > 0.995) as u8 as f32;
        let base = Color::srgb(0.16, 0.18, 0.22).mix(&Color::srgb(0.85, 0.88, 0.95), flash);
        let cloud_color = base.with_alpha(STORM_CLOUD_ALPHA * k);
        if materials
            .get(&storm.clouds)
            .is_some_and(|m| m.color != cloud_color)
        {
            if let Some(cloud) = materials.get_mut(&storm.clouds) {
                cloud.color = cloud_color;
            }
        }
        for child in children.iter() {
            if let Ok((cloud, mut ct)) = clouds.get_mut(*child) {
                let wobble = Vec2::new(
                    (t * 0.11 + cloud.phase).sin(),
                    (t * 0.07 + cloud.phase * 1.3).cos(),
                ) * storm.radius
                    * 0.06;
                ct.translation.x = cloud.anchor.x + wobble.x;
                ct.translation.y = cloud.anchor.y + wobble.y;
            }
        }
    }
}

// ------------------------------------------------- Riscos de vento e chuva

/// Risco fino que corre com o vento (ou gota de chuva), estilo pixel.
#[derive(Component)]
struct Streak {
    velocity: Vec2,
    age: f32,
    life: f32,
    alpha: f32,
}

/// Pseudo-aleatório barato em [0, 1) (xorshift num `Local`).
fn rand01(state: &mut u32) -> f32 {
    if *state == 0 {
        *state = 0x9E37_79B9;
    }
    *state ^= *state << 13;
    *state ^= *state >> 17;
    *state ^= *state << 5;
    (*state >> 8) as f32 / (1u32 << 24) as f32
}

const WIND_STREAKS_PER_SEC: f32 = 7.0;
const RAIN_PER_SEC: f32 = 180.0;

#[allow(clippy::too_many_arguments)]
fn spawn_streaks(
    mut commands: Commands,
    time: Res<Time>,
    weather: Res<SeaWeather>,
    docked: Res<MyDocked>,
    camera: Query<(&Transform, &OrthographicProjection), With<Camera2d>>,
    mut rng: Local<u32>,
    mut wind_acc: Local<f32>,
    mut rain_acc: Local<f32>,
) {
    if !weather.known || docked.0 {
        return;
    }
    let Ok((cam, ortho)) = camera.get_single() else {
        return;
    };
    let dt = time.delta_secs();
    let center = cam.translation.truncate();
    let area = ortho.area;
    let dir = Vec2::from_angle(weather.wind_dir);
    let random_point = |rng: &mut u32| {
        center
            + Vec2::new(
                area.min.x + rand01(rng) * area.width(),
                area.min.y + rand01(rng) * area.height(),
            )
    };

    *wind_acc += dt * WIND_STREAKS_PER_SEC * (0.4 + weather.wind_strength);
    while *wind_acc >= 1.0 {
        *wind_acc -= 1.0;
        let at = random_point(&mut rng);
        let len = 8.0 + 10.0 * rand01(&mut rng);
        commands.spawn((
            Sprite::from_color(Color::srgba(0.92, 0.97, 1.0, 1.0), Vec2::new(len, 1.0)),
            Transform::from_translation(at.extend(layers::WAKE + 0.1))
                .with_rotation(Quat::from_rotation_z(weather.wind_dir)),
            Streak {
                velocity: dir * (10.0 + 18.0 * weather.wind_strength),
                age: 0.0,
                life: 2.2 + rand01(&mut rng),
                alpha: 0.22,
            },
        ));
    }

    // Chuva só onde a tempestade cobre a vista.
    let view = Rect::from_center_size(center, area.size());
    for storm in &weather.storms {
        let s_center = Vec2::new(storm.x, storm.y);
        let nearest = s_center.clamp(view.min, view.max);
        if nearest.distance(s_center) > storm.radius || storm.intensity <= 0.0 {
            continue;
        }
        *rain_acc += dt * RAIN_PER_SEC * storm.intensity;
        let slant = weather.wind_dir + PI * 0.15;
        while *rain_acc >= 1.0 {
            *rain_acc -= 1.0;
            let at = random_point(&mut rng);
            if at.distance(s_center) > storm.radius * 0.95 {
                continue;
            }
            commands.spawn((
                Sprite::from_color(Color::srgba(0.72, 0.80, 0.92, 1.0), Vec2::new(5.0, 1.0)),
                Transform::from_translation(at.extend(layers::VFX + 2.0))
                    .with_rotation(Quat::from_rotation_z(slant)),
                Streak {
                    velocity: Vec2::from_angle(slant) * 60.0,
                    age: 0.0,
                    life: 0.35,
                    alpha: 0.8,
                },
            ));
        }
    }
}

fn update_streaks(
    mut commands: Commands,
    time: Res<Time>,
    mut streaks: Query<(Entity, &mut Streak, &mut Transform, &mut Sprite)>,
) {
    let dt = time.delta_secs();
    for (entity, mut streak, mut transform, mut sprite) in &mut streaks {
        streak.age += dt;
        if streak.age >= streak.life {
            commands.entity(entity).despawn();
            continue;
        }
        transform.translation += (streak.velocity * dt).extend(0.0);
        // Aparece e some suave: meia senoide ao longo da vida.
        let k = (streak.age / streak.life * PI).sin();
        sprite.color.set_alpha(streak.alpha * k);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn labels_keep_accents_and_translate() {
        use crate::i18n::{translate, Lang};
        assert_eq!(ammo_label(Ammo::Chain), "MUNIÇÃO: CORRENTE");
        let all = [
            ammo_label(Ammo::Round),
            ammo_label(Ammo::Chain),
            ammo_hint(Ammo::Round),
            ammo_hint(Ammo::Chain),
            "TEMPESTADE!",
        ];
        assert!(all.iter().all(|s| translate(s, Lang::En) != *s));
    }

    #[test]
    fn in_storm_respects_radius_and_intensity() {
        let weather = SeaWeather {
            storms: vec![StormState {
                storm_id: 1,
                x: 0.0,
                y: 0.0,
                radius: 300.0,
                intensity: 1.0,
                tempest: false,
            }],
            known: true,
            ..default()
        };
        assert!(weather.in_storm(Vec2::new(100.0, 0.0)));
        assert!(!weather.in_storm(Vec2::new(400.0, 0.0)));
    }
}
