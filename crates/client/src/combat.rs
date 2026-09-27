//! Combate ativo no client (v54). O capitão mira com o mouse: clique
//! esquerdo solta a salva do bordo que encara o cursor (segurar repete a
//! cada recarga), direito abalroa, Z abre o leque e X larga o barril na
//! esteira. No controle: RT salva, LT abalroa, L3 leque e R3 barril,
//! mirando no alvo do tiro automático. O servidor decide recarga, águas e
//! dano — aqui só vai a intenção e o ponto mirado.

use bevy::input::gamepad::{Gamepad, GamepadButton};
use bevy::prelude::*;
use bevy::window::PrimaryWindow;
use lightyear::prelude::client::*;
use marvyr_domain_combat::CombatActionKind;
use marvyr_protocol::{CombatAction, LootWreck};

use crate::net::{KnownWrecks, MyDocked, MyShip, ReliableChannel, LOOT_RADIUS_SQ};
use crate::ship::ShipVisual;

/// Segurando o clique, reenvia a salva no máximo nesta cadência (o servidor
/// só dispara quando a recarga fecha).
const HOLD_REPEAT_SECS: f32 = 0.25;
/// Sem alvo no controle, a salva sai pelo través a esta distância (m).
const PAD_AIM_REACH: f32 = 300.0;
/// Coleta automática: tenta de novo o mesmo destroço depois disto (s).
const AUTO_LOOT_RETRY_SECS: f32 = 4.0;

pub struct CombatInputPlugin;

impl Plugin for CombatInputPlugin {
    fn build(&self, app: &mut App) {
        app.init_resource::<AimPoint>().add_systems(
            Update,
            (
                (track_aim, send_combat_actions).chain(),
                draw_aim_reticle,
                auto_loot,
            ),
        );
    }
}

/// Ponto do mar sob o cursor (metros), quando o cursor está na janela.
#[derive(Resource, Debug, Default, Clone, Copy)]
pub struct AimPoint(pub Option<Vec2>);

fn track_aim(
    windows: Query<&Window, With<PrimaryWindow>>,
    camera: Query<(&Camera, &GlobalTransform), With<Camera2d>>,
    mut aim: ResMut<AimPoint>,
) {
    aim.0 = (|| {
        let cursor = windows.get_single().ok()?.cursor_position()?;
        let (camera, transform) = camera.get_single().ok()?;
        camera.viewport_to_world_2d(transform, cursor).ok()
    })();
}

/// Minha visão do meu navio no snapshot.
fn my_state<'a>(
    my_ship: &MyShip,
    visuals: &'a Query<&ShipVisual>,
) -> Option<&'a marvyr_protocol::ShipState> {
    let id = my_ship.0?;
    visuals
        .iter()
        .find(|visual| visual.target.ship_id == id)
        .map(|visual| &visual.target)
}

/// Mira do controle: o navio que o tiro automático escolheu, senão o
/// través de bombordo.
fn pad_aim(me: &marvyr_protocol::ShipState, visuals: &Query<&ShipVisual>) -> Vec2 {
    me.fire_target
        .and_then(|id| visuals.iter().find(|visual| visual.target.ship_id == id))
        .map(|visual| Vec2::new(visual.target.x, visual.target.y))
        .unwrap_or_else(|| {
            Vec2::new(me.x, me.y)
                + Vec2::from_angle(me.heading + std::f32::consts::FRAC_PI_2) * PAD_AIM_REACH
        })
}

#[allow(clippy::too_many_arguments)]
fn send_combat_actions(
    time: Res<Time>,
    mouse: Res<ButtonInput<MouseButton>>,
    keys: Res<ButtonInput<KeyCode>>,
    gamepads: Query<&Gamepad>,
    aim: Res<AimPoint>,
    docked: Res<MyDocked>,
    modal: Res<crate::input::ModalOpen>,
    chart: Query<(), With<crate::chart::ChartOverlay>>,
    buttons: Query<&Interaction>,
    my_ship: Res<MyShip>,
    visuals: Query<&ShipVisual>,
    mut hold_clock: Local<f32>,
    mut auto_clock: Local<f32>,
    mut connection_manager: ResMut<ConnectionManager>,
) {
    if docked.0 || modal.any() || !chart.is_empty() {
        return;
    }
    let Some(me) = my_state(&my_ship, &visuals) else {
        return;
    };
    let pad = |button| gamepads.iter().any(|pad| pad.just_pressed(button));
    let pad_held = |button| gamepads.iter().any(|pad| pad.pressed(button));
    // Clique em botão (tática do duelo, talentos) não dispara canhão.
    let over_ui = buttons.iter().any(|state| *state != Interaction::None);
    let mouse_aim = aim.0.filter(|_| !over_ui);
    let pad_point = pad_aim(me, &visuals);

    *hold_clock += time.delta_secs();
    let mut actions: Vec<(CombatActionKind, Vec2)> = Vec::new();
    if let Some(point) = mouse_aim {
        let fire_again = mouse.pressed(MouseButton::Left) && *hold_clock >= HOLD_REPEAT_SECS;
        if mouse.just_pressed(MouseButton::Left) || fire_again {
            actions.push((CombatActionKind::Broadside, point));
        }
        if mouse.just_pressed(MouseButton::Right) {
            actions.push((CombatActionKind::Ram, point));
        }
        if keys.just_pressed(KeyCode::KeyZ) {
            actions.push((CombatActionKind::FanSalvo, point));
        }
    }
    if keys.just_pressed(KeyCode::KeyX) {
        actions.push((CombatActionKind::FireBarrel, mouse_aim.unwrap_or(pad_point)));
    }
    let pad_fire_again = pad_held(GamepadButton::RightTrigger2) && *hold_clock >= HOLD_REPEAT_SECS;
    if pad(GamepadButton::RightTrigger2) || pad_fire_again {
        actions.push((CombatActionKind::Broadside, pad_point));
    }
    if pad(GamepadButton::LeftTrigger2) {
        actions.push((CombatActionKind::Ram, pad_point));
    }
    if pad(GamepadButton::LeftThumb) {
        actions.push((CombatActionKind::FanSalvo, pad_point));
    }
    if pad(GamepadButton::RightThumb) {
        actions.push((CombatActionKind::FireBarrel, pad_point));
    }

    // Dev (teste ao vivo sem teclado): MARVYR_AUTOCOMBAT=1 mira o pirata
    // mais perto e gira salva, leque, barril e abalroar.
    if std::env::var_os("MARVYR_AUTOCOMBAT").is_some() {
        *auto_clock += time.delta_secs();
        let prey = visuals
            .iter()
            .map(|visual| &visual.target)
            .filter(|ship| ship.faction == marvyr_protocol::Faction::Pirate && ship.hp > 0)
            .map(|ship| Vec2::new(ship.x, ship.y))
            .min_by(|a, b| {
                let mine = Vec2::new(me.x, me.y);
                a.distance(mine).total_cmp(&b.distance(mine))
            })
            .filter(|at| at.distance(Vec2::new(me.x, me.y)) <= me.weapon_range * 1.1);
        if let (Some(at), true) = (prey, *auto_clock >= 0.5) {
            *auto_clock = 0.0;
            for kind in [
                CombatActionKind::Broadside,
                CombatActionKind::FanSalvo,
                CombatActionKind::FireBarrel,
            ] {
                actions.push((kind, at));
            }
            // Abalroar só colado: longe, a arrancada leva para fora da luta.
            if at.distance(Vec2::new(me.x, me.y)) < 120.0 {
                actions.push((CombatActionKind::Ram, at));
            }
        }
    }

    for (kind, point) in actions {
        if kind == CombatActionKind::Broadside {
            *hold_clock = 0.0;
        }
        let _ = connection_manager.send_message::<ReliableChannel, _>(&CombatAction {
            kind,
            aim_x: point.x,
            aim_y: point.y,
        });
    }
}

/// Retícula no cursor e a linha do bordo que vai disparar (verde pronto,
/// âmbar recarregando). Só no mar.
fn draw_aim_reticle(
    aim: Res<AimPoint>,
    docked: Res<MyDocked>,
    my_ship: Res<MyShip>,
    visuals: Query<&ShipVisual>,
    mut gizmos: Gizmos,
) {
    if docked.0 {
        return;
    }
    let (Some(point), Some(me)) = (aim.0, my_state(&my_ship, &visuals)) else {
        return;
    };
    let ship = Vec2::new(me.x, me.y);
    let reach = me.weapon_range.max(1.0);
    let ready = me.port_cooldown_secs <= 0.0 && me.starboard_cooldown_secs <= 0.0;
    let color = if ready {
        Color::srgba(0.55, 0.95, 0.6, 0.85)
    } else {
        Color::srgba(0.95, 0.7, 0.3, 0.7)
    };
    let to = (point - ship).clamp_length_max(reach);
    gizmos.circle_2d(Isometry2d::from_translation(point), 7.0, color);
    gizmos.line_2d(point - Vec2::X * 11.0, point + Vec2::X * 11.0, color);
    gizmos.line_2d(point - Vec2::Y * 11.0, point + Vec2::Y * 11.0, color);
    crate::ui::dashed_line(&mut gizmos, ship, ship + to, 14.0, color.with_alpha(0.35));
}

/// Coleta automática: passou por cima de um destroço, o porão puxa. Cada
/// destroço é tentado de novo só depois de um tempo (porão cheio, janela
/// de saque de outro capitão).
fn auto_loot(
    time: Res<Time>,
    docked: Res<MyDocked>,
    my_ship: Res<MyShip>,
    known_wrecks: Res<KnownWrecks>,
    visuals: Query<&ShipVisual>,
    mut tried: Local<std::collections::HashMap<u32, f32>>,
    mut connection_manager: ResMut<ConnectionManager>,
) {
    if docked.0 {
        return;
    }
    let Some(me) = my_state(&my_ship, &visuals) else {
        return;
    };
    let now = time.elapsed_secs();
    tried.retain(|id, at| known_wrecks.0.contains_key(id) && now - *at < AUTO_LOOT_RETRY_SECS);
    let mine = Vec2::new(me.x, me.y);
    for (wreck_id, at) in &known_wrecks.0 {
        if mine.distance_squared(*at) > LOOT_RADIUS_SQ || tried.contains_key(wreck_id) {
            continue;
        }
        tried.insert(*wreck_id, now);
        let _ = connection_manager.send_message::<ReliableChannel, _>(&LootWreck {
            wreck_id: *wreck_id,
        });
    }
}
