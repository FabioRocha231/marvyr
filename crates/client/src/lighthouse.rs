//! v46: faróis no client. O servidor manda a lista (`LighthousesUpdate`);
//! aqui a torre vista de cima, o facho girando e o halo que clareia a
//! noite. B no bilhete manda o `RaiseLighthouse` — erguer ou reforçar é o
//! servidor quem decide e cobra.

use bevy::prelude::*;
use lightyear::prelude::client::*;
use lightyear::prelude::ClientReceiveMessage;
use marvyr_protocol::lighthouse::{COAST, LIGHT, SPACING, TEND};
use marvyr_protocol::{
    ActionKind, ActionResult, LighthouseLine, LighthousesUpdate, RaiseLighthouse,
};

use crate::assets::layers;
use crate::camera::CameraShake;
use crate::logbook::my_position;
use crate::net::{MyDocked, MyShip, ReliableChannel};
use crate::ship::ShipVisual;

const STONE: Color = Color::srgb(0.42, 0.40, 0.38);
const WHITEWASH: Color = Color::srgb(0.95, 0.93, 0.86);
const LAMP: Color = Color::srgb(1.0, 0.86, 0.42);
/// Luz do halo: creme claro (amarelo puro sobre o azul do mar vira cinza).
const GLOW: Color = Color::srgb(1.0, 0.97, 0.82);
/// Anéis do halo, de fora para dentro: a transparência soma e o centro
/// clareia mais — degradê sem textura.
const HALO_RINGS: [f32; 4] = [1.0, 0.72, 0.46, 0.24];
/// Comprimento do facho (m).
const BEAM: f32 = 220.0;
/// Voltas do facho por segundo (rad/s).
const SPIN: f32 = 0.9;

#[derive(Resource, Default)]
pub struct KnownLighthouses(pub Vec<LighthouseLine>);

/// O que o B faria aqui (para o bilhete do HUD): reforçar, erguer, nada.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LighthouseAction {
    Tend,
    Raise,
}

/// Mesma geometria do servidor, só para decidir se o bilhete aparece; a
/// palavra final (material, teto de faróis) é do servidor.
pub fn action_at(
    known: &KnownLighthouses,
    map: &marvyr_domain_world::WorldMap,
    at: Vec2,
) -> Option<LighthouseAction> {
    let near = |radius: f32| {
        known
            .0
            .iter()
            .any(|l| Vec2::new(l.x, l.y).distance(at) <= radius)
    };
    if near(TEND) {
        return Some(LighthouseAction::Tend);
    }
    let coastal = map
        .land()
        .iter()
        .any(|mass| mass.contains(at.x, at.y, COAST));
    (coastal && !near(SPACING)).then_some(LighthouseAction::Raise)
}

#[derive(Component)]
struct LighthouseVisual(u32);

#[derive(Component)]
struct Beam;

#[derive(Component)]
struct Halo;

#[derive(Resource)]
struct LighthouseAssets {
    disc: Handle<Mesh>,
}

pub struct LighthousePlugin;

impl Plugin for LighthousePlugin {
    fn build(&self, app: &mut App) {
        app.init_resource::<KnownLighthouses>()
            .add_systems(Startup, setup_assets)
            .add_systems(
                Update,
                (receive, sync_visuals, spin, send_raise, celebrate).chain(),
            );
    }
}

fn setup_assets(mut commands: Commands, mut meshes: ResMut<Assets<Mesh>>) {
    commands.insert_resource(LighthouseAssets {
        disc: meshes.add(Circle::new(1.0).mesh().resolution(40)),
    });
}

fn receive(
    mut updates: EventReader<ClientReceiveMessage<LighthousesUpdate>>,
    mut known: ResMut<KnownLighthouses>,
) {
    if let Some(update) = updates.read().last() {
        known.0 = update.message().list.clone();
    }
}

fn sync_visuals(
    mut commands: Commands,
    known: Res<KnownLighthouses>,
    assets: Option<Res<LighthouseAssets>>,
    mut materials: ResMut<Assets<ColorMaterial>>,
    visuals: Query<(Entity, &LighthouseVisual)>,
    mut labels: Query<(&mut Text2d, &Parent)>,
) {
    if !known.is_changed() {
        return;
    }
    let Some(assets) = assets else {
        return;
    };
    for (entity, visual) in &visuals {
        if !known.0.iter().any(|l| l.id == visual.0) {
            commands.entity(entity).despawn_recursive();
        }
    }
    for line in &known.0 {
        let label = crate::i18n::trf("Farol · {0}h", &[&line.hours_left.to_string()]);
        if let Some((entity, _)) = visuals.iter().find(|(_, v)| v.0 == line.id) {
            for (mut text, parent) in &mut labels {
                if parent.get() == entity && text.0 != label {
                    text.0 = label.clone();
                }
            }
            continue;
        }
        let disc = |radius: f32, color: Color, z: f32, materials: &mut Assets<ColorMaterial>| {
            (
                Mesh2d(assets.disc.clone()),
                MeshMaterial2d(materials.add(color)),
                Transform::from_xyz(0.0, 0.0, z).with_scale(Vec3::splat(radius)),
            )
        };
        commands
            .spawn((
                LighthouseVisual(line.id),
                Transform::from_xyz(line.x, line.y, layers::PROPS),
                Visibility::default(),
            ))
            .with_children(|tower| {
                // Halo da luz: o raio em que a noite clareia.
                for (index, ring) in HALO_RINGS.iter().enumerate() {
                    tower.spawn((
                        Halo,
                        disc(
                            LIGHT * ring,
                            GLOW.with_alpha(0.0),
                            -0.5 + index as f32 * 0.01,
                            &mut materials,
                        ),
                    ));
                }
                tower.spawn(disc(16.0, STONE, 0.1, &mut materials));
                tower.spawn(disc(10.0, WHITEWASH, 0.2, &mut materials));
                tower.spawn(disc(5.0, LAMP, 0.3, &mut materials));
                tower.spawn((
                    Beam,
                    Sprite {
                        color: LAMP.with_alpha(0.28),
                        custom_size: Some(Vec2::new(14.0, BEAM)),
                        anchor: bevy::sprite::Anchor::BottomCenter,
                        ..default()
                    },
                    Transform::from_xyz(0.0, 0.0, 0.25),
                ));
                tower.spawn((
                    Text2d::new(label),
                    TextFont {
                        font_size: 13.0,
                        ..default()
                    },
                    TextColor(WHITEWASH),
                    Transform::from_xyz(0.0, -30.0, layers::LABELS - layers::PROPS),
                ));
            });
    }
}

/// O facho gira; o halo acende com a noite.
fn spin(
    time: Res<Time>,
    night: Res<crate::weather::NightLevel>,
    mut beams: Query<&mut Transform, With<Beam>>,
    halos: Query<&MeshMaterial2d<ColorMaterial>, With<Halo>>,
    mut materials: ResMut<Assets<ColorMaterial>>,
) {
    let angle = time.elapsed_secs() * SPIN;
    for mut transform in &mut beams {
        transform.rotation = Quat::from_rotation_z(angle);
    }
    // Cada anel soma; de dia quase some, à noite o centro clareia bem.
    let glow = GLOW.with_alpha(0.015 + 0.06 * night.0);
    for handle in &halos {
        // Só escreve quando muda: material mudado reenvia para a GPU.
        if materials.get(&handle.0).is_some_and(|m| m.color != glow) {
            if let Some(material) = materials.get_mut(&handle.0) {
                material.color = glow;
            }
        }
    }
}

fn send_raise(
    keys: Res<ButtonInput<KeyCode>>,
    docked: Res<MyDocked>,
    context: Res<crate::input::ContextKey>,
    mut connection_manager: ResMut<ConnectionManager>,
) {
    if docked.0 || context.0 != Some(KeyCode::KeyB) || !keys.just_pressed(KeyCode::KeyB) {
        return;
    }
    let _ = connection_manager.send_message::<ReliableChannel, _>(&RaiseLighthouse);
}

fn celebrate(
    mut commands: Commands,
    mut results: EventReader<ClientReceiveMessage<ActionResult>>,
    my_ship: Res<MyShip>,
    visuals: Query<&ShipVisual>,
    mut shake: ResMut<CameraShake>,
) {
    for result in results.read() {
        let result = result.message();
        if result.action != ActionKind::Lighthouse || !result.success {
            continue;
        }
        if let Some(at) = my_position(&my_ship, &visuals) {
            crate::juice::celebrate_burst(
                &mut commands,
                &mut shake,
                at,
                crate::i18n::tr("FAROL ACESO!"),
                (LAMP, WHITEWASH),
            );
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use marvyr_domain_world::WorldMap;

    #[test]
    fn prompt_offers_raise_on_the_coast_and_tend_next_to_one() {
        let map = WorldMap::from_seed(0);
        let mass = map.land().iter().find(|m| !m.cliff).expect("terra");
        let at = Vec2::new(mass.x + mass.radius + 40.0, mass.y);
        let mut known = KnownLighthouses::default();
        assert_eq!(action_at(&known, &map, at), Some(LighthouseAction::Raise));
        known.0.push(LighthouseLine {
            id: 1,
            x: at.x,
            y: at.y,
            hours_left: 72,
            builder: String::new(),
        });
        assert_eq!(action_at(&known, &map, at), Some(LighthouseAction::Tend));
        assert_eq!(action_at(&known, &map, at + Vec2::X * 200.0), None);
    }
}
