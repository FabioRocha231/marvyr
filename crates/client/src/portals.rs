//! Portais no client (MF-059): Cerração e Sorvedouros. O servidor
//! manda a lista inteira a cada segundo; aqui só desenhamos — cerração
//! rodopiante, redemoinho em espiral, setas na borda da tela para portais
//! fora de vista, névoa de visão dentro da cerração e o salto de câmera quando
//! o próprio navio atravessa.

use std::collections::HashMap;

use bevy::prelude::*;
use bevy::render::render_asset::RenderAssetUsages;
use bevy::render::render_resource::{Extent3d, TextureDimension, TextureFormat};
use bevy::sprite::Anchor;
use lightyear::prelude::ClientReceiveMessage;
use marvyr_domain_world::map::{FOG_ZONE, MAELSTROM_ZONE};
use marvyr_protocol::{PortalKindWire, PortalState, PortalsUpdate};

use crate::assets::layers;
use crate::ship::ShipVisual;
use crate::ui;
use crate::zone::CurrentZone;

/// Portais conhecidos, por id, com o instante (s de app) em que chegaram.
#[derive(Resource, Default)]
pub struct KnownPortals {
    pub portals: HashMap<u32, PortalState>,
    pub received_at: f32,
}

#[derive(Component)]
struct PortalVisual(u32);

#[derive(Component)]
struct PortalLabel;

/// Partícula que orbita o centro do portal (névoa ou redemoinho).
#[derive(Component)]
struct Orbiter {
    angle: f32,
    radius: f32,
    /// rad/s; negativo gira no sentido horário.
    spin: f32,
    /// m/s para dentro (redemoinho suga; cerração só respira).
    suck: f32,
    outer: f32,
}

#[derive(Component)]
struct EdgeMarker(u32);

#[derive(Component)]
struct FogOverlay;

#[derive(Component)]
struct CollapseTimer;

pub struct PortalClientPlugin;

impl Plugin for PortalClientPlugin {
    fn build(&self, app: &mut App) {
        app.init_resource::<KnownPortals>()
            .add_systems(Startup, spawn_fog_overlay)
            .add_systems(
                Update,
                (
                    receive_portals,
                    sync_portal_visuals,
                    animate_orbiters,
                    update_edge_markers,
                    update_fog_and_timer,
                    snap_camera_on_teleport.after(crate::camera::follow_camera),
                ),
            );
    }
}

fn receive_portals(
    time: Res<Time>,
    mut events: EventReader<ClientReceiveMessage<PortalsUpdate>>,
    mut known: ResMut<KnownPortals>,
) {
    let Some(event) = events.read().last() else {
        return;
    };
    known.portals = event
        .message()
        .portals
        .iter()
        .map(|p| (p.portal_id, *p))
        .collect();
    known.received_at = time.elapsed_secs();
}

fn remaining(known: &KnownPortals, portal: &PortalState, now: f32) -> f32 {
    (portal.expires_in_secs - (now - known.received_at)).max(0.0)
}

fn clock(secs: f32) -> String {
    let secs = secs as u32;
    format!("{}:{:02}", secs / 60, secs % 60)
}

/// Texto curto do portal no idioma ativo.
fn portal_label(portal: &PortalState, remaining: f32) -> String {
    use crate::i18n::{tr, trf};
    let uses = portal
        .uses_left
        .map(|n| format!(" · {}", trf("{0} vaga(s)", &[&n.to_string()])))
        .unwrap_or_default();
    match portal.kind {
        PortalKindWire::FogGate => format!("{} {}{uses}", tr("Cerração"), clock(remaining)),
        PortalKindWire::FogExit => format!("{} {}", tr("Saída da Cerração"), clock(remaining)),
        PortalKindWire::Whirlpool => format!("{}{uses}", tr("Sorvedouro")),
    }
}

fn portal_color(kind: PortalKindWire) -> Color {
    match kind {
        PortalKindWire::FogGate => Color::srgb(0.86, 0.9, 0.95),
        PortalKindWire::FogExit => Color::srgb(0.7, 1.0, 0.8),
        PortalKindWire::Whirlpool => Color::srgb(0.55, 0.85, 1.0),
    }
}

/// Fio da etiqueta de borda no papel (as cores do mar são claras demais).
fn portal_ink(kind: PortalKindWire) -> Color {
    match kind {
        PortalKindWire::FogGate | PortalKindWire::FogExit => ui::INK_SOFT,
        PortalKindWire::Whirlpool => ui::TEAL,
    }
}

fn spawn_portal(commands: &mut Commands, portal: &PortalState) {
    let color = portal_color(portal.kind);
    let whirl = portal.kind == PortalKindWire::Whirlpool;
    commands
        .spawn((
            PortalVisual(portal.portal_id),
            Transform::from_xyz(portal.x, portal.y, layers::VFX - 0.5),
            Visibility::default(),
        ))
        .with_children(|parent| {
            if whirl {
                // Olho escuro do redemoinho.
                parent.spawn((
                    Sprite::from_color(Color::srgba(0.02, 0.1, 0.22, 0.7), Vec2::splat(22.0)),
                    Transform::from_xyz(0.0, 0.0, -0.2).with_rotation(Quat::from_rotation_z(0.785)),
                ));
            }
            let count = if whirl { 36 } else { 28 };
            let outer = portal.radius * if whirl { 1.6 } else { 2.2 };
            for k in 0..count {
                let t = k as f32 / count as f32;
                let (size, alpha) = if whirl {
                    (2.6, 0.85)
                } else {
                    (14.0 + 10.0 * t, 0.22)
                };
                parent.spawn((
                    Sprite::from_color(color.with_alpha(alpha), Vec2::splat(size)),
                    Transform::default(),
                    Orbiter {
                        angle: t * std::f32::consts::TAU * 3.0,
                        radius: outer * (0.2 + 0.8 * ((k * 7) % count) as f32 / count as f32),
                        spin: if whirl { 1.6 } else { 0.25 },
                        suck: if whirl { 9.0 } else { 0.0 },
                        outer,
                    },
                ));
            }
            parent.spawn((
                PortalLabel,
                Text2d::new(""),
                TextLayout::new_with_no_wrap(),
                TextFont {
                    font_size: 11.0,
                    ..default()
                },
                TextColor(color),
                Anchor::Center,
                Transform::from_xyz(0.0, -outer - 12.0, layers::LABELS),
            ));
        });
}

fn sync_portal_visuals(
    mut commands: Commands,
    time: Res<Time>,
    known: Res<KnownPortals>,
    visuals: Query<(Entity, &PortalVisual, &Children)>,
    mut labels: Query<&mut Text2d, With<PortalLabel>>,
) {
    let now = time.elapsed_secs();
    let mut drawn = Vec::new();
    for (entity, visual, children) in &visuals {
        let Some(portal) = known.portals.get(&visual.0) else {
            commands.entity(entity).despawn_recursive();
            continue;
        };
        drawn.push(visual.0);
        for child in children.iter() {
            if let Ok(mut text) = labels.get_mut(*child) {
                let label = portal_label(portal, remaining(&known, portal, now));
                if text.0 != label {
                    text.0 = label;
                }
            }
        }
    }
    for portal in known.portals.values() {
        if !drawn.contains(&portal.portal_id) {
            spawn_portal(&mut commands, portal);
        }
    }
}

fn animate_orbiters(time: Res<Time>, mut orbiters: Query<(&mut Orbiter, &mut Transform)>) {
    let dt = time.delta_secs();
    for (mut o, mut transform) in &mut orbiters {
        o.angle += o.spin * dt * (1.0 + 20.0 / o.radius.max(4.0));
        o.radius -= o.suck * dt;
        if o.radius < 3.0 {
            o.radius = o.outer;
        }
        let breathe = if o.suck == 0.0 {
            1.0 + 0.08 * (time.elapsed_secs() * 0.7 + o.angle).sin()
        } else {
            1.0
        };
        let at = Vec2::from_angle(o.angle) * o.radius * breathe;
        transform.translation = at.extend(transform.translation.z);
    }
}

/// Setas na borda da tela para portais fora de vista (até 2,5 km): é o que
/// faz o jogador largar a rota e ir atrás da cerração.
#[allow(clippy::too_many_arguments)]
fn update_edge_markers(
    mut commands: Commands,
    docked: Res<crate::net::MyDocked>,
    known: Res<KnownPortals>,
    my_ship: Res<crate::net::MyShip>,
    ships: Query<&ShipVisual>,
    camera: Query<(&Camera, &GlobalTransform), With<Camera2d>>,
    mut markers: Query<(Entity, &EdgeMarker, &mut Node, &Children)>,
    mut texts: Query<&mut Text>,
    ui_scale: Res<UiScale>,
) {
    let Ok((camera, camera_transform)) = camera.get_single() else {
        return;
    };
    let Some(viewport) = camera.logical_viewport_size() else {
        return;
    };
    let me = my_ship
        .0
        .and_then(|id| ships.iter().find(|ship| ship.target.ship_id == id))
        .map(|ship| Vec2::new(ship.target.x, ship.target.y));
    let mut wanted: HashMap<u32, (Vec2, String)> = HashMap::new();
    // Atracado, a tela de porto é o mundo: nada de etiqueta de borda.
    let me = me.filter(|_| !docked.0);
    if let Some(me) = me {
        for portal in known.portals.values() {
            let at = Vec2::new(portal.x, portal.y);
            let distance = me.distance(at);
            if distance > 2500.0 {
                continue;
            }
            let Ok(screen) = camera.world_to_viewport(camera_transform, at.extend(0.0)) else {
                continue;
            };
            // Faixa livre entre os painéis do HUD (topo e rodapé ocupados;
            // o painel de vento desce até ~140 px, a manchete de evento até ~180
            // e o bilhete de ação sobe até ~135 px do rodapé).
            let area = Rect::new(60.0, 190.0, viewport.x - 60.0, viewport.y - 165.0);
            if area.contains(screen) {
                continue;
            }
            // O viewport é lógico; `Node` em px é multiplicado pelo UiScale.
            let pos = edge_position(screen, area) / ui_scale.0;
            let name = match portal.kind {
                PortalKindWire::FogGate => "Cerração",
                PortalKindWire::FogExit => "Saída",
                PortalKindWire::Whirlpool => "Sorvedouro",
            };
            wanted.insert(
                portal.portal_id,
                (
                    pos,
                    format!(
                        "{} {}m",
                        crate::i18n::tr(name).to_uppercase(),
                        distance as u32
                    ),
                ),
            );
        }
    }
    // Etiquetas na mesma borda empilham em vez de se cobrir.
    let mut placed: Vec<(u32, Vec2)> = wanted.iter().map(|(id, (pos, _))| (*id, *pos)).collect();
    stack_tags(&mut placed, viewport.y * 0.5 / ui_scale.0);
    for (id, pos) in placed {
        if let Some(entry) = wanted.get_mut(&id) {
            entry.0 = pos;
        }
    }
    for (entity, marker, mut node, children) in &mut markers {
        match wanted.remove(&marker.0) {
            Some((pos, label)) => {
                let (left, top) = (Val::Px(pos.x - 40.0), Val::Px(pos.y - 10.0));
                if node.left != left || node.top != top {
                    node.left = left;
                    node.top = top;
                }
                for child in children.iter() {
                    if let Ok(mut text) = texts.get_mut(*child) {
                        if text.0 != label {
                            text.0 = label.clone();
                        }
                    }
                }
            }
            None => commands.entity(entity).despawn_recursive(),
        }
    }
    for (id, (pos, label)) in wanted {
        // Etiqueta de papel: tinta preta, fio na cor do portal.
        let accent = known
            .portals
            .get(&id)
            .map(|p| portal_ink(p.kind))
            .unwrap_or(ui::INK);
        commands
            .spawn((
                EdgeMarker(id),
                ui::panel(Node {
                    position_type: PositionType::Absolute,
                    left: Val::Px(pos.x - 40.0),
                    top: Val::Px(pos.y - 10.0),
                    padding: UiRect::axes(Val::Px(7.0), Val::Px(2.0)),
                    ..default()
                }),
            ))
            // `insert` troca o fio do papel (um bundle com BorderColor
            // duplicado derruba o app).
            .insert(BorderColor(accent))
            .with_children(|panel| {
                panel.spawn(ui::face(label, ui::FONT_BOLD, 12.0, ui::INK));
            });
    }
}

/// Empurra cada etiqueta que colide com uma já posta para o centro da
/// tela: para baixo na metade de cima, para cima na de baixo (v54: descer
/// na borda de baixo jogava a etiqueta em cima do HUD). Etiquetas têm
/// ~200 px: perto na horizontal já colide.
fn stack_tags(placed: &mut [(u32, Vec2)], mid_y: f32) {
    const WIDTH: f32 = 240.0;
    const GAP: f32 = 32.0;
    // Das bordas para o centro: a de cima em ordem crescente, a de baixo
    // em ordem decrescente.
    placed.sort_by(|a, b| {
        let key = |p: &Vec2| if p.y <= mid_y { p.y } else { 2.0 * mid_y - p.y };
        key(&a.1).total_cmp(&key(&b.1)).then(a.0.cmp(&b.0))
    });
    for index in 1..placed.len() {
        let mut pos = placed[index].1;
        let step = if pos.y <= mid_y { GAP } else { -GAP };
        // ponytail: O(n²) com n = portais a 2,5 km (poucos); basta.
        while let Some(hit) = placed[..index]
            .iter()
            .find(|(_, other)| (pos.x - other.x).abs() < WIDTH && (pos.y - other.y).abs() < GAP)
        {
            pos.y = hit.1.y + step;
        }
        placed[index].1 = pos;
    }
}

/// Ponto na borda de `area` na direção de `screen` a partir do centro dela.
fn edge_position(screen: Vec2, area: Rect) -> Vec2 {
    let center = area.center();
    let dir = screen - center;
    let half = area.half_size();
    let scale = (half.x / dir.x.abs().max(1e-3)).min(half.y / dir.y.abs().max(1e-3));
    center + dir * scale
}

/// Gradiente radial (transparente no centro, opaco nas bordas) para a
/// névoa de visão.
fn fog_image() -> Image {
    const SIZE: u32 = 128;
    let mut data = Vec::with_capacity((SIZE * SIZE * 4) as usize);
    for y in 0..SIZE {
        for x in 0..SIZE {
            let dx = (x as f32 + 0.5) / SIZE as f32 - 0.5;
            let dy = (y as f32 + 0.5) / SIZE as f32 - 0.5;
            let r = (dx * dx + dy * dy).sqrt() * 2.0;
            let alpha = ((r - 0.35) / 0.45).clamp(0.0, 1.0);
            data.extend_from_slice(&[255, 255, 255, (alpha * alpha * 255.0) as u8]);
        }
    }
    Image::new(
        Extent3d {
            width: SIZE,
            height: SIZE,
            depth_or_array_layers: 1,
        },
        TextureDimension::D2,
        data,
        TextureFormat::Rgba8UnormSrgb,
        RenderAssetUsages::RENDER_WORLD,
    )
}

fn spawn_fog_overlay(mut commands: Commands, mut images: ResMut<Assets<Image>>) {
    let image = images.add(fog_image());
    commands.spawn((
        FogOverlay,
        ImageNode {
            image,
            color: Color::NONE,
            ..default()
        },
        Node {
            position_type: PositionType::Absolute,
            width: Val::Percent(100.0),
            height: Val::Percent(100.0),
            ..default()
        },
        // Abaixo do HUD (que nasce depois), acima do mundo.
        GlobalZIndex(-1),
    ));
    commands
        .spawn((
            CollapseTimer,
            ui::panel(Node {
                position_type: PositionType::Absolute,
                top: Val::Px(96.0),
                left: Val::Percent(50.0),
                margin: UiRect::left(Val::Px(-110.0)),
                width: Val::Px(220.0),
                justify_content: JustifyContent::Center,
                padding: UiRect::all(Val::Px(4.0)),
                ..default()
            }),
            Visibility::Hidden,
        ))
        .with_children(|panel| {
            panel.spawn(ui::text("", 13.0, ui::DANGER));
        });
}

/// Dentro da cerração a visão fecha (névoa branca); na Passagem do Sorvedouro, um
/// véu azulado. O cronômetro mostra quando a cerração colapsa.
#[allow(clippy::type_complexity, clippy::too_many_arguments)]
fn update_fog_and_timer(
    time: Res<Time>,
    zone: Res<CurrentZone>,
    known: Res<KnownPortals>,
    my_ship: Res<crate::net::MyShip>,
    ships: Query<&ShipVisual>,
    mut fog: Query<&mut ImageNode, With<FogOverlay>>,
    mut timer: Query<(&mut Visibility, &Children), With<CollapseTimer>>,
    mut texts: Query<&mut Text>,
) {
    let zone_name = zone.0.as_ref().map(|z| z.name.as_str()).unwrap_or("");
    let goal = match zone_name {
        FOG_ZONE => Color::srgba(0.86, 0.9, 0.93, 0.97),
        MAELSTROM_ZONE => Color::srgba(0.35, 0.45, 0.85, 0.55),
        _ => Color::srgba(1.0, 1.0, 1.0, 0.0),
    };
    let k = 1.0 - (-2.0 * time.delta_secs()).exp();
    for mut image in &mut fog {
        let current = image.color.to_srgba();
        let target = goal.to_srgba();
        // Chegou (a olho): para de escrever, senão o UI refaz tudo a cada frame.
        if (current.alpha - target.alpha).abs() < 0.002
            && (current.red - target.red).abs() < 0.002
            && (current.blue - target.blue).abs() < 0.002
        {
            continue;
        }
        image.color = Color::srgba(
            current.red + (target.red - current.red) * k,
            current.green + (target.green - current.green) * k,
            current.blue + (target.blue - current.blue) * k,
            current.alpha + (target.alpha - current.alpha) * k,
        );
    }

    let me = my_ship
        .0
        .and_then(|id| ships.iter().find(|ship| ship.target.ship_id == id))
        .map(|ship| Vec2::new(ship.target.x, ship.target.y));
    let exit = (zone_name == FOG_ZONE)
        .then(|| {
            known
                .portals
                .values()
                .filter(|p| p.kind == PortalKindWire::FogExit)
                .min_by(|a, b| {
                    let d = |p: &&PortalState| me.map_or(0.0, |m| m.distance(Vec2::new(p.x, p.y)));
                    d(a).total_cmp(&d(b))
                })
        })
        .flatten();
    for (mut visibility, children) in &mut timer {
        visibility.set_if_neq(if exit.is_some() {
            Visibility::Inherited
        } else {
            Visibility::Hidden
        });
        if let Some(exit) = exit {
            let left = remaining(&known, exit, time.elapsed_secs());
            for child in children.iter() {
                if let Ok(mut text) = texts.get_mut(*child) {
                    let label = format!("A CERRACAO SE DISSIPA EM {}", clock(left));
                    if text.0 != label {
                        text.0 = label;
                    }
                }
            }
        }
    }
}

/// Atravessou um portal: a câmera pula junto em vez de varrer o oceano.
fn snap_camera_on_teleport(
    my_ship: Res<crate::net::MyShip>,
    ships: Query<(&ShipVisual, &Transform), Without<Camera2d>>,
    mut camera: Query<&mut Transform, With<Camera2d>>,
) {
    let Some(my_id) = my_ship.0 else { return };
    let Some((_, ship)) = ships.iter().find(|(v, _)| v.target.ship_id == my_id) else {
        return;
    };
    let Ok(mut cam) = camera.get_single_mut() else {
        return;
    };
    let ship_at = ship.translation.truncate();
    if cam.translation.truncate().distance(ship_at) > 900.0 {
        cam.translation = ship_at.extend(cam.translation.z);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn three_tags_on_one_edge_never_overlap() {
        // A e C colidem mas não são vizinhos na ordem (B fica entre eles).
        let mut placed = vec![
            (1, Vec2::new(640.0, 190.0)),
            (2, Vec2::new(1060.0, 190.0)),
            (3, Vec2::new(700.0, 190.0)),
        ];
        stack_tags(&mut placed, 500.0);
        for (i, (_, a)) in placed.iter().enumerate() {
            for (_, b) in &placed[i + 1..] {
                assert!((a.x - b.x).abs() >= 240.0 || (a.y - b.y).abs() >= 32.0);
            }
        }
    }

    #[test]
    fn tags_on_the_bottom_edge_stack_upward_away_from_the_hud() {
        let bottom = 900.0;
        let mut placed = vec![
            (1, Vec2::new(640.0, bottom)),
            (2, Vec2::new(660.0, bottom)),
            (3, Vec2::new(700.0, bottom)),
        ];
        stack_tags(&mut placed, 500.0);
        assert!(placed.iter().all(|(_, p)| p.y <= bottom), "nunca desce");
        for (i, (_, a)) in placed.iter().enumerate() {
            for (_, b) in &placed[i + 1..] {
                assert!((a.x - b.x).abs() >= 240.0 || (a.y - b.y).abs() >= 32.0);
            }
        }
    }

    fn portal(kind: PortalKindWire, uses_left: Option<u32>) -> PortalState {
        PortalState {
            portal_id: 1,
            kind,
            x: 0.0,
            y: 0.0,
            radius: 35.0,
            expires_in_secs: 125.0,
            uses_left,
        }
    }

    #[test]
    fn labels_show_time_and_seats() {
        let label = portal_label(&portal(PortalKindWire::FogGate, Some(2)), 125.0);
        assert_eq!(label, "Cerração 2:05 · 2 vaga(s)");
        let exit = portal_label(&portal(PortalKindWire::FogExit, None), 61.0);
        assert_eq!(exit, "Saída da Cerração 1:01");
    }

    #[test]
    fn edge_marker_stays_inside_the_free_area() {
        let area = Rect::new(60.0, 120.0, 1220.0, 590.0);
        for screen in [
            Vec2::new(5000.0, 360.0),
            Vec2::new(-300.0, -900.0),
            Vec2::new(640.0, 4000.0),
        ] {
            let p = edge_position(screen, area);
            assert!(area.inflate(0.1).contains(p), "{p:?}");
        }
    }
}
