//! Câmera do mar (MF-058): segue o próprio navio olhando um pouco à frente
//! da proa e aceita zoom pela roda do mouse. A UI vive em `bevy_ui` (espaço
//! de tela), então o zoom não mexe no HUD.

use bevy::input::mouse::{MouseScrollUnit, MouseWheel};
use bevy::prelude::*;

use crate::ship::ShipVisual;

/// Metros por pixel de tela: 0.5 = o navio médio (40 m) ocupa 80 px.
const DEFAULT_ZOOM: f32 = 0.5;
const MIN_ZOOM: f32 = 0.2;
const MAX_ZOOM: f32 = 1.4;
/// v55: zoom máximo (mais perto) durante o combate.
const COMBAT_ZOOM: f32 = 0.4;

/// Zoom desejado; a projeção persegue suavemente.
#[derive(Resource)]
pub struct CameraZoom(pub f32);

impl Default for CameraZoom {
    fn default() -> Self {
        Self(DEFAULT_ZOOM)
    }
}

pub fn setup_camera(mut commands: Commands) {
    commands.spawn((
        Camera2d,
        // Bevy 0.15: a Camera2d lê `OrthographicProjection`; um `Projection`
        // avulso é ignorado (era por isso que o zoom antigo não valia).
        OrthographicProjection {
            scale: DEFAULT_ZOOM,
            ..OrthographicProjection::default_2d()
        },
        // Começa sobre o Porto da Serra para não abrir num mar vazio.
        Transform::from_xyz(-560.0, 0.0, 0.0),
        // Ouvido do áudio espacial (MF-059): o pan estéreo sai daqui.
        SpatialListener::new(crate::audio::EAR_GAP),
    ));
}

pub fn zoom_from_wheel(
    time: Res<Time>,
    mut wheel: EventReader<MouseWheel>,
    gamepads: Query<&bevy::input::gamepad::Gamepad>,
    mut zoom: ResMut<CameraZoom>,
) {
    for event in wheel.read() {
        let steps = match event.unit {
            MouseScrollUnit::Line => event.y,
            MouseScrollUnit::Pixel => event.y / 40.0,
        };
        zoom.0 = next_zoom(zoom.0, steps);
    }
    // Analógico direito: ~4 passos de roda por segundo com ele no fim.
    let stick = crate::input::stick_zoom(&gamepads);
    if stick != 0.0 {
        zoom.0 = next_zoom(zoom.0, stick * 4.0 * time.delta_secs());
    }
}

/// Zoom multiplicativo: cada passo aproxima/afasta ~12%.
fn next_zoom(current: f32, steps: f32) -> f32 {
    (current * 0.88_f32.powf(steps)).clamp(MIN_ZOOM, MAX_ZOOM)
}

/// Distância que só um salto de portal explica.
const TELEPORT_JUMP: f32 = 1500.0;

pub fn follow_camera(
    time: Res<Time>,
    zoom: Res<CameraZoom>,
    my_ship: Res<crate::net::MyShip>,
    visuals: Query<(&ShipVisual, &Transform), Without<Camera2d>>,
    mut camera: Query<(&mut Transform, &mut OrthographicProjection), With<Camera2d>>,
) {
    let Ok((mut transform, mut ortho)) = camera.get_single_mut() else {
        return;
    };
    let dt = time.delta_secs();
    let me = my_ship.0.and_then(|my_id| {
        visuals
            .iter()
            .find(|(visual, _)| visual.target.ship_id == my_id)
    });
    // v55: em combate a câmera chega um pouco mais perto (sem afastar quem
    // já está perto); fora dele volta ao zoom da roda.
    let fighting =
        me.is_some_and(|(visual, _)| visual.target.fire_target.is_some() || visual.target.ramming);
    let wanted = if fighting {
        zoom.0.min(COMBAT_ZOOM)
    } else {
        zoom.0
    };
    if (wanted - ortho.scale).abs() > 1e-4 {
        // Entrar em combate é mais lento que o zoom da roda: sem tranco.
        let rate = if fighting { 2.5 } else { 10.0 };
        ortho.scale += (wanted - ortho.scale) * (1.0 - (-rate * dt).exp());
    }
    let Some((visual, ship)) = me else {
        return;
    };
    // Olha à frente da proa, proporcional ao seguimento: quem navega rápido
    // precisa ver o que vem, não o que ficou.
    let lead = Vec2::from_angle(visual.target.heading) * visual.target.speed * 1.6;
    let goal = (ship.translation.truncate() + lead).extend(transform.translation.z);
    // Portal ou portão de zona: salta junto, sem panorâmica pelo vazio.
    if transform.translation.truncate().distance(goal.truncate()) > TELEPORT_JUMP {
        transform.translation = goal;
        return;
    }
    transform.translation = transform.translation.lerp(goal, 1.0 - (-3.5 * dt).exp());
}

/// Tremor de câmera por "trauma" (MF-059): eventos somam trauma, o
/// deslocamento é `trauma²` × ruído e o trauma decai sozinho.
#[derive(Resource, Default)]
pub struct CameraShake {
    pub trauma: f32,
    /// Deslocamento aplicado no frame anterior (retirado antes do follow).
    applied: Vec2,
}

/// Deslocamento máximo, em pixels de tela.
const MAX_SHAKE_PX: f32 = 14.0;
/// Trauma perdido por segundo.
const TRAUMA_DECAY: f32 = 1.5;

impl CameraShake {
    pub fn add(&mut self, amount: f32) {
        self.trauma = (self.trauma + amount).clamp(0.0, 1.0);
    }
}

pub fn decay_trauma(trauma: f32, dt: f32) -> f32 {
    (trauma - TRAUMA_DECAY * dt).max(0.0)
}

/// Deslocamento em pixels: quadrático no trauma, ruído suave no tempo.
pub fn shake_offset(trauma: f32, t: f32) -> Vec2 {
    let noise = |t: f32| (t.sin() * 0.6 + (t * 2.3 + 1.7).sin() * 0.4).clamp(-1.0, 1.0);
    let t = t * 25.0;
    Vec2::new(noise(t), noise(t + 37.0)) * MAX_SHAKE_PX * trauma * trauma
}

/// Antes do follow: tira o tremor do frame anterior para o lerp de
/// `follow_camera` partir da posição real, não da tremida.
pub fn unshake_camera(
    mut shake: ResMut<CameraShake>,
    mut camera: Query<&mut Transform, With<Camera2d>>,
) {
    if let Ok(mut transform) = camera.get_single_mut() {
        transform.translation -= shake.applied.extend(0.0);
    }
    shake.applied = Vec2::ZERO;
}

/// Depois do follow: aplica o tremor deste frame por cima.
pub fn shake_camera(
    time: Res<Time>,
    mut shake: ResMut<CameraShake>,
    mut camera: Query<(&mut Transform, &OrthographicProjection), With<Camera2d>>,
) {
    shake.trauma = decay_trauma(shake.trauma, time.delta_secs());
    let Ok((mut transform, ortho)) = camera.get_single_mut() else {
        return;
    };
    let offset = shake_offset(shake.trauma, time.elapsed_secs()) * ortho.scale;
    transform.translation += offset.extend(0.0);
    shake.applied = offset;
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn trauma_decays_to_zero_and_never_below() {
        assert!((decay_trauma(1.0, 0.2) - 0.7).abs() < 1e-6);
        assert_eq!(decay_trauma(0.1, 1.0), 0.0);
        let mut shake = CameraShake::default();
        shake.add(0.8);
        shake.add(0.8);
        assert_eq!(shake.trauma, 1.0);
    }

    #[test]
    fn shake_is_quadratic_in_trauma_and_bounded() {
        assert_eq!(shake_offset(0.0, 3.0), Vec2::ZERO);
        for i in 0..200 {
            let t = i as f32 * 0.037;
            let full = shake_offset(1.0, t);
            assert!(full.x.abs() <= MAX_SHAKE_PX && full.y.abs() <= MAX_SHAKE_PX);
            let half = shake_offset(0.5, t);
            assert!((half - full * 0.25).length() < 1e-4);
        }
    }

    #[test]
    fn zoom_steps_are_multiplicative_and_clamped() {
        assert!(next_zoom(0.5, 1.0) < 0.5);
        assert!(next_zoom(0.5, -1.0) > 0.5);
        assert_eq!(next_zoom(0.5, 100.0), MIN_ZOOM);
        assert_eq!(next_zoom(0.5, -100.0), MAX_ZOOM);
    }
}
