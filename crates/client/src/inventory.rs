//! Porão e armazém em grade (v28): cada pilha é um quadro com a moldura da
//! raridade, o ícone e a quantidade; o mouse em cima mostra tudo da peça.
//! Arraste de uma grade para a outra (ou botão direito) para mover aquela
//! pilha. O servidor move; a tela só pede e festeja.

use bevy::prelude::*;
use bevy::ui::RelativeCursorPosition;
use bevy::window::PrimaryWindow;
use lightyear::prelude::client::*;
use marvyr_domain_items::{Quality, Rarity};
use marvyr_protocol::{
    StorageDeposit, StorageDepositAll, StorageLine, StorageWithdraw, StorageWithdrawAll,
    StoredElsewhere,
};
use marvyr_shared::ids::{ItemDefinitionId, ItemInstanceId};

use crate::affixes::{affix_summary, badge_frame, piece_name, quality_rarity, rarity_color};
use crate::assets::{icons, ItemIcons};
use crate::i18n::{tr, trf};
use crate::net::{MyDocked, ReliableChannel};
use crate::port_screen::{PortScreenState, PortTab};
use crate::ui;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Side {
    Hold,
    Storage,
}

#[derive(Debug, Clone, PartialEq)]
pub struct CellView {
    item: ItemDefinitionId,
    instance: Option<ItemInstanceId>,
    name: String,
    quantity: u32,
    quality: Option<Quality>,
}

impl CellView {
    fn from_line(line: &StorageLine) -> Self {
        Self {
            item: line.item,
            instance: line.instance,
            name: line.item_name.clone(),
            quantity: line.quantity,
            quality: line.quality.clone(),
        }
    }

    fn rarity(&self) -> Rarity {
        quality_rarity(self.quality.as_ref())
    }
}

/// O que a aba Porão mostra; comparado a cada quadro para remontar.
#[derive(Debug, Clone, PartialEq)]
pub struct InventoryView {
    hold: Vec<CellView>,
    storage: Vec<CellView>,
    weight: Option<(u32, u32)>,
    elsewhere: Vec<(String, u32)>,
}

pub fn inventory_view(
    hold: &[StorageLine],
    storage: &[StorageLine],
    weight: Option<(u32, u32)>,
    elsewhere: &[StoredElsewhere],
) -> InventoryView {
    InventoryView {
        hold: hold.iter().map(CellView::from_line).collect(),
        storage: storage.iter().map(CellView::from_line).collect(),
        weight,
        elsewhere: elsewhere
            .iter()
            .map(|stored| (stored.region.clone(), stored.quantity))
            .collect(),
    }
}

/// Quadro da grade (origem de arrastar, alvo de dica).
#[derive(Component, Debug, Clone)]
struct ItemCell {
    side: Side,
    cell: CellView,
}

/// Área de uma grade: soltar aqui move para este lado.
#[derive(Component, Clone, Copy)]
pub(crate) struct GridArea(Side);

/// Botão "tudo" (guardar ou levar).
#[derive(Component, Clone, Copy)]
struct MoveAllButton(Side);

const CELL_PX: f32 = 60.0;
const SCALE: f32 = 1.6;

pub fn spawn_inventory_body(
    parent: &mut ChildBuilder,
    view: &InventoryView,
    atlas: Option<&ItemIcons>,
) {
    let weight = view.weight.map_or_else(
        || String::from("—"),
        |(used, capacity)| format!("{used} / {capacity}"),
    );
    parent
        .spawn(Node {
            column_gap: Val::Px(16.0),
            flex_grow: 1.0,
            min_height: Val::Px(0.0),
            ..default()
        })
        .with_children(|row| {
            spawn_grid(
                row,
                Side::Hold,
                trf("PORÃO · {0}", &[&weight]),
                &view.hold,
                "Porão vazio.",
                atlas,
            );
            spawn_grid(
                row,
                Side::Storage,
                tr("ARMAZÉM DESTE PORTO"),
                &view.storage,
                "Nada guardado aqui.",
                atlas,
            );
        });
    if !view.elsewhere.is_empty() {
        let others: Vec<String> = view
            .elsewhere
            .iter()
            .map(|(region, quantity)| trf("{0}: {1} itens", &[&tr(region), &quantity.to_string()]))
            .collect();
        parent.spawn(ui::text(
            format!(
                "{} {}",
                tr("Guardado em outros portos:"),
                others.join(" · ")
            ),
            12.0,
            ui::TEXT_DIM,
        ));
    }
    parent.spawn(crate::i18n::label(
        "Arraste entre porão e armazém · botão direito move direto · orbe em cima da peça a transforma",
        12.0,
        ui::TEXT_DIM,
    ));
}

fn spawn_grid(
    parent: &mut ChildBuilder,
    side: Side,
    title: String,
    cells: &[CellView],
    empty: &'static str,
    atlas: Option<&ItemIcons>,
) {
    parent
        .spawn((
            Node {
                flex_direction: FlexDirection::Column,
                flex_grow: 1.0,
                flex_basis: Val::Px(0.0),
                row_gap: Val::Px(6.0),
                padding: UiRect::all(Val::Px(8.0)),
                border: UiRect::all(Val::Px(1.0)),
                overflow: Overflow::clip_y(),
                ..default()
            },
            BorderColor(ui::PANEL_BORDER.with_alpha(0.35)),
            BorderRadius::all(Val::Px(6.0)),
            BackgroundColor(ui::PAPER_SHADE.with_alpha(0.5)),
            RelativeCursorPosition::default(),
            GridArea(side),
        ))
        .with_children(|panel| {
            panel
                .spawn(Node {
                    justify_content: JustifyContent::SpaceBetween,
                    align_items: AlignItems::Center,
                    ..default()
                })
                .with_children(|head| {
                    head.spawn(ui::text(title, 13.0, ui::TEXT_DIM));
                    let label = match side {
                        Side::Hold => "Guardar tudo",
                        Side::Storage => "Levar tudo",
                    };
                    head.spawn((
                        ui::button(Node::default(), ui::BUTTON_BG),
                        MoveAllButton(side),
                    ))
                    .with_children(|b| {
                        b.spawn(crate::i18n::label(label, 12.0, ui::TEXT));
                    });
                });
            if cells.is_empty() {
                panel.spawn(crate::i18n::label(empty, 13.0, ui::TEXT));
            }
            panel
                .spawn(Node {
                    flex_wrap: FlexWrap::Wrap,
                    column_gap: Val::Px(6.0),
                    row_gap: Val::Px(6.0),
                    ..default()
                })
                .with_children(|grid| {
                    for cell in cells {
                        spawn_cell(grid, side, cell, atlas);
                    }
                });
        });
}

fn spawn_cell(parent: &mut ChildBuilder, side: Side, cell: &CellView, atlas: Option<&ItemIcons>) {
    let rarity = cell.rarity();
    parent
        .spawn((
            Button,
            Node {
                width: Val::Px(CELL_PX),
                height: Val::Px(CELL_PX),
                justify_content: JustifyContent::Center,
                align_items: AlignItems::Center,
                ..default()
            },
            BackgroundColor(Color::NONE),
            BorderRadius::all(Val::Px(6.0)),
            RelativeCursorPosition::default(),
            ItemCell {
                side,
                cell: cell.clone(),
            },
        ))
        .with_children(|slot| {
            match (atlas, icons::item(&cell.name)) {
                (Some(atlas), Some(icon)) => {
                    slot.spawn((badge_frame(atlas, rarity, SCALE), PickingBehavior::IGNORE))
                        .with_children(|frame| {
                            frame.spawn((
                                Node {
                                    width: Val::Px(24.0 * SCALE),
                                    height: Val::Px(24.0 * SCALE),
                                    ..default()
                                },
                                atlas.node(icon),
                                PickingBehavior::IGNORE,
                            ));
                        });
                }
                // Sem arte: a inicial do nome no quadro da raridade.
                _ => {
                    slot.spawn((
                        Node {
                            width: Val::Px(CELL_PX - 6.0),
                            height: Val::Px(CELL_PX - 6.0),
                            justify_content: JustifyContent::Center,
                            align_items: AlignItems::Center,
                            border: UiRect::all(Val::Px(2.0)),
                            ..default()
                        },
                        BorderColor(rarity_color(rarity)),
                        BackgroundColor(SLOT_BG),
                        PickingBehavior::IGNORE,
                    ))
                    .with_children(|inner| {
                        let initial: String = tr(&cell.name).chars().take(2).collect();
                        inner.spawn(ui::text(initial, 18.0, ui::TEXT));
                    });
                }
            }
            if cell.quantity > 1 {
                slot.spawn((
                    Node {
                        position_type: PositionType::Absolute,
                        right: Val::Px(2.0),
                        bottom: Val::Px(0.0),
                        padding: UiRect::axes(Val::Px(3.0), Val::Px(0.0)),
                        ..default()
                    },
                    BackgroundColor(ui::INK.with_alpha(0.75)),
                    BorderRadius::all(Val::Px(3.0)),
                    PickingBehavior::IGNORE,
                ))
                .with_children(|badge| {
                    badge.spawn(ui::text(cell.quantity.to_string(), 12.0, ui::PAPER));
                });
            }
        });
}

const SLOT_BG: Color = Color::srgba(0.086, 0.125, 0.169, 0.12);

// ── Dica, arrastar e festa ───────────────────────────────────────────────

/// Camada de cima da grade: dica e fantasma arrastado.
#[derive(Component)]
struct InvLayer;

#[derive(Component)]
struct Tooltip;

#[derive(Component)]
struct TooltipText;

#[derive(Component)]
struct InvGhost;

#[derive(Resource, Default)]
struct InvDrag {
    from: Option<(Side, CellView)>,
    ghost: Option<Entity>,
    /// Onde a última pilha foi solta (a festa nasce ali quando o servidor
    /// confirmar).
    dropped_at: Option<(Vec2, Rarity)>,
}

fn spawn_layer(mut commands: Commands) {
    commands
        .spawn((
            Node {
                position_type: PositionType::Absolute,
                width: Val::Percent(100.0),
                height: Val::Percent(100.0),
                ..default()
            },
            GlobalZIndex(41),
            PickingBehavior::IGNORE,
            InvLayer,
        ))
        .with_children(|layer| {
            layer
                .spawn((
                    ui::panel(Node {
                        position_type: PositionType::Absolute,
                        display: Display::None,
                        flex_direction: FlexDirection::Column,
                        padding: UiRect::axes(Val::Px(10.0), Val::Px(6.0)),
                        max_width: Val::Px(320.0),
                        ..default()
                    }),
                    PickingBehavior::IGNORE,
                    Tooltip,
                ))
                .with_children(|tip| {
                    tip.spawn((ui::text("", 13.0, ui::TEXT), TooltipText));
                });
        });
}

fn cursor_ui(windows: &Query<&Window, With<PrimaryWindow>>, scale: &UiScale) -> Option<Vec2> {
    windows
        .get_single()
        .ok()?
        .cursor_position()
        .map(|at| at / scale.0)
}

fn on_tab(docked: &MyDocked, state: &PortScreenState) -> bool {
    docked.0 && state.active_tab == PortTab::Storage
}

/// Texto da dica: nome (com raridade), quantidade e o que a peça carrega.
fn tooltip_lines(cell: &CellView) -> String {
    let quality = cell.quality.as_ref();
    let mut lines = vec![format!(
        "{} ×{}",
        piece_name(&cell.name, quality),
        cell.quantity
    )];
    let summary = affix_summary(quality);
    if !summary.is_empty() {
        lines.push(summary);
    }
    if let Some(quality) = quality.filter(|q| !q.gems.is_empty()) {
        let gems: Vec<String> = quality.gems.iter().map(|g| tr(g.item_name())).collect();
        lines.push(trf("Gemas: {0}", &[&gems.join(", ")]));
    }
    if let Some(line) = crate::affixes::aspect_line(quality) {
        lines.push(line);
    }
    lines.join("\n")
}

#[allow(clippy::too_many_arguments, clippy::type_complexity)]
fn show_tooltip(
    docked: Res<MyDocked>,
    state: Res<PortScreenState>,
    drag: Res<InvDrag>,
    windows: Query<&Window, With<PrimaryWindow>>,
    scale: Res<UiScale>,
    cells: Query<(&Interaction, &ItemCell)>,
    mut tips: Query<(&mut Node, &mut BorderColor), With<Tooltip>>,
    mut texts: Query<(&mut Text, &mut TextColor), With<TooltipText>>,
) {
    let hovered = cells
        .iter()
        .find(|(interaction, _)| **interaction != Interaction::None)
        .map(|(_, cell)| cell);
    let Ok((mut node, mut border)) = tips.get_single_mut() else {
        return;
    };
    let (Some(cell), Some(at), true, None) = (
        hovered,
        cursor_ui(&windows, &scale),
        on_tab(&docked, &state),
        drag.from.as_ref(),
    ) else {
        node.display = Display::None;
        return;
    };
    node.display = Display::Flex;
    node.left = Val::Px(at.x + 18.0);
    node.top = Val::Px(at.y + 12.0);
    border.0 = rarity_color(cell.cell.rarity());
    if let Ok((mut text, mut color)) = texts.get_single_mut() {
        let value = tooltip_lines(&cell.cell);
        if text.0 != value {
            text.0 = value;
        }
        color.0 = rarity_color(cell.cell.rarity());
    }
}

fn send_move(connection_manager: &mut ConnectionManager, from: Side, cell: &CellView) {
    let _ = match from {
        Side::Hold => connection_manager.send_message::<ReliableChannel, _>(&StorageDeposit {
            item: cell.item,
            instance: cell.instance,
        }),
        Side::Storage => connection_manager.send_message::<ReliableChannel, _>(&StorageWithdraw {
            item: cell.item,
            instance: cell.instance,
        }),
    };
}

#[allow(clippy::too_many_arguments)]
fn drag_items(
    mut commands: Commands,
    mouse: Res<ButtonInput<MouseButton>>,
    docked: Res<MyDocked>,
    state: Res<PortScreenState>,
    windows: Query<&Window, With<PrimaryWindow>>,
    scale: Res<UiScale>,
    mut drag: ResMut<InvDrag>,
    cells: Query<(&Interaction, &ItemCell)>,
    targets: Query<(&RelativeCursorPosition, &ItemCell)>,
    areas: Query<(&RelativeCursorPosition, &GridArea)>,
    all_buttons: Query<(&Interaction, &MoveAllButton), Changed<Interaction>>,
    mut ghosts: Query<&mut Node, With<InvGhost>>,
    layer: Query<Entity, With<InvLayer>>,
    atlas: Option<Res<ItemIcons>>,
    mut connection_manager: ResMut<ConnectionManager>,
) {
    let cursor = cursor_ui(&windows, &scale);
    let active = on_tab(&docked, &state);
    if active {
        for (interaction, button) in &all_buttons {
            if *interaction == Interaction::Pressed {
                let _ =
                    match button.0 {
                        Side::Hold => connection_manager
                            .send_message::<ReliableChannel, _>(&StorageDepositAll),
                        Side::Storage => connection_manager
                            .send_message::<ReliableChannel, _>(&StorageWithdrawAll),
                    };
            }
        }
    }
    let hovered = cells
        .iter()
        .find(|(interaction, _)| **interaction != Interaction::None)
        .map(|(_, cell)| cell.clone());
    // Botão direito: move direto para o outro lado.
    if active && mouse.just_pressed(MouseButton::Right) {
        if let Some(cell) = &hovered {
            send_move(&mut connection_manager, cell.side, &cell.cell);
            drag.dropped_at = cursor.map(|at| (at, cell.cell.rarity()));
        }
    }
    if active && mouse.just_pressed(MouseButton::Left) {
        if let (Some(cell), Ok(layer)) = (&hovered, layer.get_single()) {
            drag.from = Some((cell.side, cell.cell.clone()));
            if let (Some(atlas), Some(icon)) = (atlas.as_ref(), icons::item(&cell.cell.name)) {
                let ghost = commands
                    .spawn((
                        Node {
                            position_type: PositionType::Absolute,
                            width: Val::Px(48.0),
                            height: Val::Px(48.0),
                            ..default()
                        },
                        atlas.node(icon),
                        PickingBehavior::IGNORE,
                        InvGhost,
                    ))
                    .set_parent(layer)
                    .id();
                drag.ghost = Some(ghost);
            }
        }
    }
    if let (Some(at), Ok(mut node)) = (cursor, ghosts.get_single_mut()) {
        node.left = Val::Px(at.x - 24.0);
        node.top = Val::Px(at.y - 24.0);
    }
    if !(mouse.just_released(MouseButton::Left) || !active) {
        return;
    }
    if let Some(ghost) = drag.ghost.take() {
        commands.entity(ghost).despawn_recursive();
    }
    let Some((from, cell)) = drag.from.take() else {
        return;
    };
    // Orbe solto em cima de peça do armazém: gasta o orbe nela.
    let orb = marvyr_domain_items::OrbKind::from_item(cell.item);
    let under = targets
        .iter()
        .find(|(cursor, _)| cursor.mouse_over())
        .map(|(_, target)| target.clone());
    if let (true, Some(orb), Some(under)) = (active, orb, under) {
        if let (Side::Storage, Some(instance)) = (under.side, under.cell.instance) {
            let _ =
                connection_manager.send_message::<ReliableChannel, _>(&marvyr_protocol::ApplyOrb {
                    orb,
                    target: instance,
                });
            return;
        }
    }
    let target = areas
        .iter()
        .find(|(cursor, _)| cursor.mouse_over())
        .map(|(_, area)| area.0);
    if active && target.is_some_and(|side| side != from) {
        send_move(&mut connection_manager, from, &cell);
        drag.dropped_at = cursor.map(|at| (at, cell.rarity()));
    }
}

/// Partícula da festa de mover (quadrado que voa e some).
#[derive(Component)]
struct MoveSpark {
    origin: Vec2,
    velocity: Vec2,
    age: f32,
}

/// Servidor confirmou a mudança: faíscas na cor da raridade onde a pilha
/// foi solta (o som de negócio fechado já toca em `audio.rs`).
fn celebrate_move(
    mut commands: Commands,
    feedback: Res<crate::market::MarketFeedback>,
    mut drag: ResMut<InvDrag>,
    layer: Query<Entity, With<InvLayer>>,
) {
    if !feedback.is_changed() {
        return;
    }
    let Some((at, rarity)) = drag.dropped_at.take() else {
        return;
    };
    let ok = feedback.0.as_ref().is_some_and(|result| result.success);
    let (Ok(layer), true) = (layer.get_single(), ok) else {
        return;
    };
    let color = match rarity {
        Rarity::Normal => ui::BRASS,
        rarity => rarity_color(rarity),
    };
    for i in 0..16 {
        let angle = i as f32 / 16.0 * std::f32::consts::TAU;
        commands
            .spawn((
                Node {
                    position_type: PositionType::Absolute,
                    width: Val::Px(6.0),
                    height: Val::Px(6.0),
                    ..default()
                },
                BackgroundColor(if i % 4 == 0 { Color::WHITE } else { color }),
                MoveSpark {
                    origin: at,
                    velocity: Vec2::from_angle(angle) * (90.0 + (i % 3) as f32 * 30.0),
                    age: 0.0,
                },
                PickingBehavior::IGNORE,
            ))
            .set_parent(layer);
    }
}

/// Com orbe na mão, as peças do armazém que podem recebê-lo pulsam.
fn paint_orb_targets(
    time: Res<Time>,
    drag: Res<InvDrag>,
    mut cells: Query<(&ItemCell, &RelativeCursorPosition, &mut BackgroundColor)>,
) {
    let holding_orb = drag
        .from
        .as_ref()
        .is_some_and(|(_, cell)| marvyr_domain_items::OrbKind::from_item(cell.item).is_some());
    let pulse = 0.5 + 0.5 * (time.elapsed_secs() * 6.0).sin();
    for (cell, cursor, mut bg) in &mut cells {
        let target = holding_orb && cell.side == Side::Storage && cell.cell.instance.is_some();
        let color = match (target, cursor.mouse_over()) {
            (true, true) => ui::BRASS,
            (true, false) => ui::BRASS.with_alpha(0.2 + 0.35 * pulse),
            _ => Color::NONE,
        };
        if bg.0 != color {
            bg.0 = color;
        }
    }
}

fn animate_sparks(
    mut commands: Commands,
    time: Res<Time>,
    mut sparks: Query<(Entity, &mut MoveSpark, &mut Node, &mut BackgroundColor)>,
) {
    const LIFE: f32 = 0.6;
    for (entity, mut spark, mut node, mut bg) in &mut sparks {
        spark.age += time.delta_secs();
        if spark.age >= LIFE {
            commands.entity(entity).despawn_recursive();
            continue;
        }
        let t = spark.age;
        let at = spark.origin + spark.velocity * t * (1.0 - 0.5 * t);
        node.left = Val::Px(at.x - 3.0);
        node.top = Val::Px(at.y - 3.0);
        bg.0 = bg.0.with_alpha(1.0 - t / LIFE);
    }
}

/// Dev (§39): MARVYR_AUTOORB=1 gasta, 3 s depois de atracar, o primeiro
/// orbe do armazém na primeira peça que der — captura da festa sem mouse.
fn autoorb(
    time: Res<Time>,
    docked: Res<MyDocked>,
    storage: Res<crate::port_screen::KnownPortStorage>,
    mut timer: Local<f32>,
    mut done: Local<bool>,
    mut connection_manager: ResMut<ConnectionManager>,
) {
    if std::env::var_os("MARVYR_AUTOORB").is_none() || !docked.0 || *done {
        return;
    }
    *timer += time.delta_secs();
    if *timer < 3.0 {
        return;
    }
    let orb = storage
        .0
        .iter()
        .find_map(|line| marvyr_domain_items::OrbKind::from_item(line.item));
    let target = storage.0.iter().find_map(|line| line.instance);
    if let (Some(orb), Some(target)) = (orb, target) {
        *done = true;
        let _ = connection_manager
            .send_message::<ReliableChannel, _>(&marvyr_protocol::ApplyOrb { orb, target });
    }
}

pub struct InventoryPlugin;

impl Plugin for InventoryPlugin {
    fn build(&self, app: &mut App) {
        app.init_resource::<InvDrag>()
            .add_systems(Startup, spawn_layer)
            .add_systems(
                Update,
                (
                    (drag_items, show_tooltip).chain(),
                    celebrate_move,
                    animate_sparks,
                    paint_orb_targets,
                    autoorb,
                ),
            );
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn line(name: &str, quantity: u32, quality: Option<Quality>) -> StorageLine {
        StorageLine {
            item: ItemDefinitionId::stable(name),
            item_name: String::from(name),
            quantity,
            instance: None,
            quality,
        }
    }

    #[test]
    fn tooltip_tells_rarity_affixes_and_gems() {
        let quality = Quality {
            rarity: Rarity::Rare,
            affixes: vec![marvyr_domain_items::Affix {
                kind: marvyr_domain_items::AffixKind::Damage,
                value: 5,
            }],
            gems: vec![marvyr_domain_items::GemKind::Ruby],
            map_mods: Vec::new(),
            aspect: None,
        };
        let view = inventory_view(
            &[line("Canhão de Bronze", 1, Some(quality))],
            &[line("Madeira", 40, None)],
            Some((10, 100)),
            &[],
        );
        assert_eq!(
            tooltip_lines(&view.hold[0]),
            "Canhão de Bronze [Raro] ×1\n+5 dano\nGemas: Rubi"
        );
        assert_eq!(tooltip_lines(&view.storage[0]), "Madeira ×40");
    }
}
