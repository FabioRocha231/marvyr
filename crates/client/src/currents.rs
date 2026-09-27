//! v48: correntes no client. O servidor manda as faixas da semana
//! (`SeaCurrents`) e empurra o navio; aqui só os tracinhos de espuma
//! correndo no sentido da água, para o capitão ver a rota rápida.

use bevy::prelude::*;
use lightyear::prelude::ClientReceiveMessage;
use marvyr_domain_world::current::{PUSH, WIDTH};
use marvyr_protocol::SeaCurrents;

use crate::assets::layers;

const FOAM: Color = Color::srgba(0.55, 0.92, 1.0, 0.8);
/// Um tracinho a cada tantos metros de faixa.
const SPACING: f32 = 45.0;
/// Os tracinhos andam mais rápido que o empurrão, para ler de longe.
const DRIFT: f32 = PUSH * 6.0;

#[derive(Resource, Default)]
pub struct KnownCurrents(pub Vec<(Vec2, Vec2)>);

#[derive(Component)]
struct Streak {
    lane: usize,
    /// Posição ao longo da faixa (0..1) e o desvio lateral (m).
    t: f32,
    side: f32,
}

pub struct CurrentsPlugin;

impl Plugin for CurrentsPlugin {
    fn build(&self, app: &mut App) {
        app.init_resource::<KnownCurrents>()
            .add_systems(Update, (receive, spawn_streaks, drift).chain());
    }
}

fn receive(
    mut updates: EventReader<ClientReceiveMessage<SeaCurrents>>,
    mut known: ResMut<KnownCurrents>,
) {
    if let Some(update) = updates.read().last() {
        info!(
            lanes = update.message().lanes.len(),
            "correntes da semana recebidas"
        );
        known.0 = update
            .message()
            .lanes
            .iter()
            .map(|(x0, y0, x1, y1)| (Vec2::new(*x0, *y0), Vec2::new(*x1, *y1)))
            .collect();
    }
}

/// Semente fixa por tracinho: espalha sem `rand` (mesma tela sempre).
fn scatter(index: usize) -> f32 {
    let mut x = (index as u32).wrapping_mul(0x9E37_79B9) ^ 0x5bd1_e995;
    x ^= x >> 15;
    x = x.wrapping_mul(0x2c1b_3c6d);
    x ^= x >> 12;
    (x & 0xffff) as f32 / 65_535.0
}

fn spawn_streaks(
    mut commands: Commands,
    known: Res<KnownCurrents>,
    streaks: Query<Entity, With<Streak>>,
) {
    if !known.is_changed() {
        return;
    }
    for entity in &streaks {
        commands.entity(entity).despawn();
    }
    let mut spawned = 0;
    for (lane, (from, to)) in known.0.iter().enumerate() {
        let count = (from.distance(*to) / SPACING).max(1.0) as usize;
        let angle = (*to - *from).to_angle();
        for index in 0..count {
            let seed = lane * 1_000 + index;
            commands.spawn((
                Streak {
                    lane,
                    t: index as f32 / count as f32,
                    side: (scatter(seed) - 0.5) * WIDTH * 0.8,
                },
                Sprite::from_color(FOAM, Vec2::new(24.0, 3.0)),
                Transform::from_translation(from.extend(layers::WAKE + 0.2))
                    .with_rotation(Quat::from_rotation_z(angle)),
            ));
            spawned += 1;
        }
    }
    debug!(spawned, "espuma das correntes");
}

fn drift(
    time: Res<Time>,
    known: Res<KnownCurrents>,
    mut streaks: Query<(&mut Streak, &mut Transform, &mut Sprite)>,
) {
    let dt = time.delta_secs();
    for (mut streak, mut transform, mut sprite) in &mut streaks {
        let Some((from, to)) = known.0.get(streak.lane) else {
            continue;
        };
        let along = *to - *from;
        let length = along.length().max(1.0);
        streak.t = (streak.t + DRIFT * dt / length).fract();
        let normal = along.perp() / length;
        let at = *from + along * streak.t + normal * streak.side;
        transform.translation.x = at.x;
        transform.translation.y = at.y;
        // Some nas pontas: a faixa começa e termina na água, sem corte.
        let edge = (streak.t.min(1.0 - streak.t) * 8.0).min(1.0);
        sprite.color = FOAM.with_alpha(0.8 * edge);
    }
}
