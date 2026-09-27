//! v63: sinais para os aliados (party e companhia). F5 Socorro, F6 Ataquem
//! aqui, F7 Reagrupar em mim: o servidor leva a posição do navio a quem é
//! aliado; aqui só o anel pulsando no mar, o letreiro e a linha de aviso.
//! Frases prontas (como a garrafa): nada de texto livre para moderar.

use bevy::prelude::*;
use lightyear::prelude::client::*;
use lightyear::prelude::*;
use marvyr_protocol::{Signal, SignalEvent};

use crate::assets::layers;
use crate::net::{MyDocked, ReliableChannel};

const KEYS: [(KeyCode, u8); 3] = [(KeyCode::F5, 1), (KeyCode::F6, 2), (KeyCode::F7, 3)];
/// Quanto tempo o marcador fica no mar (s).
const MARKER_SECS: f32 = 8.0;
const RING: f32 = 60.0;

pub struct SignalsPlugin;

impl Plugin for SignalsPlugin {
    fn build(&self, app: &mut App) {
        app.add_systems(
            Update,
            (send_signals, receive_signals, draw_markers).chain(),
        );
    }
}

#[derive(Component)]
struct SignalMarker {
    kind: u8,
    age: f32,
}

fn color_of(kind: u8) -> Color {
    match kind {
        1 => Color::srgb(1.0, 0.3, 0.25),
        2 => Color::srgb(1.0, 0.7, 0.2),
        _ => Color::srgb(0.45, 0.95, 0.55),
    }
}

fn send_signals(
    keys: Res<ButtonInput<KeyCode>>,
    time: Res<Time>,
    mut auto_clock: Local<f32>,
    docked: Res<MyDocked>,
    mut connection_manager: ResMut<ConnectionManager>,
) {
    if docked.0 {
        return;
    }
    let mut kind = KEYS
        .into_iter()
        .find(|(key, _)| keys.just_pressed(*key))
        .map(|(_, kind)| kind);
    // Dev (teste ao vivo sem teclado): MARVYR_AUTOSIGNAL=<1..3> manda o
    // sinal a cada 4 s.
    if let Some(auto) = std::env::var("MARVYR_AUTOSIGNAL")
        .ok()
        .and_then(|value| value.parse::<u8>().ok())
    {
        *auto_clock += time.delta_secs();
        if *auto_clock >= 4.0 {
            *auto_clock = 0.0;
            kind = Some(auto);
        }
    }
    if let Some(kind) = kind {
        let _ = connection_manager.send_message::<ReliableChannel, _>(&Signal { kind });
    }
}

fn receive_signals(
    mut commands: Commands,
    mut events: EventReader<ClientReceiveMessage<SignalEvent>>,
    mut notices: EventWriter<crate::net::PlayerNotice>,
) {
    for event in events.read() {
        let signal = event.message();
        let Some(name) = marvyr_protocol::signal_name(signal.kind) else {
            continue;
        };
        let text = format!("{}: {}", signal.from, crate::i18n::tr(name));
        notices.send(crate::net::PlayerNotice(text.clone()));
        commands.spawn((
            SignalMarker {
                kind: signal.kind,
                age: 0.0,
            },
            Text2d::new(text),
            TextFont {
                font: crate::ui::FONT_BOLD,
                font_size: 14.0,
                ..default()
            },
            TextColor(color_of(signal.kind)),
            Transform::from_xyz(signal.x, signal.y + RING + 14.0, layers::LABELS),
        ));
    }
}

/// Anel que pulsa e some; o letreiro esmaece junto.
fn draw_markers(
    mut commands: Commands,
    time: Res<Time>,
    mut markers: Query<(Entity, &mut SignalMarker, &Transform, &mut TextColor)>,
    mut gizmos: Gizmos,
) {
    for (entity, mut marker, transform, mut text_color) in &mut markers {
        marker.age += time.delta_secs();
        if marker.age >= MARKER_SECS {
            commands.entity(entity).despawn();
            continue;
        }
        let fade = 1.0 - marker.age / MARKER_SECS;
        let color = color_of(marker.kind);
        text_color.0 = color.with_alpha(fade);
        let at = Vec2::new(
            transform.translation.x,
            transform.translation.y - RING - 14.0,
        );
        let pulse = (marker.age * 2.0).fract();
        gizmos.circle_2d(
            Isometry2d::from_translation(at),
            RING * (0.3 + 0.7 * pulse),
            color.with_alpha(fade * (1.0 - pulse)),
        );
        gizmos.circle_2d(
            Isometry2d::from_translation(at),
            RING * 0.25,
            color.with_alpha(fade),
        );
    }
}
