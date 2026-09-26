//! Nós de recurso no client (PRD MF-018, Phase 6). O node é desenhado do
//! snapshot do handshake e atualizado por deltas — o client nunca inventa
//! estoque (Pilar 4). Coleta é intenção (`GatherNode`); o servidor decide.

use std::collections::HashMap;

use bevy::ecs::prelude::*;
use bevy::prelude::*;
use bevy::sprite::Anchor;
use lightyear::prelude::*;
use marvyr_protocol::{GatherResult, NodeState, NodeUpdated, NodesSnapshot};

use crate::assets::{deco, layers, GameAssets};

/// Nós conhecidos: posição e estoque para a coleta (alvo do G / autogather).
#[derive(Resource, Debug, Default)]
pub struct KnownNodes(pub HashMap<u32, NodeInfo>);

#[derive(Debug, Clone)]
pub struct NodeInfo {
    pub pos: Vec2,
    pub stock: u32,
    pub resource_name: String,
    /// v37: veio dourado (rende 5x).
    pub golden: bool,
}

/// Entidade visual de um node: peças + rótulo filho com nome e estoque.
#[derive(Component)]
pub struct NodeVisual {
    pub node_id: u32,
}

/// Rótulo de estoque pendurado no node visual — casa pelo `node_id`.
#[derive(Component)]
pub struct NodeLabel {
    pub node_id: u32,
}

pub struct NodePlugin;

impl Plugin for NodePlugin {
    fn build(&self, app: &mut App) {
        app.init_resource::<KnownNodes>().add_systems(
            Update,
            (
                handle_nodes_snapshot,
                handle_node_updated,
                handle_gather_result,
                sync_golden_beams,
                animate_golden_beams,
            ),
        );
    }
}

/// Peça visual do node (tora, pedra, coral); escurece quando esgota.
#[derive(Component)]
pub struct NodePiece;

fn label_text(state: &NodeState) -> String {
    let name = if state.golden {
        crate::i18n::trf("{0} DOURADO", &[&crate::i18n::tr(&state.resource_name)])
    } else {
        crate::i18n::tr(&state.resource_name)
    };
    format!("{name} {}/{}", state.stock, state.max_stock)
}

/// v37: feixe e faíscas douradas sobre o veio dourado.
#[derive(Component)]
struct GoldenBeam;

const GOLD: Color = Color::srgb(1.0, 0.82, 0.25);

/// Liga e desliga o feixe dourado conforme o servidor diz.
fn sync_golden_beams(
    mut commands: Commands,
    known: Res<KnownNodes>,
    mut images: ResMut<Assets<Image>>,
    mut beam: Local<Option<Handle<Image>>>,
    nodes: Query<(Entity, &NodeVisual, Option<&Children>)>,
    beams: Query<(), With<GoldenBeam>>,
) {
    if !known.is_changed() {
        return;
    }
    for (entity, visual, children) in &nodes {
        let golden = known.0.get(&visual.node_id).is_some_and(|n| n.golden);
        let current: Vec<Entity> = children
            .map(|c| c.iter().copied().filter(|c| beams.contains(*c)).collect())
            .unwrap_or_default();
        match (golden, current.is_empty()) {
            (true, true) => {
                let image = beam
                    .get_or_insert_with(|| images.add(crate::ship::beam_image()))
                    .clone();
                commands.entity(entity).with_children(|parent| {
                    parent.spawn((
                        GoldenBeam,
                        Sprite {
                            image,
                            color: GOLD,
                            custom_size: Some(Vec2::new(14.0, 150.0)),
                            // No Bevy 0.15 o sprite só respeita o `anchor`
                            // de dentro; o componente `Anchor` é do Text2d.
                            anchor: Anchor::BottomCenter,
                            ..default()
                        },
                        Transform::from_xyz(0.0, 0.0, -0.05),
                    ));
                });
            }
            (false, false) => {
                for child in current {
                    commands.entity(child).despawn_recursive();
                }
            }
            _ => {}
        }
    }
}

/// Pulso do feixe e faíscas que sobem do veio.
fn animate_golden_beams(
    mut commands: Commands,
    time: Res<Time>,
    mut beams: Query<(&mut Sprite, &mut Transform, &GlobalTransform), With<GoldenBeam>>,
    mut clock: Local<f32>,
) {
    let t = time.elapsed_secs();
    *clock += time.delta_secs();
    let sparkle = *clock >= 0.25;
    if sparkle {
        *clock = 0.0;
    }
    for (i, (mut sprite, mut transform, global)) in beams.iter_mut().enumerate() {
        let pulse = 0.5 + 0.5 * (t * 3.5 + i as f32).sin();
        sprite.color = GOLD.with_alpha(0.6 + 0.35 * pulse);
        transform.scale.x = 1.0 + 0.25 * pulse;
        if sparkle {
            let at = global.translation().truncate();
            let drift = ((t * 7.3 + i as f32 * 1.7).sin()) * 10.0;
            crate::vfx::spawn_particle(
                &mut commands,
                at + Vec2::new(drift, 4.0),
                crate::vfx::Particle {
                    velocity: Vec2::new(drift * 0.4, 34.0),
                    drag: 0.6,
                    life: 1.1,
                    age: 0.0,
                    size: (2.5, 0.5),
                    color: GOLD,
                    z: layers::VFX,
                },
            );
        }
    }
}

/// Node esgotado fica translúcido até o respawn.
fn piece_alpha(stock: u32) -> f32 {
    if stock == 0 {
        0.35
    } else {
        1.0
    }
}

/// Arte de cada recurso (MF-058): toras à deriva, afloramento de rocha com
/// veio de minério, recife de coral. Recurso desconhecido cai nas pedras.
fn spawn_node_visual(commands: &mut Commands, assets: &GameAssets, state: &NodeState) {
    let deco_sprite = |index| {
        Sprite::from_atlas_image(
            assets.water_and_islands.clone(),
            TextureAtlas {
                layout: assets.deco.clone(),
                index,
            },
        )
    };
    let alpha = piece_alpha(state.stock);
    let mut pieces: Vec<(Sprite, Vec2, f32, f32)> = Vec::new();
    match state.resource_name.as_str() {
        "Madeira" => {
            for (index, offset, angle) in [
                (deco::PLANK, Vec2::new(-6.0, 5.0), 0.3),
                (deco::PLANK, Vec2::new(7.0, -3.0), -0.4),
                (deco::PLANK_B, Vec2::new(-2.0, -9.0), 1.2),
                (deco::PLANK_B, Vec2::new(9.0, 8.0), 2.0),
            ] {
                pieces.push((deco_sprite(index), offset, angle, 1.2));
            }
        }
        "Coral Negro" => {
            for (offset, size, color) in [
                (Vec2::new(-5.0, 3.0), 7.0, Color::srgb(0.62, 0.22, 0.55)),
                (Vec2::new(5.0, -2.0), 6.0, Color::srgb(0.78, 0.35, 0.62)),
                (Vec2::new(0.0, -7.0), 5.0, Color::srgb(0.45, 0.14, 0.42)),
                (Vec2::new(6.0, 7.0), 4.0, Color::srgb(0.9, 0.55, 0.75)),
                (Vec2::new(-8.0, -5.0), 3.0, Color::srgb(0.95, 0.7, 0.85)),
            ] {
                pieces.push((
                    Sprite::from_color(color, Vec2::splat(size)),
                    offset,
                    0.78,
                    1.0,
                ));
            }
        }
        _ => {
            pieces.push((
                deco_sprite(deco::ROCK_MOSS_B),
                Vec2::new(-4.0, 2.0),
                0.0,
                1.5,
            ));
            pieces.push((deco_sprite(deco::ROCK), Vec2::new(8.0, -5.0), 0.0, 1.1));
            for offset in [
                Vec2::new(-6.0, 4.0),
                Vec2::new(-1.0, -1.0),
                Vec2::new(9.0, -3.0),
            ] {
                pieces.push((
                    Sprite::from_color(Color::srgb(1.0, 0.82, 0.35), Vec2::splat(1.5)),
                    offset,
                    0.0,
                    1.0,
                ));
            }
        }
    }
    commands
        .spawn((
            NodeVisual {
                node_id: state.node_id,
            },
            Transform::from_xyz(state.x, state.y, layers::RESOURCES),
            Visibility::default(),
        ))
        .with_children(|parent| {
            for (i, (mut sprite, offset, angle, scale)) in pieces.into_iter().enumerate() {
                sprite.color.set_alpha(alpha);
                parent.spawn((
                    sprite,
                    Transform::from_translation(offset.extend(i as f32 * 0.01))
                        .with_rotation(Quat::from_rotation_z(angle))
                        .with_scale(Vec3::splat(scale)),
                    NodePiece,
                ));
            }
            // Etiqueta impressa: papel sob tinta — legível em água rasa e funda.
            let label = label_text(state);
            let width = label.chars().count() as f32 * 4.4 + 8.0;
            parent.spawn((
                Sprite {
                    color: crate::ui::PAPER.with_alpha(0.92),
                    custom_size: Some(Vec2::new(width, 11.0)),
                    ..default()
                },
                Transform::from_xyz(0.0, -19.0, layers::LABELS - layers::RESOURCES - 0.01),
            ));
            parent.spawn((
                NodeLabel {
                    node_id: state.node_id,
                },
                Text2d::new(label),
                TextLayout::new_with_no_wrap(),
                TextFont {
                    font_size: 8.0,
                    ..default()
                },
                TextColor(crate::ui::INK),
                Anchor::Center,
                Transform::from_xyz(0.0, -19.0, layers::LABELS - layers::RESOURCES),
            ));
        });
}

/// Estado completo no handshake (depois disso, só deltas).
#[allow(clippy::too_many_arguments)]
fn handle_nodes_snapshot(
    mut commands: Commands,
    mut snapshot_events: EventReader<ClientReceiveMessage<NodesSnapshot>>,
    mut known: ResMut<KnownNodes>,
    assets: Res<GameAssets>,
    existing: Query<&NodeVisual>,
) {
    for event in snapshot_events.read() {
        for state in &event.message().nodes {
            known.0.insert(
                state.node_id,
                NodeInfo {
                    pos: Vec2::new(state.x, state.y),
                    stock: state.stock,
                    resource_name: state.resource_name.clone(),
                    golden: state.golden,
                },
            );
            if existing
                .iter()
                .any(|visual| visual.node_id == state.node_id)
            {
                continue;
            }
            spawn_node_visual(&mut commands, &assets, state);
        }
        info!(
            nodes = event.message().nodes.len(),
            "mapa de recursos recebido"
        );
    }
}

/// Delta de node (coleta de outro jogador, ou o próprio respawn).
fn handle_node_updated(
    mut updated_events: EventReader<ClientReceiveMessage<NodeUpdated>>,
    mut known: ResMut<KnownNodes>,
    nodes: Query<(&NodeVisual, &Children)>,
    mut pieces: Query<&mut Sprite, With<NodePiece>>,
    mut labels: Query<(&mut Text2d, &NodeLabel)>,
) {
    for event in updated_events.read() {
        let state = &event.message().node;
        known.0.insert(
            state.node_id,
            NodeInfo {
                pos: Vec2::new(state.x, state.y),
                stock: state.stock,
                resource_name: state.resource_name.clone(),
                golden: state.golden,
            },
        );
        // Esgotado escurece; repovoado volta à cor do recurso.
        if let Some((_, children)) = nodes
            .iter()
            .find(|(visual, _)| visual.node_id == state.node_id)
        {
            for child in children.iter() {
                if let Ok(mut sprite) = pieces.get_mut(*child) {
                    sprite.color.set_alpha(piece_alpha(state.stock));
                }
            }
        }
        let label_text = label_text(state);
        for (mut text, label) in &mut labels {
            if label.node_id == state.node_id {
                text.0 = label_text.clone();
            }
        }
    }
}

fn handle_gather_result(
    mut events: EventReader<ClientReceiveMessage<GatherResult>>,
    mut notices: EventWriter<crate::net::PlayerNotice>,
) {
    for event in events.read() {
        let result = event.message();
        if result.success {
            info!(
                node_id = result.node_id,
                gathered = result.gathered,
                "coleta no porão"
            );
        } else {
            warn!(node_id = result.node_id, reason = %result.reason, "coleta recusada pelo servidor");
        }
        notices.send_batch(crate::net::PlayerNotice::from_refusal(
            result.success,
            &result.reason,
        ));
    }
}
