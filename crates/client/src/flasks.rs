//! Frascos de bordo na tela (v25): teclas 1-4 no mar, cinto no HUD com o
//! líquido na altura das cargas, e o efeito visível em volta do casco para
//! todo mundo. O servidor decide cargas e efeito; aqui só se mostra — a
//! recusa local (sem dose, sem frasco, já ligado) é só para o cinto tremer
//! em vez de mandar um pedido que o servidor ignoraria.

use std::collections::HashMap;

use bevy::prelude::*;
use lightyear::prelude::client::*;
use marvyr_domain_combat::flask::{DOSE, MAX_CHARGES};
use marvyr_domain_combat::FlaskKind;
use marvyr_protocol::{FlaskWire, UseFlask};

use crate::assets::{icons, layers, ItemIcons};
use crate::input::KeySlot;
use crate::net::{MyDocked, MyShip, ReliableChannel};
use crate::ship::ShipVisual;
use crate::ui;
use crate::vfx::{spawn_particle, Particle};

pub const KEYS: [KeyCode; 4] = [
    KeyCode::Digit1,
    KeyCode::Digit2,
    KeyCode::Digit3,
    KeyCode::Digit4,
];

/// Cor do líquido (mesma paleta do atlas).
pub fn flask_color(kind: FlaskKind) -> Color {
    let (r, g, b) = match kind {
        FlaskKind::Repair => (214, 52, 60),
        FlaskKind::Wind => (150, 226, 240),
        FlaskKind::Fury => (250, 150, 40),
        FlaskKind::Tar => (104, 60, 132),
    };
    Color::srgb_u8(r, g, b)
}

pub fn flask_label(kind: FlaskKind) -> &'static str {
    match kind {
        FlaskKind::Repair => "Estopa",
        FlaskKind::Wind => "Vento",
        FlaskKind::Fury => "Fúria",
        FlaskKind::Tar => "Breu",
    }
}

fn bit(mask: u8, kind: FlaskKind) -> bool {
    mask & (1 << kind.index()) != 0
}

/// Por que o frasco não sai (ou `None`: pode beber).
fn refusal(flasks: &FlaskWire, kind: FlaskKind) -> Option<&'static str> {
    if !bit(flasks.aboard, kind) {
        Some("sem esse frasco no porão")
    } else if bit(flasks.active, kind) {
        Some("já está fazendo efeito")
    } else if flasks.charges[kind.index()] < DOSE {
        Some("frasco vazio: acerte canhão ou atraque")
    } else {
        None
    }
}

// ── Cinto no HUD ─────────────────────────────────────────────────────────

#[derive(Component, Clone, Copy)]
struct FlaskSlot(FlaskKind);

/// Recorte que sobe e desce: mostra o frasco cheio só até a altura da carga.
#[derive(Component, Clone, Copy)]
struct FlaskLiquid(FlaskKind);

#[derive(Component, Clone, Copy)]
struct FlaskPip(FlaskKind, u8);

/// Tremida de recusa.
#[derive(Component)]
struct FlaskShake(f32);

const ICON_PX: f32 = 40.0;
const SLOT_BG: Color = Color::srgba(0.086, 0.125, 0.169, 0.10);

/// Cinto de frascos: chamado pelo HUD dentro da coluna da base.
pub fn spawn_belt(parent: &mut ChildBuilder, atlas: Option<&ItemIcons>) {
    parent
        .spawn((
            ui::panel(Node {
                column_gap: Val::Px(10.0),
                padding: UiRect::axes(Val::Px(12.0), Val::Px(6.0)),
                align_items: AlignItems::Center,
                ..default()
            }),
            FlaskBeltPanel,
        ))
        .with_children(|belt| {
            for (kind, key) in FlaskKind::ALL.into_iter().zip(KEYS) {
                spawn_slot(belt, kind, key, atlas);
            }
        });
}

/// Painel do cinto (o atlas carrega depois do HUD: os ícones entram quando
/// ele existir).
#[derive(Component)]
pub struct FlaskBeltPanel;

fn spawn_slot(parent: &mut ChildBuilder, kind: FlaskKind, key: KeyCode, atlas: Option<&ItemIcons>) {
    parent
        .spawn((
            Node {
                flex_direction: FlexDirection::Column,
                align_items: AlignItems::Center,
                row_gap: Val::Px(2.0),
                padding: UiRect::all(Val::Px(3.0)),
                border: UiRect::all(Val::Px(2.0)),
                ..default()
            },
            BackgroundColor(SLOT_BG),
            BorderColor(Color::NONE),
            BorderRadius::all(Val::Px(5.0)),
            FlaskSlot(kind),
        ))
        .with_children(|slot| {
            slot.spawn(Node {
                width: Val::Px(ICON_PX),
                height: Val::Px(ICON_PX),
                ..default()
            })
            .with_children(|icon| {
                if let Some(atlas) = atlas {
                    icon.spawn((
                        Node {
                            position_type: PositionType::Absolute,
                            width: Val::Px(ICON_PX),
                            height: Val::Px(ICON_PX),
                            ..default()
                        },
                        atlas.node(icons::EMPTY_FLASK),
                    ));
                    icon.spawn((
                        Node {
                            position_type: PositionType::Absolute,
                            bottom: Val::Px(0.0),
                            width: Val::Px(ICON_PX),
                            height: Val::Percent(100.0),
                            overflow: Overflow::clip(),
                            ..default()
                        },
                        FlaskLiquid(kind),
                    ))
                    .with_children(|clip| {
                        clip.spawn((
                            Node {
                                position_type: PositionType::Absolute,
                                bottom: Val::Px(0.0),
                                width: Val::Px(ICON_PX),
                                height: Val::Px(ICON_PX),
                                ..default()
                            },
                            atlas.node(icons::flask(kind)),
                        ));
                    });
                }
            });
            slot.spawn(Node {
                column_gap: Val::Px(3.0),
                ..default()
            })
            .with_children(|pips| {
                for dose in 0..MAX_CHARGES / DOSE {
                    pips.spawn((
                        Node {
                            width: Val::Px(7.0),
                            height: Val::Px(7.0),
                            ..default()
                        },
                        BackgroundColor(flask_color(kind)),
                        BorderRadius::MAX,
                        FlaskPip(kind, dose),
                    ));
                }
            });
            slot.spawn(Node {
                align_items: AlignItems::Center,
                column_gap: Val::Px(4.0),
                ..default()
            })
            .with_children(|row| {
                row.spawn((Node::default(), KeySlot(key)));
                row.spawn(crate::i18n::label_face(
                    flask_label(kind),
                    ui::FONT_BOLD,
                    11.0,
                    ui::INK_SOFT,
                ));
            });
        });
}

/// O HUD nasce antes do atlas carregar: remonta o cinto quando ele chega.
fn rebuild_belt_with_icons(
    mut commands: Commands,
    atlas: Option<Res<ItemIcons>>,
    belts: Query<Entity, With<FlaskBeltPanel>>,
    liquids: Query<(), With<FlaskLiquid>>,
) {
    let Some(atlas) = atlas else { return };
    if !liquids.is_empty() {
        return;
    }
    for belt in &belts {
        commands
            .entity(belt)
            .despawn_descendants()
            .with_children(|belt| {
                for (kind, key) in FlaskKind::ALL.into_iter().zip(KEYS) {
                    spawn_slot(belt, kind, key, Some(&atlas));
                }
            });
    }
}

fn my_flasks<'a>(my_ship: &MyShip, visuals: &'a Query<&ShipVisual>) -> Option<&'a FlaskWire> {
    let me = my_ship.0?;
    visuals
        .iter()
        .find(|visual| visual.target.ship_id == me)
        .map(|visual| &visual.target.flasks)
}

/// v54: sem frasco a bordo o cinto some (quatro vidros vazios eram ruído
/// na tela). Volta enquanto um vidro treme: a tecla apertada à toa ainda
/// dá o recado.
fn toggle_belt(
    my_ship: Res<MyShip>,
    visuals: Query<&ShipVisual>,
    shakes: Query<(), With<FlaskShake>>,
    mut panels: Query<&mut Node, With<FlaskBeltPanel>>,
) {
    let aboard = my_flasks(&my_ship, &visuals).is_some_and(|flasks| flasks.aboard != 0);
    let display = if aboard || !shakes.is_empty() {
        Display::Flex
    } else {
        Display::None
    };
    for mut node in &mut panels {
        if node.display != display {
            node.display = display;
        }
    }
}

#[allow(clippy::type_complexity, clippy::too_many_arguments)]
fn update_belt(
    time: Res<Time>,
    my_ship: Res<MyShip>,
    visuals: Query<&ShipVisual>,
    mut slots: Query<
        (
            &FlaskSlot,
            &mut BackgroundColor,
            &mut BorderColor,
            &mut Node,
        ),
        Without<FlaskLiquid>,
    >,
    mut liquids: Query<(&FlaskLiquid, &mut Node), Without<FlaskSlot>>,
    mut pips: Query<(&FlaskPip, &mut BackgroundColor), Without<FlaskSlot>>,
    mut shakes: Query<(Entity, &mut FlaskShake)>,
    mut commands: Commands,
) {
    let flasks = my_flasks(&my_ship, &visuals).copied().unwrap_or_default();
    let pulse = 0.5 + 0.5 * (time.elapsed_secs() * 7.0).sin();
    for (slot, mut bg, mut border, mut node) in &mut slots {
        let kind = slot.0;
        let aboard = bit(flasks.aboard, kind);
        let active = bit(flasks.active, kind);
        bg.0 = if active {
            flask_color(kind).with_alpha(0.25 + 0.2 * pulse)
        } else if aboard {
            SLOT_BG
        } else {
            SLOT_BG.with_alpha(0.03)
        };
        border.0 = if active {
            flask_color(kind)
        } else {
            Color::NONE
        };
        node.left = Val::Px(0.0);
    }
    for (entity, mut shake) in &mut shakes {
        shake.0 += time.delta_secs();
        if shake.0 > 0.35 {
            commands.entity(entity).remove::<FlaskShake>();
            continue;
        }
        if let Ok((_, mut bg, mut border, mut node)) = slots.get_mut(entity) {
            node.left = Val::Px((shake.0 * 60.0).sin() * 5.0 * (1.0 - shake.0 / 0.35));
            border.0 = ui::DANGER;
            bg.0 = ui::DANGER.with_alpha(0.25);
        }
    }
    for (liquid, mut node) in &mut liquids {
        let charges = flasks.charges[liquid.0.index()];
        let fill = if bit(flasks.aboard, liquid.0) {
            f32::from(charges) / f32::from(MAX_CHARGES)
        } else {
            0.0
        };
        node.height = Val::Percent(fill * 100.0);
    }
    for (pip, mut bg) in &mut pips {
        let doses = flasks.charges[pip.0.index()] / DOSE;
        let lit = bit(flasks.aboard, pip.0) && pip.1 < doses;
        bg.0 = if lit {
            flask_color(pip.0)
        } else {
            ui::INK.with_alpha(0.15)
        };
    }
}

// ── Teclas ───────────────────────────────────────────────────────────────

#[allow(clippy::too_many_arguments)]
fn send_flask_input(
    keys: Res<ButtonInput<KeyCode>>,
    docked: Res<MyDocked>,
    my_ship: Res<MyShip>,
    visuals: Query<&ShipVisual>,
    slots: Query<(Entity, &FlaskSlot)>,
    mut commands: Commands,
    mut notices: EventWriter<crate::net::PlayerNotice>,
    mut connection_manager: ResMut<ConnectionManager>,
    melee: Res<crate::melee::MeleeView>,
) {
    // Atracado os números são da oficina (crafting.rs); no duelo de
    // abordagem, 1-3 são táticas (melee.rs).
    if docked.0 || melee.active() {
        return;
    }
    let Some(kind) = FlaskKind::ALL
        .into_iter()
        .zip(KEYS)
        .find(|(_, key)| keys.just_pressed(*key))
        .map(|(kind, _)| kind)
    else {
        return;
    };
    let Some(flasks) = my_flasks(&my_ship, &visuals) else {
        return;
    };
    if let Some(reason) = refusal(flasks, kind) {
        notices.send(crate::net::PlayerNotice(String::from(reason)));
        if let Some((entity, _)) = slots.iter().find(|(_, slot)| slot.0 == kind) {
            commands.entity(entity).insert(FlaskShake(0.0));
        }
        return;
    }
    info!(?kind, "bebendo frasco");
    let _ = connection_manager.send_message::<ReliableChannel, _>(&UseFlask { kind });
}

/// Dev (§39): MARVYR_AUTOFLASK=<s> saca o armazém uma vez ao atracar (os
/// frascos vão para o porão) e, no mar, bebe os quatro em sequência, um a
/// cada <s> segundos — capturas sem teclado.
fn autoflask_system(
    time: Res<Time>,
    docked: Res<MyDocked>,
    mut timer: Local<f32>,
    mut next: Local<usize>,
    mut withdrawn: Local<bool>,
    mut connection_manager: ResMut<ConnectionManager>,
) {
    let Some(every) = std::env::var("MARVYR_AUTOFLASK")
        .ok()
        .and_then(|secs| secs.parse::<f32>().ok())
    else {
        return;
    };
    if docked.0 {
        if !*withdrawn {
            *withdrawn = true;
            let _ = connection_manager
                .send_message::<ReliableChannel, _>(&marvyr_protocol::StorageWithdrawAll);
        }
        return;
    }
    *timer += time.delta_secs();
    if *timer < every {
        return;
    }
    *timer = 0.0;
    let kind = FlaskKind::ALL[*next % FlaskKind::ALL.len()];
    *next += 1;
    let _ = connection_manager.send_message::<ReliableChannel, _>(&UseFlask { kind });
}

// ── Efeitos ──────────────────────────────────────────────────────────────

/// Frasco que acabou de ligar (qualquer navio): som se for o meu, e a
/// explosão de cor em volta do casco.
#[derive(Event, Debug, Clone, Copy)]
pub struct FlaskDrunk {
    pub kind: FlaskKind,
    pub mine: bool,
}

fn detect_drinks(
    my_ship: Res<MyShip>,
    visuals: Query<(&ShipVisual, &Transform)>,
    mut seen: Local<HashMap<u32, u8>>,
    mut drunk: EventWriter<FlaskDrunk>,
    mut commands: Commands,
) {
    for (visual, transform) in &visuals {
        let id = visual.target.ship_id;
        let active = visual.target.flasks.active;
        let before = seen.insert(id, active);
        let Some(before) = before else { continue };
        for kind in FlaskKind::ALL {
            if bit(active, kind) && !bit(before, kind) {
                let mine = my_ship.0 == Some(id);
                drunk.send(FlaskDrunk { kind, mine });
                burst(&mut commands, transform.translation.truncate(), kind);
            }
        }
    }
    if seen.len() > visuals.iter().len() * 2 + 64 {
        seen.retain(|id, _| visuals.iter().any(|(v, _)| v.target.ship_id == *id));
    }
}

/// Anel de gotas na cor do frasco que abre do casco.
fn burst(commands: &mut Commands, at: Vec2, kind: FlaskKind) {
    let color = flask_color(kind);
    for i in 0..40 {
        let angle = i as f32 / 40.0 * std::f32::consts::TAU;
        let speed = 55.0 + (i % 4) as f32 * 14.0;
        spawn_particle(
            commands,
            at + Vec2::from_angle(angle) * 10.0,
            Particle {
                velocity: Vec2::from_angle(angle) * speed,
                drag: 2.2,
                life: 1.0,
                age: 0.0,
                size: (6.0, 2.5),
                color: if i % 5 == 0 { Color::WHITE } else { color },
                z: layers::VFX,
            },
        );
    }
}

/// Enquanto o efeito dura, o casco solta a assinatura do frasco: brasas na
/// Fúria, esteira branca no Vento, fumaça de piche no Breu, faíscas de
/// remendo na Estopa. Todo mundo vê — frasco ligado é informação de combate.
fn emit_active(
    time: Res<Time>,
    mut commands: Commands,
    visuals: Query<(&ShipVisual, &Transform)>,
    mut clock: Local<f32>,
    mut seed: Local<u32>,
) {
    *clock += time.delta_secs();
    if *clock < 0.07 {
        return;
    }
    *clock = 0.0;
    for (visual, transform) in &visuals {
        let active = visual.target.flasks.active;
        if active == 0 {
            continue;
        }
        let at = transform.translation.truncate();
        let heading = Vec2::from_angle(visual.target.heading);
        // Duas partículas por frasco a cada batida: rastro contínuo.
        let lit = FlaskKind::ALL.into_iter().filter(|kind| bit(active, *kind));
        for kind in lit.flat_map(|kind| [kind, kind]) {
            *seed = seed.wrapping_mul(1_103_515_245).wrapping_add(12_345);
            let r = (*seed >> 8) as f32 / (1u32 << 24) as f32;
            let jitter = Vec2::from_angle(r * std::f32::consts::TAU) * (8.0 + 14.0 * r);
            let particle = match kind {
                FlaskKind::Repair => Particle {
                    velocity: Vec2::new(0.0, 22.0),
                    drag: 0.5,
                    life: 1.0,
                    age: 0.0,
                    size: (4.5, 2.5),
                    color: if r > 0.6 {
                        Color::WHITE
                    } else {
                        Color::srgb(1.0, 0.3, 0.35)
                    },
                    z: layers::VFX,
                },
                FlaskKind::Wind => Particle {
                    velocity: -heading * 70.0,
                    drag: 1.0,
                    life: 0.7,
                    age: 0.0,
                    size: (3.5, 5.5),
                    color: Color::WHITE,
                    z: layers::VFX,
                },
                FlaskKind::Fury => Particle {
                    velocity: Vec2::new(jitter.x * 0.6, 22.0),
                    drag: 1.5,
                    life: 0.7,
                    age: 0.0,
                    size: (4.0, 1.5),
                    color: if r > 0.5 {
                        Color::srgb(1.0, 0.55, 0.1)
                    } else {
                        Color::srgb(1.0, 0.85, 0.3)
                    },
                    z: layers::VFX,
                },
                FlaskKind::Tar => Particle {
                    velocity: jitter * 0.4,
                    drag: 1.2,
                    life: 1.1,
                    age: 0.0,
                    size: (5.0, 11.0),
                    color: Color::srgba(0.22, 0.10, 0.30, 0.85),
                    z: layers::VFX,
                },
            };
            spawn_particle(&mut commands, at + jitter, particle);
        }
    }
}

pub struct FlasksPlugin;

impl Plugin for FlasksPlugin {
    fn build(&self, app: &mut App) {
        app.add_event::<FlaskDrunk>().add_systems(
            Update,
            (
                send_flask_input,
                autoflask_system,
                rebuild_belt_with_icons,
                update_belt,
                toggle_belt,
                detect_drinks,
                emit_active,
            ),
        );
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn refusal_reads_the_server_state() {
        let mut flasks = FlaskWire {
            charges: [MAX_CHARGES, DOSE - 1, MAX_CHARGES, MAX_CHARGES],
            active: 0b0100,
            aboard: 0b0111,
        };
        assert_eq!(refusal(&flasks, FlaskKind::Repair), None);
        assert!(refusal(&flasks, FlaskKind::Wind).is_some(), "sem dose");
        assert!(refusal(&flasks, FlaskKind::Fury).is_some(), "já ligado");
        assert!(
            refusal(&flasks, FlaskKind::Tar).is_some(),
            "não está a bordo"
        );
        flasks.aboard = 0b1111;
        assert_eq!(refusal(&flasks, FlaskKind::Tar), None);
    }
}
