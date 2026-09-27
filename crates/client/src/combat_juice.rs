//! Juice do combate ativo (v55): combo de afundamentos, tábuas voando do
//! casco que afunda e o tranco e a espuma do abalroar. O zoom de combate
//! fica na câmera (`camera::follow_camera`). Tudo só desenho: o servidor
//! decide quem afundou.

use bevy::prelude::*;
use lightyear::prelude::*;
use marvyr_protocol::{WorldEvent, WorldEventKind};

use crate::assets::layers;
use crate::camera::CameraShake;
use crate::juice::SeaEvent;
use crate::net::MyShip;
use crate::ship::ShipVisual;
use crate::vfx::{spawn_particle, Particle};

/// Afundamentos com menos disto entre um e outro somam combo.
const COMBO_WINDOW_SECS: f32 = 10.0;
/// Tábuas do casco que afunda.
const DEBRIS: usize = 14;

pub struct CombatJuicePlugin;

impl Plugin for CombatJuicePlugin {
    fn build(&self, app: &mut App) {
        app.init_resource::<Combo>().add_systems(
            Update,
            (count_combo, burst_debris, ram_feedback, draw_threats),
        );
    }
}

/// Sequência de afundamentos do meu navio.
#[derive(Resource, Debug, Default)]
pub struct Combo {
    pub count: u32,
    last_at: f32,
}

impl Combo {
    /// Conta um afundamento em `now`; devolve o combo atual.
    pub fn bump(&mut self, now: f32) -> u32 {
        self.count = if self.count > 0 && now - self.last_at <= COMBO_WINDOW_SECS {
            self.count + 1
        } else {
            1
        };
        self.last_at = now;
        self.count
    }
}

/// O servidor avisa só quem afundou ("Voce afundou…"): cada aviso é um
/// afundamento meu.
fn count_combo(
    mut commands: Commands,
    time: Res<Time>,
    mut events: EventReader<ClientReceiveMessage<WorldEvent>>,
    mut combo: ResMut<Combo>,
    mut shake: ResMut<CameraShake>,
    my_ship: Res<MyShip>,
    visuals: Query<&ShipVisual>,
) {
    for event in events.read() {
        let message = event.message();
        if message.kind != WorldEventKind::Kill || !message.text.starts_with("Voce afundou") {
            continue;
        }
        let count = combo.bump(time.elapsed_secs());
        if count < 2 {
            continue;
        }
        let Some(me) = my_ship
            .0
            .and_then(|id| visuals.iter().find(|visual| visual.target.ship_id == id))
        else {
            continue;
        };
        // Quanto maior o combo, mais quente a cor e mais forte o tranco.
        let heat = ((count - 1) as f32 / 4.0).min(1.0);
        let color = Color::srgb(1.0, 0.85 - 0.5 * heat, 0.3 - 0.2 * heat);
        crate::juice::spawn_float_text(
            &mut commands,
            Vec2::new(me.target.x, me.target.y + 40.0),
            crate::i18n::trf("COMBO x{0}!", &[&count.to_string()]),
            color,
        );
        shake.add(0.25 + 0.1 * count.min(5) as f32);
    }
}

/// Casco que afunda solta tábuas e fumaça (o letreiro já vem do juice).
fn burst_debris(mut commands: Commands, mut events: EventReader<SeaEvent>) {
    for event in events.read() {
        let SeaEvent::Sunk { at, .. } = *event else {
            continue;
        };
        for i in 0..DEBRIS {
            // Ângulos e forças fixos por índice: sem sorteio no desenho.
            let angle = i as f32 * 2.4;
            let speed = 30.0 + (i % 4) as f32 * 14.0;
            spawn_particle(
                &mut commands,
                at,
                Particle {
                    velocity: Vec2::from_angle(angle) * speed,
                    drag: 2.2,
                    life: 1.6 + (i % 3) as f32 * 0.3,
                    age: 0.0,
                    size: (5.0 - (i % 3) as f32, 2.0),
                    color: if i % 3 == 0 {
                        Color::srgb(0.35, 0.22, 0.12)
                    } else {
                        Color::srgb(0.55, 0.38, 0.2)
                    },
                    z: layers::VFX,
                },
            );
        }
    }
}

/// Abalroando: tranco na largada e espuma pelos dois bordos.
fn ram_feedback(
    mut commands: Commands,
    my_ship: Res<MyShip>,
    visuals: Query<&ShipVisual>,
    mut shake: ResMut<CameraShake>,
    mut was_ramming: Local<bool>,
) {
    let Some(me) = my_ship
        .0
        .and_then(|id| visuals.iter().find(|visual| visual.target.ship_id == id))
    else {
        return;
    };
    let ramming = me.target.ramming;
    if ramming && !*was_ramming {
        shake.add(0.35);
    }
    *was_ramming = ramming;
    if !ramming {
        return;
    }
    let at = Vec2::new(me.target.x, me.target.y);
    let heading = me.target.heading;
    for side in [-1.0_f32, 1.0] {
        let out = Vec2::from_angle(heading + side * 1.9) * 26.0;
        spawn_particle(
            &mut commands,
            at + Vec2::from_angle(heading) * 14.0,
            Particle::foam(out),
        );
    }
}

/// v56: perigo anunciado no mar — o círculo do tiro pesado da elite enche
/// até cair, e o brulote leva um anel pulsante do tamanho da explosão.
fn draw_threats(time: Res<Time>, visuals: Query<&ShipVisual>, mut gizmos: Gizmos) {
    let pulse = 0.5 + 0.5 * (time.elapsed_secs() * 8.0).sin();
    let danger = Color::srgb(1.0, 0.25, 0.2);
    for visual in &visuals {
        let state = &visual.target;
        if let Some((x, y, progress)) = state.telegraph {
            let at = Isometry2d::from_translation(Vec2::new(x, y));
            let radius = marvyr_protocol::HEAVY_SHOT_RADIUS;
            gizmos.circle_2d(at, radius, danger.with_alpha(0.9));
            // Enchimento: anéis até o raio atual (gizmo não preenche).
            let filled = radius * progress;
            let mut ring = 4.0;
            while ring < filled {
                gizmos.circle_2d(at, ring, danger.with_alpha(0.35));
                ring += 6.0;
            }
        }
        if state.npc_kind == 11 {
            gizmos.circle_2d(
                Isometry2d::from_translation(Vec2::new(state.x, state.y)),
                marvyr_protocol::FIRESHIP_BLAST_RADIUS,
                danger.with_alpha(0.25 + 0.5 * pulse),
            );
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn kills_close_together_stack_the_combo() {
        let mut combo = Combo::default();
        assert_eq!(combo.bump(0.0), 1);
        assert_eq!(combo.bump(5.0), 2);
        assert_eq!(combo.bump(14.0), 3);
        assert_eq!(combo.bump(14.0 + COMBO_WINDOW_SECS + 0.1), 1, "esfriou");
    }
}
