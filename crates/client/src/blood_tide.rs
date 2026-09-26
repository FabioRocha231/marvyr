//! v32: Maré Sangrenta no client — só desenha o que o servidor manda no
//! `SeaEventState` (a água tingida fica no shader do mar, `world.rs`):
//! brasas subindo, os Baús Malditos ainda fechados e a festa quando o seu
//! abre.

use bevy::prelude::*;
use lightyear::prelude::ClientReceiveMessage;
use marvyr_protocol::{ActionKind, ActionResult, SeaEventKind};

use crate::assets::{deco, layers, GameAssets};
use crate::camera::CameraShake;
use crate::net::MyShip;
use crate::seafaring::SeaEvents;
use crate::ship::ShipVisual;
use crate::vfx::{spawn_particle, Particle};

/// Vermelho da maré (anel, rótulo, festa).
pub const BLOOD: Color = Color::srgb(0.9, 0.15, 0.12);
/// Cinzas que o baú pede (espelha `blood_tide::ASH_PER_CHEST` do servidor;
/// só texto).
const ASH_PER_CHEST: u32 = 10;

/// Baú Maldito fechado, pela posição que o servidor mandou.
#[derive(Component)]
struct CursedChestMark {
    at: Vec2,
}

pub struct BloodTidePlugin;

impl Plugin for BloodTidePlugin {
    fn build(&self, app: &mut App) {
        app.add_systems(
            Update,
            (sync_tide, animate_chests, drift_embers, celebrate_chest),
        );
    }
}

fn tide(events: &SeaEvents) -> Option<&marvyr_protocol::SeaEventState> {
    events
        .0
        .iter()
        .find(|event| event.kind == SeaEventKind::BloodTide)
}

/// Sobe/baixa o tingido e mantém os baús iguais aos do servidor.
fn sync_tide(
    mut commands: Commands,
    events: Res<SeaEvents>,
    assets: Res<GameAssets>,
    chests: Query<(Entity, &CursedChestMark)>,
) {
    let current = tide(&events);
    let wanted: Vec<Vec2> = current
        .map(|event| {
            event
                .chests
                .iter()
                .map(|(x, y)| Vec2::new(*x, *y))
                .collect()
        })
        .unwrap_or_default();
    for (entity, chest) in &chests {
        if !wanted.iter().any(|at| at.distance(chest.at) < 1.0) {
            commands.entity(entity).despawn_recursive();
        }
    }
    for at in wanted {
        if chests.iter().any(|(_, chest)| chest.at.distance(at) < 1.0) {
            continue;
        }
        commands
            .spawn((
                CursedChestMark { at },
                Transform::from_translation(at.extend(layers::WRECKS)),
                Visibility::default(),
            ))
            .with_children(|parent| {
                let mut chest = Sprite::from_atlas_image(
                    assets.water_and_islands.clone(),
                    TextureAtlas {
                        layout: assets.deco.clone(),
                        index: deco::CHEST_GOLD,
                    },
                );
                chest.color = Color::srgb(1.0, 0.5, 0.45);
                parent.spawn((chest, Transform::from_scale(Vec3::splat(1.3))));
                parent.spawn((
                    Text2d::new(crate::i18n::trf(
                        "Baú Maldito · {0} Cinzas",
                        &[&ASH_PER_CHEST.to_string()],
                    )),
                    TextFont {
                        font_size: 13.0,
                        ..default()
                    },
                    TextColor(BLOOD),
                    Transform::from_xyz(0.0, 26.0, layers::LABELS - layers::WRECKS),
                ));
            });
    }
}

/// Baú pulsa e solta fagulhas vermelhas.
fn animate_chests(
    mut commands: Commands,
    time: Res<Time>,
    mut gizmos: Gizmos,
    mut chests: Query<(&CursedChestMark, &mut Transform)>,
    mut clock: Local<f32>,
) {
    let t = time.elapsed_secs();
    *clock += time.delta_secs();
    let spark = *clock >= 0.12;
    if spark {
        *clock = 0.0;
    }
    for (chest, mut transform) in &mut chests {
        let pulse = 0.5 + 0.5 * (t * 3.0 + chest.at.x * 0.01).sin();
        transform.scale = Vec3::splat(1.0 + 0.08 * pulse);
        gizmos.circle_2d(
            Isometry2d::from_translation(chest.at),
            30.0 + 6.0 * pulse,
            BLOOD.with_alpha(0.35 + 0.35 * pulse),
        );
        if spark {
            let angle = t * 7.3 + chest.at.y;
            spawn_particle(
                &mut commands,
                chest.at + Vec2::from_angle(angle) * 10.0,
                Particle {
                    velocity: Vec2::new(angle.cos() * 4.0, 16.0),
                    drag: 1.2,
                    life: 0.9,
                    age: 0.0,
                    size: (2.4, 0.8),
                    color: Color::srgb(1.0, 0.4, 0.2),
                    z: layers::VFX,
                },
            );
        }
    }
}

/// Brasas subindo por toda a área da maré.
fn drift_embers(
    mut commands: Commands,
    time: Res<Time>,
    events: Res<SeaEvents>,
    mut clock: Local<f32>,
    mut seed: Local<u32>,
) {
    let Some(event) = tide(&events) else {
        return;
    };
    *clock += time.delta_secs();
    // ~40 brasas por segundo, espalhadas pelo disco.
    while *clock >= 0.025 {
        *clock -= 0.025;
        *seed = seed.wrapping_add(1);
        let k = *seed as f32;
        let r = event.radius * (k * 0.618_034).fract().sqrt();
        let angle = k * 2.399_963;
        let at = Vec2::new(event.x, event.y) + Vec2::from_angle(angle) * r;
        let hot = *seed % 3 == 0;
        spawn_particle(
            &mut commands,
            at,
            Particle {
                velocity: Vec2::new((k * 1.7).sin() * 3.0, 10.0),
                drag: 0.4,
                life: 1.6,
                age: 0.0,
                size: (if hot { 3.4 } else { 2.6 }, 0.6),
                color: if hot {
                    Color::srgb(1.0, 0.55, 0.2)
                } else {
                    Color::srgb(0.85, 0.12, 0.1)
                },
                z: layers::VFX,
            },
        );
    }
}

/// Baú aberto (o servidor confirmou): letreiro, estouro e tremor.
fn celebrate_chest(
    mut commands: Commands,
    mut results: EventReader<ClientReceiveMessage<ActionResult>>,
    my_ship: Res<MyShip>,
    visuals: Query<&ShipVisual>,
    mut shake: ResMut<CameraShake>,
) {
    for result in results.read() {
        let result = result.message();
        if result.action != ActionKind::CursedChest || !result.success {
            continue;
        }
        let Some(me) = visuals
            .iter()
            .find(|visual| Some(visual.target.ship_id) == my_ship.0)
        else {
            continue;
        };
        let at = Vec2::new(me.target.x, me.target.y);
        crate::juice::spawn_float_text(
            &mut commands,
            at + Vec2::new(0.0, 30.0),
            crate::i18n::tr("BAÚ MALDITO!"),
            BLOOD,
        );
        for i in 0..28 {
            let angle = i as f32 / 28.0 * std::f32::consts::TAU;
            spawn_particle(
                &mut commands,
                at,
                Particle {
                    velocity: Vec2::from_angle(angle) * (40.0 + (i % 4) as f32 * 12.0),
                    drag: 3.0,
                    life: 0.9,
                    age: 0.0,
                    size: (3.0, 1.0),
                    color: if i % 2 == 0 {
                        BLOOD
                    } else {
                        Color::srgb(1.0, 0.8, 0.3)
                    },
                    z: layers::VFX,
                },
            );
        }
        shake.add(0.45);
    }
}
