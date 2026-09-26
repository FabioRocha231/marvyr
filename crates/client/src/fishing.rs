//! v39: pesca no client. Espaço manda o `CastLine`; o servidor diz quando
//! a linha está na água, quando o peixe morde e o que veio. Aqui só a boia:
//! flutua ao lado do casco, afunda e espirra na mordida, some no fim.

use bevy::prelude::*;
use lightyear::prelude::client::*;
use lightyear::prelude::ClientReceiveMessage;
use marvyr_protocol::{ActionKind, ActionResult, CastLine};

use crate::assets::layers;
use crate::camera::CameraShake;
use crate::logbook::my_position;
use crate::net::{MyDocked, MyShip, ReliableChannel};
use crate::ship::ShipVisual;

/// Distância da boia ao casco (m).
const BOBBER_OFFSET: f32 = 34.0;
const BOBBER_RED: Color = Color::srgb(0.9, 0.2, 0.15);
const SPLASH: Color = Color::srgb(0.85, 0.95, 1.0);

#[derive(Component)]
struct Bobber {
    bitten: bool,
    age: f32,
}

pub struct FishingPlugin;

impl Plugin for FishingPlugin {
    fn build(&self, app: &mut App) {
        app.add_systems(Update, (send_cast, follow_results, bob));
    }
}

/// Dev: `MARVYR_AUTOFISH=1` lança sozinho e puxa na mordida.
fn autofish() -> bool {
    std::env::var_os("MARVYR_AUTOFISH").is_some()
}

fn send_cast(
    time: Res<Time>,
    keys: Res<ButtonInput<KeyCode>>,
    docked: Res<MyDocked>,
    bobbers: Query<&Bobber>,
    mut auto_clock: Local<f32>,
    mut connection_manager: ResMut<ConnectionManager>,
) {
    if docked.0 {
        return;
    }
    let mut cast = keys.just_pressed(KeyCode::Space);
    if autofish() {
        *auto_clock += time.delta_secs();
        let bitten = bobbers.iter().any(|b| b.bitten && b.age > 0.3);
        if bitten || (bobbers.is_empty() && *auto_clock > 4.0) {
            *auto_clock = 0.0;
            cast = true;
        }
    }
    if cast {
        let _ = connection_manager.send_message::<ReliableChannel, _>(&CastLine);
    }
}

fn splash(commands: &mut Commands, at: Vec2, count: usize) {
    for i in 0..count {
        let angle = i as f32 / count as f32 * std::f32::consts::TAU;
        crate::vfx::spawn_particle(
            commands,
            at,
            crate::vfx::Particle {
                velocity: Vec2::from_angle(angle) * 26.0,
                drag: 4.0,
                life: 0.5,
                age: 0.0,
                size: (2.5, 0.5),
                color: SPLASH,
                z: layers::VFX,
            },
        );
    }
}

#[allow(clippy::too_many_arguments)]
fn follow_results(
    mut commands: Commands,
    mut results: EventReader<ClientReceiveMessage<ActionResult>>,
    my_ship: Res<MyShip>,
    visuals: Query<&ShipVisual>,
    mut bobbers: Query<(Entity, &mut Bobber, &Transform)>,
    mut shake: ResMut<CameraShake>,
) {
    for result in results.read() {
        let result = result.message();
        match (result.action, result.success) {
            (ActionKind::FishCast, true) => {
                let Some(me) = visuals
                    .iter()
                    .find(|visual| Some(visual.target.ship_id) == my_ship.0)
                else {
                    continue;
                };
                for (entity, _, _) in &bobbers {
                    commands.entity(entity).despawn_recursive();
                }
                // Boia ao lado do casco (boreste), longe da proa.
                let side = me.target.heading - std::f32::consts::FRAC_PI_2;
                let at =
                    Vec2::new(me.target.x, me.target.y) + Vec2::from_angle(side) * BOBBER_OFFSET;
                splash(&mut commands, at, 8);
                commands.spawn((
                    Bobber {
                        bitten: false,
                        age: 0.0,
                    },
                    Sprite::from_color(BOBBER_RED, Vec2::splat(6.0)),
                    Transform::from_translation(at.extend(layers::VFX)),
                ));
            }
            (ActionKind::FishBite, true) => {
                for (_, mut bobber, transform) in &mut bobbers {
                    bobber.bitten = true;
                    bobber.age = 0.0;
                    let at = transform.translation.truncate();
                    splash(&mut commands, at, 14);
                    crate::juice::spawn_float_text(
                        &mut commands,
                        at,
                        String::from("!"),
                        BOBBER_RED,
                    );
                    shake.add(0.12);
                }
            }
            (ActionKind::Fish, success) => {
                for (entity, _, _) in &bobbers {
                    commands.entity(entity).despawn_recursive();
                }
                if success {
                    if let Some(at) = my_position(&my_ship, &visuals) {
                        crate::juice::celebrate_burst(
                            &mut commands,
                            &mut shake,
                            at,
                            crate::i18n::tr("FISGOU!"),
                            (Color::srgb(0.55, 0.85, 1.0), Color::srgb(1.0, 0.85, 0.35)),
                        );
                    }
                }
            }
            _ => {}
        }
    }
}

/// A boia balança na água; mordida, mergulha e pisca.
fn bob(time: Res<Time>, mut bobbers: Query<(&mut Bobber, &mut Transform, &mut Sprite)>) {
    let dt = time.delta_secs();
    for (mut bobber, mut transform, mut sprite) in &mut bobbers {
        bobber.age += dt;
        let (speed, depth) = if bobber.bitten {
            (18.0, 0.55)
        } else {
            (3.0, 0.15)
        };
        let wave = (bobber.age * speed).sin();
        transform.scale = Vec3::splat(1.0 - depth * 0.5 + depth * wave * 0.5);
        sprite.color = if bobber.bitten && wave > 0.0 {
            Color::WHITE
        } else {
            BOBBER_RED
        };
    }
}
