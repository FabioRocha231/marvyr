//! Carta náutica (MV-066): o que este capitão já navegou, por mundo. `M`
//! abre e fecha; o nevoeiro sai por onde o navio passa. Fica só no disco do
//! jogador (`<dados>/charts/<seed>.txt`) — carta é conhecimento, não poder.
//!
//! A carta desenha as zonas lado a lado (grade de `Area::cell`), não
//! nas coordenadas do plano, onde moram a milhares de metros umas das
//! outras.

use std::collections::HashSet;
use std::path::PathBuf;

use bevy::asset::RenderAssetUsages;
use bevy::prelude::*;
use bevy::render::render_resource::{Extent3d, TextureDimension, TextureFormat};
use marvyr_domain_world::{LandMass, RiskTier, WorldMap};

use crate::net::MyDocked;
use crate::session::ConnectionStatus;
use crate::ui;
use crate::world::ClientWorld;

/// Célula de revelação (m).
const CELL: f32 = 250.0;
/// Raio revelado em volta do navio (m).
const SIGHT: f32 = 700.0;
/// Resolução da carta (px por lado).
const TEX: u32 = 320;
/// ponytail: grava a cada 10 s — fechar o jogo perde no máximo isso.
const SAVE_EVERY: f32 = 10.0;
/// Faixa do paredão desenhada em volta de cada zona.
const WALL_BAND: f32 = 340.0;

type Cell = (i32, i32);

#[derive(Resource, Default)]
pub struct Chart {
    seed: Option<u64>,
    seen: HashSet<Cell>,
    dirty: bool,
    save_clock: f32,
}

#[derive(Component)]
struct ChartOverlay;

#[derive(Component)]
struct ChartShip;

pub struct ChartPlugin;

impl Plugin for ChartPlugin {
    fn build(&self, app: &mut App) {
        app.init_resource::<Chart>()
            .add_systems(Update, (track_chart, toggle_chart, move_ship_marker));
    }
}

fn cell_of(x: f32, y: f32) -> Cell {
    ((x / CELL).floor() as i32, (y / CELL).floor() as i32)
}

/// Revela as células a até `SIGHT` de (x, y). `true` se alguma era nova.
fn reveal(seen: &mut HashSet<Cell>, x: f32, y: f32) -> bool {
    let reach = (SIGHT / CELL).ceil() as i32;
    let (cx, cy) = cell_of(x, y);
    let mut added = false;
    for dx in -reach..=reach {
        for dy in -reach..=reach {
            if ((dx * dx + dy * dy) as f32) * CELL * CELL <= SIGHT * SIGHT {
                added |= seen.insert((cx + dx, cy + dy));
            }
        }
    }
    added
}

fn chart_path(seed: u64) -> PathBuf {
    crate::config::data_dir()
        .join("charts")
        .join(format!("{seed}.txt"))
}

/// Uma célula por linha, `x,y`. Linha estragada é ignorada, não derruba.
fn parse(raw: &str) -> HashSet<Cell> {
    raw.lines()
        .filter_map(|line| {
            let (x, y) = line.split_once(',')?;
            Some((x.trim().parse().ok()?, y.trim().parse().ok()?))
        })
        .collect()
}

fn serialize(seen: &HashSet<Cell>) -> String {
    seen.iter().map(|(x, y)| format!("{x},{y}\n")).collect()
}

fn save(seed: u64, seen: &HashSet<Cell>) {
    let path = chart_path(seed);
    let written = path
        .parent()
        .map_or(Ok(()), std::fs::create_dir_all)
        .and_then(|()| std::fs::write(&path, serialize(seen)));
    if let Err(error) = written {
        warn!(%error, path = %path.display(), "carta não foi gravada");
    }
}

/// Carrega a carta do mundo que chegou; revela em volta do navio no mar.
fn track_chart(
    time: Res<Time>,
    world: Option<Res<ClientWorld>>,
    status: Res<ConnectionStatus>,
    camera: Query<&Transform, With<Camera2d>>,
    mut chart: ResMut<Chart>,
) {
    let Some(world) = world else {
        return;
    };
    let seed = world.0.features().seed;
    if chart.seed != Some(seed) {
        // Mundo trocou (outro servidor): o que falta gravar é do anterior.
        if let (Some(previous), true) = (chart.seed, chart.dirty) {
            save(previous, &chart.seen);
        }
        let raw = std::fs::read_to_string(chart_path(seed)).unwrap_or_default();
        chart.seen = parse(&raw);
        chart.seed = Some(seed);
        chart.dirty = false;
    }
    if *status != ConnectionStatus::InGame {
        return;
    }
    // A câmera segue o navio: onde ela está, o capitão está vendo.
    if let Ok(camera) = camera.get_single() {
        let (x, y) = (camera.translation.x, camera.translation.y);
        if reveal(&mut chart.seen, x, y) {
            chart.dirty = true;
        }
    }
    chart.save_clock += time.delta_secs();
    if chart.dirty && chart.save_clock >= SAVE_EVERY {
        chart.save_clock = 0.0;
        chart.dirty = false;
        save(seed, &chart.seen);
    }
}

/// Passo da grade da carta: a maior zona (com paredão) mais uma folga, para
/// vizinhas nunca se sobreporem no desenho. Não é o `CHART_CELL` do
/// domínio, que mede distância de viagem (contratos).
fn grid_step(map: &WorldMap) -> f32 {
    let widest = map
        .features()
        .areas
        .iter()
        .map(|area| area.radius)
        .fold(0.0, f32::max);
    2.0 * (widest + WALL_BAND) + CELL
}

/// Centro da zona na carta.
fn cell_center(area: &marvyr_domain_world::Area, step: f32) -> Vec2 {
    Vec2::new(area.cell.0 as f32, area.cell.1 as f32) * step
}

/// Retângulo da carta (min, max) em coordenadas de carta.
fn chart_bounds(map: &WorldMap) -> (Vec2, Vec2) {
    let mut min = Vec2::splat(f32::MAX);
    let mut max = Vec2::splat(f32::MIN);
    let step = grid_step(map);
    for area in &map.features().areas {
        let center = cell_center(area, step);
        let reach = Vec2::splat(area.radius + WALL_BAND);
        min = min.min(center - reach);
        max = max.max(center + reach);
    }
    // Quadrado: a imagem é quadrada, a carta não se deforma.
    let side = (max - min).max_element();
    let middle = (min + max) / 2.0;
    (
        middle - Vec2::splat(side / 2.0),
        middle + Vec2::splat(side / 2.0),
    )
}

/// Posição (0..1, 0..1, y para baixo) do ponto de mundo na imagem da carta.
fn chart_uv(map: &WorldMap, bounds: (Vec2, Vec2), x: f32, y: f32) -> Vec2 {
    let step = grid_step(map);
    let (cx, cy) = match map.area_at(x, y).map(|index| &map.features().areas[index]) {
        Some(area) => (cell_center(area, step) + Vec2::new(x - area.x, y - area.y)).into(),
        None => (x, y),
    };
    let span = bounds.1 - bounds.0;
    Vec2::new((cx - bounds.0.x) / span.x, 1.0 - (cy - bounds.0.y) / span.y)
}

const FOG: [u8; 4] = [196, 182, 150, 255];
const OUTSIDE: [u8; 4] = [216, 199, 162, 255];
const CLIFF: [u8; 4] = [120, 116, 108, 255];
const SHORE: [u8; 4] = [164, 170, 104, 255];

fn water(tier: Option<RiskTier>) -> [u8; 4] {
    match tier {
        Some(RiskTier::Protected) => [120, 176, 190, 255],
        Some(RiskTier::Frontier) => [102, 150, 186, 255],
        _ => [86, 118, 160, 255],
    }
}

/// Pixels RGBA da carta: nevoeiro onde o capitão nunca foi, terra e água
/// (tingida pelo risco) onde foi.
fn render_chart(map: &WorldMap, seen: &HashSet<Cell>) -> Vec<u8> {
    let bounds = chart_bounds(map);
    let areas = &map.features().areas;
    // Só a terra de cada zona, para não testar o mundo inteiro por pixel.
    let land_by_area: Vec<Vec<LandMass>> = areas
        .iter()
        .map(|area| {
            map.land()
                .iter()
                .filter(|m| {
                    (m.x - area.x).hypot(m.y - area.y) <= area.radius + WALL_BAND + m.radius
                })
                .copied()
                .collect()
        })
        .collect();
    let step = grid_step(map);
    let scale = (bounds.1.x - bounds.0.x) / TEX as f32;
    let mut pixels = Vec::with_capacity((TEX * TEX * 4) as usize);
    for py in 0..TEX {
        for px in 0..TEX {
            let chart = Vec2::new(
                bounds.0.x + (px as f32 + 0.5) * scale,
                bounds.1.y - (py as f32 + 0.5) * scale,
            );
            let hit = areas.iter().enumerate().find_map(|(index, area)| {
                let center = cell_center(area, step);
                let offset = chart - center;
                (offset.length() <= area.radius + WALL_BAND)
                    .then(|| (index, Vec2::new(area.x, area.y) + offset))
            });
            let color = match hit {
                None => OUTSIDE,
                Some((_, world)) if !seen.contains(&cell_of(world.x, world.y)) => FOG,
                Some((index, world)) => {
                    match land_by_area[index]
                        .iter()
                        .find(|m| m.contains(world.x, world.y, 0.0))
                    {
                        Some(mass) if mass.cliff => CLIFF,
                        Some(_) => SHORE,
                        None => water(map.zone_at(world.x, world.y).ok().map(|z| z.tier)),
                    }
                }
            };
            pixels.extend_from_slice(&color);
        }
    }
    pixels
}

#[allow(clippy::too_many_arguments)]
fn toggle_chart(
    mut commands: Commands,
    keys: Res<ButtonInput<KeyCode>>,
    status: Res<ConnectionStatus>,
    docked: Res<MyDocked>,
    world: Option<Res<ClientWorld>>,
    chart: Res<Chart>,
    mut images: ResMut<Assets<Image>>,
    open: Query<Entity, With<ChartOverlay>>,
    (time, mut shot_at): (Res<Time>, Local<Option<Option<f32>>>),
    (lighthouses, currents): (
        Res<crate::lighthouse::KnownLighthouses>,
        Res<crate::currents::KnownCurrents>,
    ),
) {
    let close = |commands: &mut Commands| {
        for entity in &open {
            commands.entity(entity).despawn_recursive();
        }
    };
    // Atracado, M é letra do mercado; fora do mar não há carta.
    if docked.0 || *status != ConnectionStatus::InGame {
        close(&mut commands);
        return;
    }
    // Dev (captura de tela): MARVYR_SHOT_CHART=<s> abre a carta uma vez,
    // `s` segundos depois de o app abrir.
    // Lida uma vez; depois de disparar, vira `None` para sempre.
    let after = shot_at.get_or_insert_with(|| {
        std::env::var("MARVYR_SHOT_CHART")
            .ok()
            .and_then(|raw| raw.parse::<f32>().ok())
    });
    let shot = after.is_some_and(|after| time.elapsed_secs() >= after);
    if shot {
        *after = None;
    }
    if !keys.just_pressed(KeyCode::KeyM) && !shot {
        return;
    }
    if !open.is_empty() {
        close(&mut commands);
        return;
    }
    let Some(world) = world else {
        return;
    };
    let map = &world.0;
    let image = images.add(Image::new(
        Extent3d {
            width: TEX,
            height: TEX,
            depth_or_array_layers: 1,
        },
        TextureDimension::D2,
        render_chart(map, &chart.seen),
        TextureFormat::Rgba8UnormSrgb,
        RenderAssetUsages::RENDER_WORLD,
    ));
    let bounds = chart_bounds(map);
    commands
        .spawn((
            ChartOverlay,
            Node {
                width: Val::Percent(100.0),
                height: Val::Percent(100.0),
                position_type: PositionType::Absolute,
                flex_direction: FlexDirection::Column,
                align_items: AlignItems::Center,
                justify_content: JustifyContent::Center,
                ..default()
            },
            BackgroundColor(Color::srgba(0.02, 0.05, 0.09, 0.55)),
            GlobalZIndex(30),
        ))
        .with_children(|root| {
            root.spawn(ui::panel(Node {
                padding: UiRect::all(Val::Px(8.0)),
                flex_direction: FlexDirection::Column,
                align_items: AlignItems::Center,
                row_gap: Val::Px(6.0),
                ..default()
            }))
            .with_children(|frame| {
                frame.spawn(ui::display("Carta Náutica", 24.0, ui::INK));
                frame
                    .spawn((
                        ImageNode::new(image),
                        Node {
                            width: Val::Vh(62.0),
                            height: Val::Vh(62.0),
                            ..default()
                        },
                    ))
                    .with_children(|sheet| {
                        for region in map.regions() {
                            let Some(port) = &region.port else {
                                continue;
                            };
                            if !chart.seen.contains(&cell_of(port.x, port.y)) {
                                continue;
                            }
                            let at = chart_uv(map, bounds, port.x, port.y);
                            sheet.spawn((
                                Node {
                                    position_type: PositionType::Absolute,
                                    left: Val::Percent(at.x * 100.0),
                                    top: Val::Percent(at.y * 100.0),
                                    ..default()
                                },
                                ui::text(port.name, 13.0, ui::INK),
                            ));
                        }
                        // v48: correntes da semana, pontilhadas no sentido.
                        for (from, to) in &currents.0 {
                            for step in 0..12 {
                                let p = from.lerp(*to, step as f32 / 11.0);
                                let at = chart_uv(map, bounds, p.x, p.y);
                                let size = 3.0 + step as f32 * 0.3;
                                sheet.spawn((
                                    Node {
                                        position_type: PositionType::Absolute,
                                        left: Val::Percent(at.x * 100.0),
                                        top: Val::Percent(at.y * 100.0),
                                        width: Val::Px(size),
                                        height: Val::Px(size),
                                        margin: UiRect::all(Val::Px(-size / 2.0)),
                                        ..default()
                                    },
                                    BackgroundColor(Color::srgb(0.2, 0.55, 0.8)),
                                    BorderRadius::all(Val::Px(size / 2.0)),
                                ));
                            }
                        }
                        // v46: faróis de jogador, acesos para todo mundo.
                        for lighthouse in &lighthouses.0 {
                            let at = chart_uv(map, bounds, lighthouse.x, lighthouse.y);
                            sheet.spawn((
                                Node {
                                    position_type: PositionType::Absolute,
                                    left: Val::Percent(at.x * 100.0),
                                    top: Val::Percent(at.y * 100.0),
                                    width: Val::Px(8.0),
                                    height: Val::Px(8.0),
                                    margin: UiRect::all(Val::Px(-4.0)),
                                    ..default()
                                },
                                BackgroundColor(Color::srgb(1.0, 0.86, 0.42)),
                                BorderColor(ui::INK),
                                BorderRadius::all(Val::Px(4.0)),
                            ));
                        }
                        sheet.spawn((
                            ChartShip,
                            Node {
                                position_type: PositionType::Absolute,
                                width: Val::Px(10.0),
                                height: Val::Px(10.0),
                                margin: UiRect::all(Val::Px(-5.0)),
                                ..default()
                            },
                            BackgroundColor(ui::VERMILION),
                            BorderRadius::all(Val::Px(5.0)),
                        ));
                    });
                frame.spawn(ui::text("M fecha a carta", 14.0, ui::INK_SOFT));
            });
        });
}

fn move_ship_marker(
    world: Option<Res<ClientWorld>>,
    camera: Query<&Transform, With<Camera2d>>,
    mut marker: Query<&mut Node, With<ChartShip>>,
) {
    let (Some(world), Ok(camera), Ok(mut node)) =
        (world, camera.get_single(), marker.get_single_mut())
    else {
        return;
    };
    let at = chart_uv(
        &world.0,
        chart_bounds(&world.0),
        camera.translation.x,
        camera.translation.y,
    );
    node.left = Val::Percent(at.x * 100.0);
    node.top = Val::Percent(at.y * 100.0);
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn sailing_reveals_a_disc_and_the_chart_round_trips_through_text() {
        let mut seen = HashSet::new();
        assert!(reveal(&mut seen, 0.0, 0.0));
        assert!(!reveal(&mut seen, 10.0, 10.0), "mesma vizinhança");
        assert!(seen.contains(&cell_of(SIGHT - 1.0, 0.0)));
        assert!(!seen.contains(&cell_of(SIGHT + CELL * 2.0, 0.0)));
        assert_eq!(parse(&serialize(&seen)), seen);
        assert_eq!(
            parse("1,2\nlixo\n3,x\n-4,5"),
            HashSet::from([(1, 2), (-4, 5)])
        );
    }

    #[test]
    fn unexplored_sea_is_fog_and_explored_sea_is_water() {
        let map = WorldMap::from_seed(7);
        let spawn = map.features().spawn;
        let blank = render_chart(&map, &HashSet::new());
        let mut seen = HashSet::new();
        reveal(&mut seen, spawn.0, spawn.1);
        let explored = render_chart(&map, &seen);
        let at = chart_uv(&map, chart_bounds(&map), spawn.0, spawn.1);
        let (px, py) = ((at.x * TEX as f32) as usize, (at.y * TEX as f32) as usize);
        let pixel = |pixels: &[u8]| {
            let i = (py * TEX as usize + px) * 4;
            [pixels[i], pixels[i + 1], pixels[i + 2], pixels[i + 3]]
        };
        assert_eq!(pixel(&blank), FOG);
        assert_ne!(pixel(&explored), FOG);
        assert_eq!(blank.len(), (TEX * TEX * 4) as usize);
    }

    #[test]
    fn zones_never_overlap_on_the_chart() {
        for seed in 1..40 {
            let map = WorldMap::from_seed(seed);
            let step = grid_step(&map);
            let areas = &map.features().areas;
            for (i, a) in areas.iter().enumerate() {
                for b in &areas[i + 1..] {
                    let gap = cell_center(a, step).distance(cell_center(b, step));
                    assert!(
                        gap > a.radius + b.radius + 2.0 * WALL_BAND,
                        "seed {seed}: {} x {}",
                        a.name,
                        b.name
                    );
                }
            }
        }
    }
}
