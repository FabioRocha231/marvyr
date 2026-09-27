//! Controle (gamepad) e dispositivo ativo (MV-062).
//!
//! O jogo inteiro lê `ButtonInput<KeyCode>`. Em vez de reescrever cada
//! sistema, a ponte traduz o controle para as mesmas teclas logo depois da
//! leitura de entrada do Bevy: tudo que funciona no teclado funciona no
//! controle, inclusive a tela de porto (setas, Enter, Tab, Esc).
//!
//! No mar: analógico/direcional = leme e velas, LB/RB = bordos, A = a ação
//! do bilhete de contexto, X = reparo, Y = munição, B/Start = livreto,
//! Select = idioma.
//! Em menus: direcional = setas, A = Enter, B = Esc, LB/RB = abas.

use std::collections::HashSet;

use bevy::input::gamepad::{Gamepad, GamepadAxis, GamepadButton};
use bevy::input::InputSystem;
use bevy::prelude::*;

/// Último dispositivo usado: decide se os bilhetes mostram "G" ou "A".
#[derive(Resource, Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum InputDevice {
    #[default]
    Keyboard,
    Gamepad,
}

/// Tecla que o botão A aciona no mar (a do bilhete de contexto da vez).
#[derive(Resource, Debug, Default, Clone, Copy, PartialEq, Eq)]
pub struct ContextKey(pub Option<KeyCode>);

/// Painéis modais na frente do mar. Com qualquer um aberto o controle
/// navega menu em vez de pilotar e as teclas vão só para o modal.
#[derive(Resource, Debug, Default, Clone, Copy, PartialEq, Eq)]
pub struct ModalOpen {
    pub book: bool,
    pub welcome: bool,
    /// v51: bilhete da mensagem na garrafa (F3).
    pub bottle: bool,
    /// v62: digitando o nome da companhia.
    pub company: bool,
}

impl ModalOpen {
    pub fn any(&self) -> bool {
        self.book || self.welcome || self.bottle || self.company
    }
}

/// Teclas do quadro, desviadas para o modal aberto. O jogo (leme, velas,
/// canhões…) lê `ButtonInput<KeyCode>`, que fica vazio enquanto um modal
/// está na frente: uma guarda só, na raiz, em vez de uma por sistema.
#[derive(Resource, Debug, Default)]
pub struct ModalKeys(pub ButtonInput<KeyCode>);

/// As teclas que um modal deve ler: as desviadas quando ele está aberto.
pub fn modal_keys<'a>(
    modal: &ModalOpen,
    captured: &'a ModalKeys,
    keys: &'a ButtonInput<KeyCode>,
) -> &'a ButtonInput<KeyCode> {
    if modal.any() {
        &captured.0
    } else {
        keys
    }
}

/// Vaga de tecla num bilhete: o quadrinho muda sozinho quando o jogador
/// troca de teclado para controle (e volta).
#[derive(Component, Debug, Clone, Copy, PartialEq, Eq)]
pub struct KeySlot(pub KeyCode);

/// v54: botões do mouse no HUD. Teclas que nenhum teclado comum tem, usadas
/// só como vaga de glifo — o `KeySlot` troca sozinho para RT/LT no controle.
///
/// ponytail: `KeySlot` só conhece `KeyCode`; um enum de "entrada" próprio
/// vale a pena se o mouse ganhar mais ações no HUD.
pub const MOUSE_LEFT: KeyCode = KeyCode::F13;
pub const MOUSE_RIGHT: KeyCode = KeyCode::F14;

const STICK_HOLD: f32 = 0.35;
const STICK_FLICK: f32 = 0.65;

pub struct GamepadPlugin;

impl Plugin for GamepadPlugin {
    fn build(&self, app: &mut App) {
        // Dev: MARVYR_DEVICE=gamepad mostra os glifos do controle sem ter um.
        let device = match std::env::var("MARVYR_DEVICE").as_deref() {
            Ok("gamepad") => InputDevice::Gamepad,
            _ => InputDevice::default(),
        };
        app.insert_resource(device)
            .init_resource::<ContextKey>()
            .init_resource::<ModalOpen>()
            .init_resource::<ModalKeys>()
            .add_systems(
                PreUpdate,
                (detect_device, bridge_gamepad, capture_for_modal)
                    .chain()
                    .after(InputSystem),
            )
            .add_systems(PostUpdate, refresh_key_slots);
    }
}

fn detect_device(
    keys: Res<ButtonInput<KeyCode>>,
    mouse: Res<ButtonInput<MouseButton>>,
    gamepads: Query<&Gamepad>,
    mut device: ResMut<InputDevice>,
) {
    let pad_active = gamepads.iter().any(|pad| {
        pad.get_just_pressed().next().is_some()
            || pad.left_stick().length() > STICK_FLICK
            || pad.right_stick().length() > STICK_FLICK
    });
    let keyboard_active =
        keys.get_just_pressed().next().is_some() || mouse.get_just_pressed().next().is_some();
    let next = if pad_active {
        InputDevice::Gamepad
    } else if keyboard_active {
        InputDevice::Keyboard
    } else {
        *device
    };
    device.set_if_neq(next);
}

/// Estado da ponte entre quadros: o que ela mesma segurou (para soltar só
/// isso) e onde o analógico estava (para "piscar" setas/velas uma vez).
#[derive(Default)]
struct Bridge {
    held: HashSet<KeyCode>,
    stick_y: i8,
    stick_x: i8,
}

fn bridge_gamepad(
    gamepads: Query<&Gamepad>,
    docked: Res<crate::net::MyDocked>,
    modal: Res<ModalOpen>,
    context: Res<ContextKey>,
    mut keys: ResMut<ButtonInput<KeyCode>>,
    mut bridge: Local<Bridge>,
) {
    let menu = docked.0 || modal.any();
    let mut hold: HashSet<KeyCode> = HashSet::new();
    let mut taps: Vec<KeyCode> = Vec::new();
    let mut stick_y = 0_i8;
    let mut stick_x = 0_i8;

    for pad in &gamepads {
        let stick = pad.left_stick();
        let dpad_x = axis(pad, GamepadButton::DPadLeft, GamepadButton::DPadRight);
        if flick(stick.y) != 0 {
            stick_y = flick(stick.y);
        }
        if flick(stick.x) != 0 {
            stick_x = flick(stick.x);
        }

        let tap = |button: GamepadButton, key: KeyCode, taps: &mut Vec<KeyCode>| {
            if pad.just_pressed(button) {
                taps.push(key);
            }
        };
        tap(GamepadButton::Start, KeyCode::Escape, &mut taps);
        tap(GamepadButton::Select, KeyCode::KeyL, &mut taps);
        if menu {
            if docked.0 {
                tap(GamepadButton::West, KeyCode::KeyP, &mut taps);
            }
            tap(GamepadButton::East, KeyCode::Escape, &mut taps);
            tap(GamepadButton::South, KeyCode::Enter, &mut taps);
            tap(GamepadButton::RightTrigger, KeyCode::Tab, &mut taps);
            if pad.just_pressed(GamepadButton::LeftTrigger) {
                hold.insert(KeyCode::ShiftLeft);
                taps.push(KeyCode::Tab);
            }
            tap(GamepadButton::DPadUp, KeyCode::ArrowUp, &mut taps);
            tap(GamepadButton::DPadDown, KeyCode::ArrowDown, &mut taps);
            tap(GamepadButton::DPadLeft, KeyCode::ArrowLeft, &mut taps);
            tap(GamepadButton::DPadRight, KeyCode::ArrowRight, &mut taps);
        } else {
            if let Some(key) = context.0 {
                tap(GamepadButton::South, key, &mut taps);
            }
            tap(GamepadButton::LeftTrigger, KeyCode::KeyQ, &mut taps);
            tap(GamepadButton::RightTrigger, KeyCode::KeyR, &mut taps);
            tap(GamepadButton::West, KeyCode::KeyK, &mut taps);
            tap(GamepadButton::North, KeyCode::KeyC, &mut taps);
            tap(GamepadButton::DPadUp, KeyCode::KeyW, &mut taps);
            tap(GamepadButton::DPadDown, KeyCode::KeyS, &mut taps);
            // Leme: segura enquanto o analógico/direcional estiver torto.
            if stick.x < -STICK_HOLD || dpad_x < 0.0 {
                hold.insert(KeyCode::ArrowLeft);
            }
            if stick.x > STICK_HOLD || dpad_x > 0.0 {
                hold.insert(KeyCode::ArrowRight);
            }
        }
    }

    // Analógico vertical/horizontal "pisca" uma vez por empurrão.
    if stick_y != bridge.stick_y {
        match (stick_y, menu) {
            (1, true) => taps.push(KeyCode::ArrowUp),
            (-1, true) => taps.push(KeyCode::ArrowDown),
            (1, false) => taps.push(KeyCode::KeyW),
            (-1, false) => taps.push(KeyCode::KeyS),
            _ => {}
        }
        bridge.stick_y = stick_y;
    }
    if stick_x != bridge.stick_x {
        if menu {
            match stick_x {
                1 => taps.push(KeyCode::ArrowRight),
                -1 => taps.push(KeyCode::ArrowLeft),
                _ => {}
            }
        }
        bridge.stick_x = stick_x;
    }

    for key in bridge.held.difference(&hold) {
        keys.release(*key);
    }
    for key in &hold {
        keys.press(*key);
    }
    for key in taps {
        keys.press(key);
        // Toque: solta no mesmo quadro — `just_pressed` fica valendo neste
        // quadro e o próximo `clear` do Bevy limpa o resto.
        keys.release(key);
    }
    bridge.held = hold;
}

fn refresh_key_slots(
    mut commands: Commands,
    device: Res<InputDevice>,
    lang: Res<crate::i18n::Lang>,
    slots: Query<(Entity, Ref<KeySlot>)>,
) {
    for (entity, slot) in &slots {
        if !(device.is_changed() || lang.is_changed() || slot.is_changed()) {
            continue;
        }
        // "Mouse E/D" traduz (Mouse L/R); letras e botões passam iguais.
        let label = crate::i18n::tr(glyph(slot.0, *device));
        commands.entity(entity).despawn_descendants();
        commands
            .entity(entity)
            .with_children(|parent| crate::ui::keycap(parent, &label));
    }
}

fn capture_for_modal(
    modal: Res<ModalOpen>,
    mut keys: ResMut<ButtonInput<KeyCode>>,
    mut captured: ResMut<ModalKeys>,
) {
    if modal.any() {
        captured.0 = keys.clone();
        keys.reset_all();
    } else if captured.0.get_pressed().next().is_some()
        || captured.0.get_just_pressed().next().is_some()
    {
        captured.0 = ButtonInput::default();
    }
}

fn axis(pad: &Gamepad, negative: GamepadButton, positive: GamepadButton) -> f32 {
    let mut value = 0.0;
    if pad.pressed(negative) {
        value -= 1.0;
    }
    if pad.pressed(positive) {
        value += 1.0;
    }
    value
}

/// -1, 0 ou 1: o analógico passou do limiar de "empurrão".
fn flick(value: f32) -> i8 {
    if value > STICK_FLICK {
        1
    } else if value < -STICK_FLICK {
        -1
    } else {
        0
    }
}

/// Zoom pelo analógico direito (a roda do mouse faz o mesmo).
pub fn stick_zoom(gamepads: &Query<&Gamepad>) -> f32 {
    gamepads
        .iter()
        .filter_map(|pad| pad.get(GamepadAxis::RightStickY))
        .find(|value| value.abs() > STICK_HOLD)
        .unwrap_or(0.0)
}

/// Rótulo de uma tecla no dispositivo ativo, para bilhetes e livreto.
pub fn glyph(key: KeyCode, device: InputDevice) -> &'static str {
    match device {
        InputDevice::Gamepad => pad_glyph(key),
        InputDevice::Keyboard => key_glyph(key),
    }
}

fn key_glyph(key: KeyCode) -> &'static str {
    match key {
        KeyCode::KeyW => "W",
        KeyCode::KeyS => "S",
        KeyCode::KeyA => "A",
        KeyCode::KeyD => "D",
        KeyCode::KeyQ => "Q",
        KeyCode::KeyR => "R",
        KeyCode::KeyC => "C",
        KeyCode::KeyE => "E",
        KeyCode::KeyF => "F",
        KeyCode::KeyG => "G",
        KeyCode::KeyH => "H",
        KeyCode::KeyJ => "J",
        KeyCode::KeyK => "K",
        KeyCode::KeyB => "B",
        KeyCode::KeyP => "P",
        KeyCode::KeyL => "L",
        KeyCode::KeyM => "M",
        KeyCode::KeyZ => "Z",
        KeyCode::KeyX => "X",
        MOUSE_LEFT => "Mouse E",
        MOUSE_RIGHT => "Mouse D",
        KeyCode::F1 => "F1",
        KeyCode::F3 => "F3",
        KeyCode::Digit1 => "1",
        KeyCode::Digit2 => "2",
        KeyCode::Digit3 => "3",
        KeyCode::Digit4 => "4",
        KeyCode::Escape => "Esc",
        KeyCode::Enter => "Enter",
        KeyCode::Tab => "Tab",
        _ => "?",
    }
}

fn pad_glyph(key: KeyCode) -> &'static str {
    match key {
        KeyCode::KeyW => "D-pad cima",
        KeyCode::KeyS => "D-pad baixo",
        KeyCode::KeyA | KeyCode::KeyD => "Analógico",
        KeyCode::KeyQ => "LB",
        KeyCode::KeyR => "RB",
        KeyCode::KeyC => "Y",
        KeyCode::KeyK => "X",
        // A aciona o bilhete de contexto: atracar, coletar, saquear,
        // abordar, cavar, contratar.
        KeyCode::KeyE
        | KeyCode::KeyF
        | KeyCode::KeyG
        | KeyCode::KeyH
        | KeyCode::KeyJ
        | KeyCode::KeyB
        | KeyCode::Enter => "A",
        KeyCode::KeyP => "X",
        KeyCode::Escape | KeyCode::F1 => "Start",
        KeyCode::Tab => "LB/RB",
        KeyCode::KeyL => "Select",
        KeyCode::KeyZ => "L3",
        KeyCode::KeyX => "R3",
        MOUSE_LEFT => "RT",
        MOUSE_RIGHT => "LT",
        _ => "?",
    }
}

#[cfg(test)]
pub(crate) fn init_systems_for_tests(world: &mut World) {
    fn init<M>(world: &mut World, system: impl IntoSystem<(), (), M>) {
        let mut system = IntoSystem::into_system(system);
        system.initialize(world);
    }
    init(world, detect_device);
    init(world, bridge_gamepad);
    init(world, capture_for_modal);
    init(world, refresh_key_slots);
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn context_actions_share_the_a_button_on_pad() {
        for key in [KeyCode::KeyE, KeyCode::KeyG, KeyCode::KeyF, KeyCode::KeyJ] {
            assert_eq!(glyph(key, InputDevice::Gamepad), "A");
        }
        assert_eq!(glyph(KeyCode::KeyG, InputDevice::Keyboard), "G");
    }

    #[test]
    fn flick_needs_a_real_push() {
        assert_eq!(flick(0.3), 0);
        assert_eq!(flick(0.9), 1);
        assert_eq!(flick(-0.9), -1);
    }
}
