//! Gemas de suporte na tela (v24): aba "Gemas" do porto. Arraste a gema da
//! algibeira até um encaixe da peça instalada; arraste para fora (ou botão
//! direito) para tirar. O servidor decide; a tela festeja quando o loadout
//! confirmado muda — faíscas na cor da gema, onda no encaixe e som.

use bevy::prelude::*;
use bevy::ui::RelativeCursorPosition;
use bevy::window::PrimaryWindow;
use lightyear::prelude::client::*;
use marvyr_domain_items::synergy::PAIRS;
use marvyr_domain_items::{EquipmentSlot, GemKind, Rarity, Synergy};
use marvyr_protocol::{LoadoutLine, SocketGem, StorageLine, UnsocketGem};

use crate::affixes::{affix_label, quality_rarity, rarity_color, spawn_badge};
use crate::assets::{icons, ItemIcons};
use crate::i18n::{tr, trf};
use crate::net::{MyDocked, ReliableChannel};
use crate::port_screen::{KnownLoadout, KnownPortStorage, PortScreenState, PortTab};
use crate::ui;

/// Cor viva da gema (mesma paleta do atlas), para faíscas e brilho.
pub fn gem_color(gem: GemKind) -> Color {
    let (r, g, b) = match gem {
        GemKind::Ruby => (232, 60, 70),
        GemKind::Sapphire => (70, 120, 240),
        GemKind::Emerald => (60, 200, 110),
        GemKind::Topaz => (250, 196, 60),
        GemKind::Amethyst => (170, 90, 230),
        GemKind::Diamond => (236, 244, 250),
    };
    Color::srgb_u8(r, g, b)
}

/// "+8 dano · +10% recarga": ganho e custo com o sinal certo. Recarga
/// negativa no afixo é canhão mais lento, que o jogador lê como "+%".
pub fn gem_effects(gem: GemKind) -> String {
    gem.effects()
        .iter()
        .map(signed_label)
        .collect::<Vec<_>>()
        .join(" · ")
}

fn flip_sign(label: &str) -> String {
    match label.chars().next() {
        Some('+') => format!("-{}", &label[1..]),
        Some('-') => format!("+{}", &label[1..]),
        _ => label.to_owned(),
    }
}

#[derive(Debug, Clone, PartialEq)]
pub struct PieceView {
    slot: EquipmentSlot,
    name: String,
    rarity: Rarity,
    sockets: u8,
    gems: Vec<GemKind>,
    synergies: Vec<Synergy>,
}

/// O que a aba mostra; comparado a cada quadro para remontar.
#[derive(Debug, Clone, PartialEq)]
pub struct GemsView {
    pieces: Vec<PieceView>,
    pouch: Vec<(GemKind, u32)>,
    sets: Vec<Synergy>,
}

/// v27: conjuntos acesos no navio (o servidor manda no loadout).
#[derive(Resource, Debug, Default)]
pub struct KnownGemSets(pub Vec<Synergy>);

fn receive_gem_sets(
    mut events: EventReader<
        lightyear::prelude::ClientReceiveMessage<marvyr_protocol::LoadoutSnapshot>,
    >,
    mut known: ResMut<KnownGemSets>,
) {
    for event in events.read() {
        known.0 = event.message().sets.clone();
    }
}

/// "Salva Contínua: -8% recarga" — nome e o que ela soma.
pub fn synergy_line(synergy: Synergy) -> String {
    let bonus: Vec<String> = synergy.bonus().iter().map(signed_label).collect();
    format!("{}: {}", tr(&synergy.name()), bonus.join(" · "))
}

fn signed_label(affix: &marvyr_domain_items::Affix) -> String {
    let label = affix_label(&marvyr_domain_items::Affix {
        kind: affix.kind,
        value: affix.value.abs(),
    });
    if affix.value >= 0 {
        label
    } else {
        flip_sign(&label)
    }
}

pub fn gems_view(loadout: &[LoadoutLine], storage: &[StorageLine], sets: &[Synergy]) -> GemsView {
    let pieces = loadout
        .iter()
        .filter(|line| line.equipped)
        .map(|line| PieceView {
            slot: line.slot,
            name: line.item_name.clone(),
            rarity: quality_rarity(line.quality.as_ref()),
            sockets: line.sockets,
            gems: line_gems(line).to_vec(),
            synergies: line.synergies.clone(),
        })
        .collect();
    let pouch = GemKind::ALL
        .into_iter()
        .filter_map(|gem| {
            let quantity: u32 = storage
                .iter()
                .filter(|line| line.item_name == gem.item_name())
                .map(|line| line.quantity)
                .sum();
            (quantity > 0).then_some((gem, quantity))
        })
        .collect();
    GemsView {
        pieces,
        pouch,
        sets: sets.to_vec(),
    }
}

fn line_gems(line: &LoadoutLine) -> &[GemKind] {
    line.quality
        .as_ref()
        .map_or(&[], |quality| quality.gems.as_slice())
}

/// Encaixe na tela: alvo de soltar (vazio) ou origem de arrastar (cheio).
/// `linked` = a gema dele faz parte de uma sinergia acesa (brilha).
#[derive(Component, Debug, Clone, Copy)]
pub struct SocketCell {
    slot: EquipmentSlot,
    index: u8,
    gem: Option<GemKind>,
    linked: bool,
}

/// Elo entre dois encaixes vizinhos; aceso quando os dois se ligam.
#[derive(Component)]
struct SocketLink;

const LINK_GOLD: Color = Color::srgb(1.0, 0.78, 0.2);

/// A gema do encaixe `index` está em alguma sinergia acesa da peça (ou
/// num conjunto do navio)?
fn in_synergy(piece: &PieceView, sets: &[Synergy], index: usize) -> bool {
    let Some(gem) = piece.gems.get(index) else {
        return false;
    };
    piece
        .synergies
        .iter()
        .chain(sets)
        .any(|synergy| synergy.gems().contains(gem))
}

/// Gema da algibeira: origem de arrastar.
#[derive(Component, Debug, Clone, Copy)]
struct GemSource(GemKind);

/// Área da algibeira: soltar a gema de um encaixe aqui a devolve.
#[derive(Component)]
struct PouchArea;

const SOCKET_PX: f32 = 44.0;
const SOCKET_BG: Color = Color::srgba(0.086, 0.125, 0.169, 0.55);

pub fn spawn_gems_body(parent: &mut ChildBuilder, view: &GemsView, atlas: Option<&ItemIcons>) {
    parent
        .spawn(Node {
            column_gap: Val::Px(18.0),
            flex_grow: 1.0,
            min_height: Val::Px(0.0),
            ..default()
        })
        .with_children(|row| {
            row.spawn(Node {
                flex_direction: FlexDirection::Column,
                flex_grow: 1.3,
                flex_basis: Val::Px(0.0),
                row_gap: Val::Px(8.0),
                ..default()
            })
            .with_children(|ship| {
                ship.spawn(crate::i18n::label("PEÇAS INSTALADAS", 13.0, ui::TEXT_DIM));
                if view.pieces.is_empty() {
                    ship.spawn(crate::i18n::label(
                        "Instale peças na aba Equipamento para ter encaixes.",
                        14.0,
                        ui::TEXT,
                    ));
                }
                // Conjunto aceso: faixa dourada no topo das peças.
                for set in &view.sets {
                    ship.spawn((
                        Node {
                            padding: UiRect::axes(Val::Px(10.0), Val::Px(4.0)),
                            border: UiRect::all(Val::Px(2.0)),
                            ..default()
                        },
                        BackgroundColor(LINK_GOLD.with_alpha(0.25)),
                        BorderColor(LINK_GOLD),
                        BorderRadius::all(Val::Px(4.0)),
                        SetBanner,
                    ))
                    .with_children(|banner| {
                        banner.spawn(ui::text(
                            synergy_line(*set).to_uppercase(),
                            14.0,
                            ui::BRASS_INK,
                        ));
                    });
                }
                for piece in &view.pieces {
                    spawn_piece(ship, piece, &view.sets, atlas);
                }
            });
            row.spawn((
                Node {
                    flex_direction: FlexDirection::Column,
                    flex_grow: 1.0,
                    flex_basis: Val::Px(0.0),
                    row_gap: Val::Px(6.0),
                    padding: UiRect::all(Val::Px(8.0)),
                    border: UiRect::all(Val::Px(1.0)),
                    ..default()
                },
                BorderColor(ui::PANEL_BORDER.with_alpha(0.35)),
                BorderRadius::all(Val::Px(6.0)),
                BackgroundColor(ui::PAPER_SHADE.with_alpha(0.5)),
                RelativeCursorPosition::default(),
                PouchArea,
            ))
            .with_children(|pouch| {
                pouch.spawn(crate::i18n::label("GEMAS NO ARMAZÉM", 13.0, ui::TEXT_DIM));
                if view.pouch.is_empty() {
                    pouch.spawn(crate::i18n::label(
                        "Nenhuma gema aqui. Lapide na aba Fabricação.",
                        14.0,
                        ui::TEXT,
                    ));
                }
                for (gem, quantity) in &view.pouch {
                    spawn_pouch_gem(pouch, *gem, *quantity, atlas);
                }
                spawn_link_legend(pouch, atlas);
            });
        });
    parent.spawn(crate::i18n::label(
        "Arraste a gema até um encaixe · arraste para fora ou clique com o botão direito para tirar",
        12.0,
        ui::TEXT_DIM,
    ));
}

/// Faixa do conjunto (pulsa em `paint_links`).
#[derive(Component)]
struct SetBanner;

/// Legenda das ligações: quais pares acendem e o que rendem. Descobrir a
/// combinação é metade da graça, mas sem lista ninguém acha.
fn spawn_link_legend(parent: &mut ChildBuilder, atlas: Option<&ItemIcons>) {
    parent.spawn(crate::i18n::label("LIGAÇÕES", 12.0, ui::TEXT_DIM));
    for (a, b, _, _) in PAIRS {
        parent
            .spawn(Node {
                align_items: AlignItems::Center,
                column_gap: Val::Px(3.0),
                ..default()
            })
            .with_children(|row| {
                for gem in [a, b] {
                    if let Some(atlas) = atlas {
                        row.spawn((
                            Node {
                                width: Val::Px(16.0),
                                height: Val::Px(16.0),
                                ..default()
                            },
                            atlas.node(icons::gem(gem)),
                        ));
                    }
                }
                row.spawn(ui::text(
                    synergy_line(Synergy::Pair(a, b)),
                    11.0,
                    ui::TEXT_DIM,
                ));
            });
    }
    parent.spawn(crate::i18n::label(
        "3 iguais na peça: Ressonância · mesma gema nas 3 peças: Conjunto",
        11.0,
        ui::TEXT_DIM,
    ));
}

fn spawn_piece(
    parent: &mut ChildBuilder,
    piece: &PieceView,
    sets: &[Synergy],
    atlas: Option<&ItemIcons>,
) {
    parent
        .spawn((
            Node {
                align_items: AlignItems::Center,
                column_gap: Val::Px(10.0),
                padding: UiRect::all(Val::Px(6.0)),
                border: UiRect::all(Val::Px(1.0)),
                ..default()
            },
            BorderColor(rarity_color(piece.rarity).with_alpha(0.6)),
            BorderRadius::all(Val::Px(6.0)),
            BackgroundColor(ui::PAPER_SHADE),
        ))
        .with_children(|card| {
            if let (Some(atlas), Some(icon)) = (atlas, icons::item(&piece.name)) {
                spawn_badge(card, atlas, icon, piece.rarity, 1.5);
            }
            card.spawn(Node {
                flex_direction: FlexDirection::Column,
                flex_grow: 1.0,
                row_gap: Val::Px(2.0),
                ..default()
            })
            .with_children(|text| {
                text.spawn(ui::text(
                    crate::affixes::piece_name(&piece.name, None),
                    15.0,
                    rarity_color(piece.rarity),
                ));
                let effects: Vec<String> = piece
                    .gems
                    .iter()
                    .map(|gem| format!("{}: {}", tr(gem.support_name()), gem_effects(*gem)))
                    .collect();
                let summary = if effects.is_empty() {
                    trf("{0} encaixe(s) livre(s)", &[&piece.sockets.to_string()])
                } else {
                    effects.join("\n")
                };
                text.spawn(ui::text(summary, 12.0, ui::TEXT_DIM));
                for synergy in &piece.synergies {
                    text.spawn(ui::text(
                        format!("» {}", synergy_line(*synergy)),
                        12.0,
                        ui::BRASS_INK,
                    ));
                }
            });
            card.spawn(Node {
                align_items: AlignItems::Center,
                ..default()
            })
            .with_children(|sockets| {
                for index in 0..piece.sockets {
                    let i = usize::from(index);
                    if index > 0 {
                        // Elo aceso só quando as duas pontas se ligam.
                        let lit = in_synergy(piece, sets, i - 1) && in_synergy(piece, sets, i);
                        sockets.spawn((
                            Node {
                                width: Val::Px(12.0),
                                height: Val::Px(6.0),
                                ..default()
                            },
                            BackgroundColor(if lit {
                                LINK_GOLD
                            } else {
                                SOCKET_BG.with_alpha(0.25)
                            }),
                            BorderRadius::all(Val::Px(2.0)),
                            SocketLink,
                        ));
                    }
                    let gem = piece.gems.get(i).copied();
                    let linked = in_synergy(piece, sets, i);
                    spawn_socket(sockets, piece.slot, index, gem, linked, atlas);
                }
            });
        });
}

fn spawn_socket(
    parent: &mut ChildBuilder,
    slot: EquipmentSlot,
    index: u8,
    gem: Option<GemKind>,
    linked: bool,
    atlas: Option<&ItemIcons>,
) {
    parent
        .spawn((
            Button,
            Node {
                width: Val::Px(SOCKET_PX),
                height: Val::Px(SOCKET_PX),
                justify_content: JustifyContent::Center,
                align_items: AlignItems::Center,
                border: UiRect::all(Val::Px(2.0)),
                ..default()
            },
            BackgroundColor(SOCKET_BG),
            BorderColor(gem.map_or(ui::BRASS_INK, gem_color)),
            BorderRadius::MAX,
            RelativeCursorPosition::default(),
            SocketCell {
                slot,
                index,
                gem,
                linked,
            },
        ))
        .with_children(|cell| {
            if let Some(atlas) = atlas {
                let icon = gem.map_or(icons::SOCKET, icons::gem);
                cell.spawn((
                    Node {
                        width: Val::Px(32.0),
                        height: Val::Px(32.0),
                        ..default()
                    },
                    atlas.node(icon),
                    PickingBehavior::IGNORE,
                ));
            }
        });
}

fn spawn_pouch_gem(
    parent: &mut ChildBuilder,
    gem: GemKind,
    quantity: u32,
    atlas: Option<&ItemIcons>,
) {
    parent
        .spawn((
            ui::button(
                Node {
                    justify_content: JustifyContent::Start,
                    column_gap: Val::Px(8.0),
                    ..default()
                },
                ui::BUTTON_BG,
            ),
            GemSource(gem),
        ))
        .with_children(|row| {
            if let Some(atlas) = atlas {
                row.spawn((
                    Node {
                        width: Val::Px(32.0),
                        height: Val::Px(32.0),
                        flex_shrink: 0.0,
                        ..default()
                    },
                    atlas.node(icons::gem(gem)),
                ));
            }
            row.spawn(Node {
                flex_direction: FlexDirection::Column,
                ..default()
            })
            .with_children(|text| {
                text.spawn(ui::text(
                    format!(
                        "{} ×{} — {}",
                        tr(gem.item_name()),
                        quantity,
                        tr(gem.support_name())
                    ),
                    14.0,
                    ui::TEXT,
                ));
                text.spawn(ui::text(gem_effects(gem), 12.0, ui::TEXT_DIM));
            });
        });
}

// ── Arrastar e soltar ────────────────────────────────────────────────────

#[derive(Debug, Clone, Copy, PartialEq)]
enum DragFrom {
    Pouch(GemKind),
    Socket(EquipmentSlot, u8, GemKind),
}

impl DragFrom {
    fn gem(self) -> GemKind {
        match self {
            DragFrom::Pouch(gem) | DragFrom::Socket(_, _, gem) => gem,
        }
    }
}

#[derive(Resource, Default)]
struct GemDrag {
    from: Option<DragFrom>,
    ghost: Option<Entity>,
}

/// Camada de cima (fantasma arrastado e efeitos), acima da tela de porto.
#[derive(Component)]
struct GemFxLayer;

#[derive(Component)]
struct DragGhost;

const GHOST_PX: f32 = 48.0;

fn spawn_fx_layer(mut commands: Commands) {
    commands.spawn((
        Node {
            position_type: PositionType::Absolute,
            width: Val::Percent(100.0),
            height: Val::Percent(100.0),
            ..default()
        },
        GlobalZIndex(40),
        PickingBehavior::IGNORE,
        GemFxLayer,
    ));
}

/// Cursor em px de `Node` (lógico da janela dividido pelo UiScale).
fn cursor_ui(windows: &Query<&Window, With<PrimaryWindow>>, scale: &UiScale) -> Option<Vec2> {
    windows
        .get_single()
        .ok()?
        .cursor_position()
        .map(|at| at / scale.0)
}

fn on_gem_tab(docked: &MyDocked, state: &PortScreenState) -> bool {
    docked.0 && state.active_tab == PortTab::Gems
}

#[allow(clippy::too_many_arguments)]
fn start_drag(
    mut commands: Commands,
    mouse: Res<ButtonInput<MouseButton>>,
    docked: Res<MyDocked>,
    state: Res<PortScreenState>,
    mut drag: ResMut<GemDrag>,
    sources: Query<(&Interaction, &GemSource)>,
    sockets: Query<(&RelativeCursorPosition, &SocketCell)>,
    layer: Query<Entity, With<GemFxLayer>>,
    atlas: Option<Res<ItemIcons>>,
) {
    if !on_gem_tab(&docked, &state) || !mouse.just_pressed(MouseButton::Left) {
        return;
    }
    let from = sources
        .iter()
        .find(|(interaction, _)| **interaction == Interaction::Pressed)
        .map(|(_, source)| DragFrom::Pouch(source.0))
        .or_else(|| {
            sockets.iter().find_map(|(cursor, cell)| {
                let gem = cell.gem.filter(|_| cursor.mouse_over())?;
                Some(DragFrom::Socket(cell.slot, cell.index, gem))
            })
        });
    let (Some(from), Ok(layer), Some(atlas)) = (from, layer.get_single(), atlas) else {
        return;
    };
    drag.from = Some(from);
    let ghost = commands
        .spawn((
            Node {
                position_type: PositionType::Absolute,
                width: Val::Px(GHOST_PX),
                height: Val::Px(GHOST_PX),
                ..default()
            },
            atlas.node(icons::gem(from.gem())),
            PickingBehavior::IGNORE,
            DragGhost,
        ))
        .set_parent(layer)
        .id();
    drag.ghost = Some(ghost);
}

/// Fantasma segue o cursor; encaixes livres pulsam enquanto há gema na mão.
#[allow(clippy::too_many_arguments)]
fn follow_drag(
    time: Res<Time>,
    windows: Query<&Window, With<PrimaryWindow>>,
    scale: Res<UiScale>,
    drag: Res<GemDrag>,
    mut ghosts: Query<&mut Node, With<DragGhost>>,
    mut sockets: Query<(&RelativeCursorPosition, &SocketCell, &mut BackgroundColor)>,
) {
    if let (Some(at), Ok(mut node)) = (cursor_ui(&windows, &scale), ghosts.get_single_mut()) {
        node.left = Val::Px(at.x - GHOST_PX / 2.0);
        node.top = Val::Px(at.y - GHOST_PX / 2.0);
    }
    let holding = matches!(drag.from, Some(DragFrom::Pouch(_)));
    let pulse = 0.5 + 0.5 * (time.elapsed_secs() * 6.0).sin();
    for (cursor, cell, mut bg) in &mut sockets {
        let target = holding && cell.gem.is_none();
        bg.0 = match (target, cursor.mouse_over()) {
            (true, true) => ui::BRASS,
            (true, false) => SOCKET_BG.mix(&ui::BRASS, 0.25 + 0.35 * pulse),
            _ => SOCKET_BG,
        };
    }
}

#[allow(clippy::too_many_arguments)]
fn drop_gem(
    mut commands: Commands,
    mouse: Res<ButtonInput<MouseButton>>,
    docked: Res<MyDocked>,
    state: Res<PortScreenState>,
    mut drag: ResMut<GemDrag>,
    sockets: Query<(&RelativeCursorPosition, &SocketCell)>,
    mut connection_manager: ResMut<ConnectionManager>,
) {
    let hovered = sockets
        .iter()
        .find(|(cursor, _)| cursor.mouse_over())
        .map(|(_, cell)| *cell);
    // Botão direito num encaixe cheio: tira direto.
    if on_gem_tab(&docked, &state) && mouse.just_pressed(MouseButton::Right) {
        if let Some(cell) = hovered.filter(|cell| cell.gem.is_some()) {
            let _ = connection_manager.send_message::<ReliableChannel, _>(&UnsocketGem {
                slot: cell.slot,
                index: cell.index,
            });
        }
    }
    let released = mouse.just_released(MouseButton::Left);
    let lost = !on_gem_tab(&docked, &state);
    if !(released || lost) {
        return;
    }
    if let Some(ghost) = drag.ghost.take() {
        commands.entity(ghost).despawn_recursive();
    }
    let Some(from) = drag.from.take() else {
        return;
    };
    if lost {
        return;
    }
    match from {
        DragFrom::Pouch(gem) => {
            if let Some(cell) = hovered.filter(|cell| cell.gem.is_none()) {
                let _ = connection_manager.send_message::<ReliableChannel, _>(&SocketGem {
                    slot: cell.slot,
                    gem,
                });
            }
        }
        DragFrom::Socket(slot, index, _) => {
            let same = hovered.is_some_and(|cell| cell.slot == slot && cell.index == index);
            if !same {
                let _ = connection_manager
                    .send_message::<ReliableChannel, _>(&UnsocketGem { slot, index });
            }
        }
    }
}

// ── Efeitos ──────────────────────────────────────────────────────────────

/// Som da gema (tocado em `audio.rs`).
#[derive(Event, Debug, Clone, Copy)]
pub struct GemSound {
    pub socketed: bool,
}

/// Troca confirmada pelo servidor, esperando o encaixe aparecer na tela.
#[derive(Debug, Clone, Copy)]
struct GemFx {
    slot: EquipmentSlot,
    index: u8,
    gem: GemKind,
    socketed: bool,
    frames: u8,
}

#[derive(Resource, Default)]
struct PendingGemFx(Vec<GemFx>);

/// Encaixou ou tirou? Compara o loadout confirmado com o anterior (mesma
/// peça no slot; troca de peça não é gema).
fn gem_changes(
    old: &[LoadoutLine],
    new: &[LoadoutLine],
) -> Vec<(EquipmentSlot, u8, GemKind, bool)> {
    let mut changes = Vec::new();
    for line in new {
        let Some(before) = old
            .iter()
            .find(|b| b.slot == line.slot && b.item_name == line.item_name && b.equipped)
        else {
            continue;
        };
        let (was, now) = (line_gems(before), line_gems(line));
        if now.len() == was.len() + 1 && now.starts_with(was) {
            changes.push((line.slot, (now.len() - 1) as u8, now[now.len() - 1], true));
        } else if now.len() + 1 == was.len() {
            let index = (0..was.len())
                .find(|&i| now.get(i) != Some(&was[i]))
                .unwrap_or(was.len() - 1);
            changes.push((line.slot, index as u8, was[index], false));
        }
    }
    changes
}

/// Sinergia que acabou de acender: o nome sobe do encaixe com faíscas
/// douradas. `slot` `None` = conjunto do navio (sobe da primeira peça).
#[derive(Debug, Clone)]
struct SynergyFx {
    slot: Option<EquipmentSlot>,
    name: String,
    frames: u8,
}

#[derive(Resource, Default)]
struct PendingSynergyFx(Vec<SynergyFx>);

/// Som do "acendeu" (tocado em `audio.rs`).
#[derive(Event, Debug, Clone, Copy)]
pub struct SynergySound;

/// Sinergias novas no loadout confirmado (mesma peça no slot).
fn new_synergies(old: &[LoadoutLine], new: &[LoadoutLine]) -> Vec<(EquipmentSlot, Synergy)> {
    new.iter()
        .flat_map(|line| {
            let before = old
                .iter()
                .find(|b| b.slot == line.slot)
                .map_or(&[][..], |b| b.synergies.as_slice());
            line.synergies
                .iter()
                .filter(move |s| !before.contains(s))
                .map(move |s| (line.slot, *s))
        })
        .collect()
}

#[allow(clippy::too_many_arguments)]
fn detect_gem_changes(
    known: Res<KnownLoadout>,
    sets: Res<KnownGemSets>,
    mut last: Local<Option<Vec<LoadoutLine>>>,
    mut last_sets: Local<Option<Vec<Synergy>>>,
    mut pending: ResMut<PendingGemFx>,
    mut lit: ResMut<PendingSynergyFx>,
    mut sounds: EventWriter<GemSound>,
    mut synergy_sounds: EventWriter<SynergySound>,
) {
    if sets.is_changed() {
        if let Some(old) = last_sets.as_ref() {
            for set in sets.0.iter().filter(|s| !old.contains(s)) {
                synergy_sounds.send(SynergySound);
                lit.0.push(SynergyFx {
                    slot: None,
                    name: tr(&set.name()),
                    frames: 0,
                });
            }
        }
        *last_sets = Some(sets.0.clone());
    }
    if !known.is_changed() {
        return;
    }
    if let Some(old) = last.as_ref() {
        for (slot, synergy) in new_synergies(old, &known.0) {
            info!(?slot, ?synergy, "sinergia acesa");
            synergy_sounds.send(SynergySound);
            lit.0.push(SynergyFx {
                slot: Some(slot),
                name: tr(&synergy.name()),
                frames: 0,
            });
        }
        for (slot, index, gem, socketed) in gem_changes(old, &known.0) {
            info!(?slot, index, ?gem, socketed, "gema: efeito");
            sounds.send(GemSound { socketed });
            pending.0.push(GemFx {
                slot,
                index,
                gem,
                socketed,
                frames: 0,
            });
        }
    }
    *last = Some(known.0.clone());
}

#[derive(Component)]
struct GemSpark {
    origin: Vec2,
    velocity: Vec2,
    age: f32,
    life: f32,
}

/// Onda que abre do encaixe.
#[derive(Component)]
struct GemRing {
    center: Vec2,
    age: f32,
}

/// Gema grande que encolhe até caber no encaixe (encaixar) ou sobe e some
/// (tirar).
#[derive(Component)]
struct GemPop {
    center: Vec2,
    age: f32,
    socketed: bool,
}

const RING_LIFE: f32 = 0.5;
const POP_LIFE: f32 = 0.55;

/// Nome da sinergia subindo e sumindo.
#[derive(Component)]
struct FloatText {
    origin: Vec2,
    age: f32,
}

const FLOAT_LIFE: f32 = 1.6;

fn spawn_synergy_fx(
    mut commands: Commands,
    mut pending: ResMut<PendingSynergyFx>,
    cells: Query<(&SocketCell, &GlobalTransform, &ComputedNode)>,
    layer: Query<Entity, With<GemFxLayer>>,
) {
    let Ok(layer) = layer.get_single() else {
        return;
    };
    pending.0.retain_mut(|fx| {
        fx.frames += 1;
        let found = cells.iter().find(|(cell, _, node)| {
            fx.slot.map_or(true, |slot| cell.slot == slot) && cell.index == 0 && node.size().x > 0.0
        });
        let Some((_, transform, node)) = found else {
            return fx.frames < 30;
        };
        let center = transform.translation().truncate() * node.inverse_scale_factor();
        for i in 0..24 {
            let angle = i as f32 / 24.0 * std::f32::consts::TAU;
            commands
                .spawn((
                    Node {
                        position_type: PositionType::Absolute,
                        width: Val::Px(6.0),
                        height: Val::Px(6.0),
                        ..default()
                    },
                    BackgroundColor(if i % 3 == 0 { Color::WHITE } else { LINK_GOLD }),
                    GemSpark {
                        origin: center,
                        velocity: Vec2::from_angle(angle) * (140.0 + (i % 4) as f32 * 30.0),
                        age: 0.0,
                        life: 0.9,
                    },
                    PickingBehavior::IGNORE,
                ))
                .set_parent(layer);
        }
        commands
            .spawn((
                Node {
                    position_type: PositionType::Absolute,
                    padding: UiRect::axes(Val::Px(10.0), Val::Px(4.0)),
                    border: UiRect::all(Val::Px(2.0)),
                    ..default()
                },
                BackgroundColor(ui::PAPER),
                BorderColor(LINK_GOLD),
                BorderRadius::all(Val::Px(4.0)),
                FloatText {
                    origin: center - Vec2::new(60.0, 40.0),
                    age: 0.0,
                },
                PickingBehavior::IGNORE,
            ))
            .with_children(|tag| {
                tag.spawn(ui::text(
                    format!("{}!", fx.name.to_uppercase()),
                    18.0,
                    ui::BRASS_INK,
                ));
            })
            .set_parent(layer);
        false
    });
}

#[allow(clippy::type_complexity)]
fn animate_float_text(
    mut commands: Commands,
    time: Res<Time>,
    mut floats: Query<(
        Entity,
        &mut FloatText,
        &mut Node,
        &mut BackgroundColor,
        &mut BorderColor,
    )>,
    mut texts: Query<(&Parent, &mut TextColor)>,
) {
    for (entity, mut float, mut node, mut bg, mut border) in &mut floats {
        float.age += time.delta_secs();
        let t = float.age / FLOAT_LIFE;
        if t >= 1.0 {
            commands.entity(entity).despawn_recursive();
            continue;
        }
        let alpha = if t < 0.7 { 1.0 } else { 1.0 - (t - 0.7) / 0.3 };
        node.left = Val::Px(float.origin.x);
        node.top = Val::Px(float.origin.y - 50.0 * t);
        bg.0 = bg.0.with_alpha(alpha);
        border.0 = border.0.with_alpha(alpha);
        for (parent, mut color) in &mut texts {
            if parent.get() == entity {
                color.0 = color.0.with_alpha(alpha);
            }
        }
    }
}

/// Encaixes ligados respiram em dourado; elos e faixa do conjunto também.
#[allow(clippy::type_complexity)]
fn paint_links(
    time: Res<Time>,
    mut sockets: Query<(&SocketCell, &mut BorderColor), (Without<SocketLink>, Without<SetBanner>)>,
    mut links: Query<&mut BackgroundColor, (With<SocketLink>, Without<SetBanner>)>,
    mut banners: Query<&mut BorderColor, (With<SetBanner>, Without<SocketCell>)>,
) {
    let pulse = 0.5 + 0.5 * (time.elapsed_secs() * 3.0).sin();
    let glow = LINK_GOLD.mix(&Color::WHITE, 0.45 * pulse);
    for (cell, mut border) in &mut sockets {
        if cell.linked {
            border.0 = glow;
        }
    }
    for mut bg in &mut links {
        if bg.0.alpha() > 0.9 {
            bg.0 = glow;
        }
    }
    for mut border in &mut banners {
        border.0 = glow;
    }
}

fn spawn_gem_fx(
    mut commands: Commands,
    mut pending: ResMut<PendingGemFx>,
    cells: Query<(&SocketCell, &GlobalTransform, &ComputedNode)>,
    layer: Query<Entity, With<GemFxLayer>>,
    atlas: Option<Res<ItemIcons>>,
) {
    let Ok(layer) = layer.get_single() else {
        return;
    };
    pending.0.retain_mut(|fx| {
        fx.frames += 1;
        let found = cells.iter().find(|(cell, _, node)| {
            cell.slot == fx.slot && cell.index == fx.index && node.size().x > 0.0
        });
        let Some((_, transform, node)) = found else {
            // A tela remonta no quadro seguinte; sem o encaixe em meio
            // segundo (aba fechada), a festa fica só no som.
            return fx.frames < 30;
        };
        // Transform da UI é em px físicos; `Node` quer px de layout.
        let center = transform.translation().truncate() * node.inverse_scale_factor();
        let color = gem_color(fx.gem);
        let count = if fx.socketed { 18 } else { 10 };
        for i in 0..count {
            let angle = i as f32 / count as f32 * std::f32::consts::TAU + (i % 3) as f32 * 0.3;
            let speed = if fx.socketed { 110.0 } else { 60.0 } + (i % 4) as f32 * 25.0;
            commands
                .spawn((
                    Node {
                        position_type: PositionType::Absolute,
                        width: Val::Px(6.0),
                        height: Val::Px(6.0),
                        ..default()
                    },
                    BackgroundColor(if i % 4 == 0 { Color::WHITE } else { color }),
                    GemSpark {
                        origin: center,
                        velocity: Vec2::from_angle(angle) * speed,
                        age: 0.0,
                        life: if fx.socketed { 0.7 } else { 0.9 },
                    },
                    PickingBehavior::IGNORE,
                ))
                .set_parent(layer);
        }
        if fx.socketed {
            commands
                .spawn((
                    Node {
                        position_type: PositionType::Absolute,
                        border: UiRect::all(Val::Px(3.0)),
                        ..default()
                    },
                    BorderColor(color),
                    BorderRadius::MAX,
                    GemRing { center, age: 0.0 },
                    PickingBehavior::IGNORE,
                ))
                .set_parent(layer);
        }
        if let Some(atlas) = atlas.as_ref() {
            commands
                .spawn((
                    Node {
                        position_type: PositionType::Absolute,
                        ..default()
                    },
                    atlas.node(icons::gem(fx.gem)),
                    GemPop {
                        center,
                        age: 0.0,
                        socketed: fx.socketed,
                    },
                    PickingBehavior::IGNORE,
                ))
                .set_parent(layer);
        }
        false
    });
}

fn place(node: &mut Node, center: Vec2, size: f32) {
    node.left = Val::Px(center.x - size / 2.0);
    node.top = Val::Px(center.y - size / 2.0);
    node.width = Val::Px(size);
    node.height = Val::Px(size);
}

#[allow(clippy::type_complexity)]
fn animate_gem_fx(
    mut commands: Commands,
    time: Res<Time>,
    mut sparks: Query<(Entity, &mut GemSpark, &mut Node, &mut BackgroundColor)>,
    mut rings: Query<(Entity, &mut GemRing, &mut Node, &mut BorderColor), Without<GemSpark>>,
    mut pops: Query<
        (Entity, &mut GemPop, &mut Node, &mut ImageNode),
        (Without<GemSpark>, Without<GemRing>),
    >,
) {
    let dt = time.delta_secs();
    for (entity, mut spark, mut node, mut bg) in &mut sparks {
        spark.age += dt;
        if spark.age >= spark.life {
            commands.entity(entity).despawn_recursive();
            continue;
        }
        let t = spark.age;
        // Leque que desacelera e cai um pouco.
        let at = spark.origin + spark.velocity * t * (1.0 - 0.4 * t) + Vec2::new(0.0, 90.0 * t * t);
        node.left = Val::Px(at.x - 3.0);
        node.top = Val::Px(at.y - 3.0);
        bg.0 = bg.0.with_alpha(1.0 - t / spark.life);
    }
    for (entity, mut ring, mut node, mut border) in &mut rings {
        ring.age += dt;
        let t = ring.age / RING_LIFE;
        if t >= 1.0 {
            commands.entity(entity).despawn_recursive();
            continue;
        }
        place(&mut node, ring.center, SOCKET_PX + 60.0 * t);
        border.0 = border.0.with_alpha(1.0 - t);
    }
    for (entity, mut pop, mut node, mut image) in &mut pops {
        pop.age += dt;
        let t = pop.age / POP_LIFE;
        if t >= 1.0 {
            commands.entity(entity).despawn_recursive();
            continue;
        }
        if pop.socketed {
            // Cai de grande no encaixe: 3x → 1x com um quique no fim.
            let ease = (1.0 - t).powi(2);
            place(&mut node, pop.center, 32.0 * (1.0 + 2.0 * ease));
            image.color = Color::WHITE.with_alpha((1.4 - t).min(1.0));
        } else {
            // Salta do encaixe e some.
            let lifted = pop.center - Vec2::new(0.0, 60.0 * t);
            place(&mut node, lifted, 32.0 * (1.0 + 0.3 * t));
            image.color = Color::WHITE.with_alpha(1.0 - t);
        }
    }
}

/// Dev (§39): MARVYR_AUTOGEM=1 encaixa a primeira gema da algibeira duas
/// vezes e depois tira uma — capturas do efeito sem mouse.
fn autogem(
    time: Res<Time>,
    docked: Res<MyDocked>,
    loadout: Res<KnownLoadout>,
    storage: Res<KnownPortStorage>,
    mut timer: Local<f32>,
    mut step: Local<u8>,
    mut connection_manager: ResMut<ConnectionManager>,
) {
    if std::env::var_os("MARVYR_AUTOGEM").is_none() || !docked.0 || *step >= 3 {
        return;
    }
    *timer += time.delta_secs();
    if *timer < 3.0 {
        return;
    }
    let view = gems_view(&loadout.0, &storage.0, &[]);
    let Some(piece) = view.pieces.iter().max_by_key(|piece| piece.sockets) else {
        return;
    };
    *timer = 0.0;
    if *step < 2 {
        let Some((gem, _)) = view.pouch.first() else {
            return;
        };
        let _ = connection_manager.send_message::<ReliableChannel, _>(&SocketGem {
            slot: piece.slot,
            gem: *gem,
        });
    } else {
        let _ = connection_manager.send_message::<ReliableChannel, _>(&UnsocketGem {
            slot: piece.slot,
            index: 0,
        });
    }
    *step += 1;
}

pub struct GemsPlugin;

impl Plugin for GemsPlugin {
    fn build(&self, app: &mut App) {
        app.init_resource::<GemDrag>()
            .init_resource::<PendingGemFx>()
            .init_resource::<PendingSynergyFx>()
            .init_resource::<KnownGemSets>()
            .add_event::<GemSound>()
            .add_event::<SynergySound>()
            .add_systems(Startup, spawn_fx_layer)
            .add_systems(
                Update,
                (
                    (start_drag, follow_drag, drop_gem).chain(),
                    detect_gem_changes,
                    spawn_gem_fx,
                    animate_gem_fx,
                    autogem,
                    receive_gem_sets,
                    spawn_synergy_fx,
                    animate_float_text,
                    paint_links,
                ),
            );
    }
}

#[cfg(test)]
mod tests {
    use marvyr_domain_items::Quality;

    use super::*;

    fn line(gems: Vec<GemKind>) -> LoadoutLine {
        LoadoutLine {
            slot: EquipmentSlot::Weapon,
            item_name: String::from("Canhão de Bronze"),
            equipped: true,
            quality: Some(Quality {
                rarity: Rarity::Rare,
                affixes: Vec::new(),
                gems,
                map_mods: Vec::new(),
                aspect: None,
            }),
            sockets: 3,
            synergies: Vec::new(),
        }
    }

    #[test]
    fn confirmed_loadout_tells_what_was_socketed_or_pulled() {
        use GemKind::*;
        let socketed = gem_changes(&[line(vec![Ruby])], &[line(vec![Ruby, Topaz])]);
        assert_eq!(socketed, vec![(EquipmentSlot::Weapon, 1, Topaz, true)]);
        let pulled = gem_changes(&[line(vec![Ruby, Topaz])], &[line(vec![Topaz])]);
        assert_eq!(pulled, vec![(EquipmentSlot::Weapon, 0, Ruby, false)]);
        assert!(gem_changes(&[line(vec![Ruby])], &[line(vec![Ruby])]).is_empty());
    }

    #[test]
    fn newly_lit_synergy_is_detected_and_named() {
        use GemKind::*;
        let mut before = line(vec![Ruby]);
        let mut after = line(vec![Ruby, Emerald]);
        before.synergies = Vec::new();
        after.synergies = vec![Synergy::Pair(Ruby, Emerald)];
        assert_eq!(
            new_synergies(&[before.clone()], &[after.clone()]),
            vec![(EquipmentSlot::Weapon, Synergy::Pair(Ruby, Emerald))]
        );
        assert!(new_synergies(&[after.clone()], &[after]).is_empty());
        assert_eq!(
            synergy_line(Synergy::Pair(Ruby, Emerald)),
            "Salva Contínua: -8% recarga"
        );
    }

    #[test]
    fn effects_read_gain_and_cost() {
        assert_eq!(gem_effects(GemKind::Ruby), "+8 dano · +10% recarga");
        assert_eq!(gem_effects(GemKind::Sapphire), "+15% alcance · -3 dano");
    }

    #[test]
    fn pouch_sums_stacks_and_skips_what_is_not_a_gem() {
        let stack = |name: &str, quantity| StorageLine {
            item: GemKind::Ruby.item_id(),
            item_name: String::from(name),
            quantity,
            instance: None,
            quality: None,
        };
        let view = gems_view(
            &[line(vec![])],
            &[stack("Rubi", 20), stack("Rubi", 3), stack("Madeira", 50)],
            &[],
        );
        assert_eq!(view.pouch, vec![(GemKind::Ruby, 23)]);
        assert_eq!(view.pieces[0].sockets, 3);
    }
}
