//! Cenário do vertical slice (MF-057D/E, MF-058). A geografia vem do
//! `WorldMap` — zonas, portos e terra são os mesmos do servidor; esta
//! camada só escolhe a arte. Mar e terra são um shader (`sea.wgsl`); portos,
//! vegetação e rochas são sprites do pack Scallywag por cima.

use bevy::prelude::*;
use bevy::render::render_resource::{AsBindGroup, ShaderRef};
use bevy::sprite::{Anchor, Material2d, Material2dPlugin};
use lightyear::prelude::ClientReceiveMessage;
use marvyr_domain_world::map::{FOG_SLOTS, PIRATE_PORT};
use marvyr_domain_world::{LandMass, RiskTier, WorldMap, ZoneShape};
use marvyr_protocol::PortalsUpdate;

use crate::assets::{building, deco, fort, layers, GameAssets};
use crate::zone::CurrentZone;

/// Discos de terra enviados ao shader por quadro — só os perto da câmera
/// (o mundo inteiro tem 160+ com as instâncias).
const MAX_LAND: usize = 64;
const MAX_SAFE: usize = 4;

// O derive `ShaderType` (encase) gera uma fn `check` por campo que o rustc
// acusa como código morto; o allow fica restrito a este módulo.
#[allow(dead_code)]
mod uniform {
    use super::{MAX_LAND, MAX_SAFE};
    use bevy::math::Vec4;
    use bevy::render::render_resource::ShaderType;

    #[derive(ShaderType, Debug, Clone)]
    pub struct SeaParams {
        pub land: [Vec4; MAX_LAND],
        pub safe: [Vec4; MAX_SAFE],
        pub info: Vec4,
        /// v32: Maré Sangrenta (xy centro, z raio, w força).
        pub blood: Vec4,
    }
}
pub use uniform::SeaParams;

#[derive(Asset, TypePath, AsBindGroup, Debug, Clone)]
pub struct SeaMaterial {
    #[uniform(0)]
    pub params: SeaParams,
}

impl Material2d for SeaMaterial {
    fn fragment_shader() -> ShaderRef {
        "shaders/sea.wgsl".into()
    }
}

/// Material do mar: o tom de perigo acompanha a zona e a terra (do
/// `ClientWorld`, quando chegar) acompanha a câmera.
#[derive(Resource)]
struct Sea {
    material: Handle<SeaMaterial>,
    streamed_at: Vec3,
}

/// Anel do portão: do tamanho do vão no paredão.
const GATE_RING_INNER: f32 = 200.0;
const GATE_RING_OUTER: f32 = 215.0;

/// O quadro do oceano anda com a câmera (MV-066): as zonas moram longe
/// umas das outras no plano, e o shader desenha pela posição de mundo.
#[derive(Component)]
struct OceanQuad;

fn follow_ocean(
    camera: Query<&Transform, (With<Camera2d>, Without<OceanQuad>)>,
    mut ocean: Query<&mut Transform, With<OceanQuad>>,
) {
    let (Ok(camera), Ok(mut ocean)) = (camera.get_single(), ocean.get_single_mut()) else {
        return;
    };
    ocean.translation.x = camera.translation.x;
    ocean.translation.y = camera.translation.y;
}

/// Mundo do servidor (MV-065): montado da seed que chega no handshake.
/// Ausente até a conexão — nada de geografia antes disso.
#[derive(Resource)]
pub struct ClientWorld(pub WorldMap);

/// Bandeira animada (porto ou navio): cor do atlas + fase própria.
#[derive(Component)]
pub struct WavingFlag {
    pub color: usize,
    pub phase: usize,
}

pub struct WorldVisualPlugin;

impl Plugin for WorldVisualPlugin {
    fn build(&self, app: &mut App) {
        app.add_plugins(Material2dPlugin::<SeaMaterial>::default())
            // O oceano já anima atrás do login; a terra vem com a seed.
            .add_systems(Startup, spawn_ocean)
            .add_systems(Update, spawn_world.run_if(resource_added::<ClientWorld>))
            .add_systems(
                Update,
                (
                    tint_sea_by_zone,
                    tint_blood_tide,
                    apply_arenas.before(stream_land),
                    stream_land,
                    animate_flags,
                    follow_ocean,
                ),
            );
    }
}

/// Paredão (borda de zona ou de instância) é penhasco, não ilha com palmeira.
fn is_cliff(mass: &LandMass) -> bool {
    mass.cliff
}

/// Parâmetros do shader com a terra que cabe na vista em `center`.
/// `extra`: terra fora do mapa base (ilhas ocultas já avistadas).
pub fn sea_params(map: &WorldMap, extra: &[LandMass], center: Vec2, view_radius: f32) -> SeaParams {
    let mut land = [Vec4::ZERO; MAX_LAND];
    // Mais perto primeiro: com paredão em volta da zona, a vista pode ter
    // mais discos que o shader comporta — os de longe ficam de fora.
    let gap = |mass: &&LandMass| center.distance(Vec2::new(mass.x, mass.y)) - mass.radius;
    let mut visible: Vec<&LandMass> = map
        .land()
        .iter()
        .chain(map.arena_land())
        .chain(extra)
        .filter(|mass| gap(mass) < view_radius)
        .collect();
    visible.sort_by(|a, b| gap(a).total_cmp(&gap(b)));
    let mut count = 0;
    for (slot, mass) in land.iter_mut().zip(visible) {
        *slot = Vec4::new(mass.x, mass.y, mass.radius, is_cliff(mass) as u8 as f32);
        count += 1;
    }
    let mut safe = [Vec4::ZERO; MAX_SAFE];
    let protected = map.zones().iter().filter(|z| z.tier == RiskTier::Protected);
    let mut safe_count = 0;
    for (slot, zone) in safe.iter_mut().zip(protected) {
        let ZoneShape::Circle { x, y, radius } = zone.shape;
        *slot = Vec4::new(x, y, radius, 0.0);
        safe_count += 1;
    }
    SeaParams {
        land,
        safe,
        info: Vec4::new(count as f32, safe_count as f32, 0.0, 0.0),
        blood: Vec4::ZERO,
    }
}

fn atlas(image: &Handle<Image>, layout: &Handle<TextureAtlasLayout>, index: usize) -> Sprite {
    Sprite::from_atlas_image(
        image.clone(),
        TextureAtlas {
            layout: layout.clone(),
            index,
        },
    )
}

fn prop(commands: &mut Commands, sprite: Sprite, at: Vec2, scale: f32, z: f32) {
    commands.spawn((
        sprite,
        Transform::from_translation(at.extend(z)).with_scale(Vec3::splat(scale)),
    ));
}

fn label(commands: &mut Commands, text: &str, at: Vec2, size: f32, color: Color) {
    commands.spawn((
        Text2d::new(crate::i18n::tr(text)),
        TextLayout::new_with_no_wrap(),
        TextFont {
            font_size: size,
            ..default()
        },
        TextColor(color),
        Anchor::Center,
        Transform::from_translation(at.extend(layers::LABELS)),
    ));
}

/// Distância com sinal até a terra (negativa dentro), como no shader mas
/// sem o ruído da costa — por isso quem usa pede folga.
fn land_distance(land: &[LandMass], p: Vec2) -> f32 {
    land.iter()
        .map(|m| p.distance(Vec2::new(m.x, m.y)) - m.radius)
        .fold(f32::MAX, f32::min)
}

// ponytail: o mundo nasce uma vez por sessão; reconectar num servidor com
// outra seed pede reiniciar o jogo (despawn do mundo se isso virar rotina).
fn spawn_ocean(
    mut commands: Commands,
    mut meshes: ResMut<Assets<Mesh>>,
    mut materials: ResMut<Assets<SeaMaterial>>,
) {
    // Mar aberto, sem terra, até o mapa chegar.
    let sea = materials.add(SeaMaterial {
        params: SeaParams {
            land: [Vec4::ZERO; MAX_LAND],
            safe: [Vec4::ZERO; MAX_SAFE],
            info: Vec4::ZERO,
            blood: Vec4::ZERO,
        },
    });
    // Cobre o mapa principal e as instâncias (cerrações a leste, Sorvedouro a oeste).
    commands.spawn((
        Mesh2d(meshes.add(Rectangle::new(11000.0, 6400.0))),
        MeshMaterial2d(sea.clone()),
        Transform::from_xyz(0.0, 300.0, layers::OCEAN),
        OceanQuad,
    ));
    commands.insert_resource(Sea {
        material: sea,
        streamed_at: Vec3::splat(f32::MAX),
    });
}

fn spawn_world(
    mut commands: Commands,
    assets: Res<GameAssets>,
    world: Res<ClientWorld>,
    sea: Option<ResMut<Sea>>,
    mut meshes: ResMut<Assets<Mesh>>,
    mut colors: ResMut<Assets<ColorMaterial>>,
) {
    let map = &world.0;
    // O shader recebe a terra nova no próximo quadro.
    if let Some(mut sea) = sea {
        sea.streamed_at = Vec3::splat(f32::MAX);
    }

    let ports: Vec<Vec2> = map
        .regions()
        .iter()
        .filter_map(|r| r.port.as_ref())
        .map(|p| Vec2::new(p.x, p.y))
        .collect();
    for port in map.regions().iter().filter_map(|r| r.port.as_ref()) {
        let dock = Vec2::new(port.x, port.y);
        let inland = inland_from(map.land(), dock);
        spawn_port(&mut commands, &assets, map.land(), port.name, dock, inland);
    }
    spawn_vegetation(&mut commands, &assets, map.land(), &ports);

    let danger = Color::srgb(1.0, 0.82, 0.78);
    for &(text, x, y) in &map.features().labels {
        label(&mut commands, text, Vec2::new(x, y), 16.0, danger);
    }
    spawn_gates(&mut commands, map, &mut meshes, &mut colors);
}

/// Portões de zona (MV-066): anel no vão do paredão e, do lado de dentro,
/// o nome da zona para onde ele leva.
fn spawn_gates(
    commands: &mut Commands,
    map: &WorldMap,
    meshes: &mut Assets<Mesh>,
    colors: &mut Assets<ColorMaterial>,
) {
    let features = map.features();
    let ring = meshes.add(Annulus::new(GATE_RING_INNER, GATE_RING_OUTER));
    let glow = colors.add(ColorMaterial::from(Color::srgba(0.85, 0.95, 1.0, 0.35)));
    for exit in &features.exits {
        let (from, to) = (&features.areas[exit.from], &features.areas[exit.to]);
        let gate = Vec2::new(exit.x, exit.y);
        let inward = (Vec2::new(from.x, from.y) - gate).normalize_or_zero();
        commands.spawn((
            Mesh2d(ring.clone()),
            MeshMaterial2d(glow.clone()),
            Transform::from_translation(gate.extend(layers::PROPS)),
        ));
        let text = format!("→ {}", crate::i18n::tr(to.name));
        commands.spawn((
            Text2d::new(text),
            TextLayout::new_with_no_wrap(),
            TextFont {
                font_size: 18.0,
                ..default()
            },
            TextColor(Color::srgb(0.95, 0.97, 1.0)),
            Anchor::Center,
            Transform::from_translation((gate + inward * 420.0).extend(layers::LABELS)),
        ));
    }
}

/// Miolo das cerrações abertas (MV-066): a mesma semente sorteia os mesmos
/// rochedos do servidor; muda o mapa e força o shader a recarregar a terra.
fn apply_arenas(
    mut events: EventReader<ClientReceiveMessage<PortalsUpdate>>,
    world: Option<ResMut<ClientWorld>>,
    sea: Option<ResMut<Sea>>,
    mut applied: Local<[Option<u64>; FOG_SLOTS.len()]>,
) {
    let (Some(event), Some(mut world)) = (events.read().last(), world) else {
        return;
    };
    if world.is_added() {
        // Mapa novo (reconexão) nasce sem miolo.
        *applied = Default::default();
    }
    let mut open = [None; FOG_SLOTS.len()];
    for &(slot, layout) in &event.message().arenas {
        if let Some(entry) = open.get_mut(usize::from(slot)) {
            *entry = Some(layout);
        }
    }
    if open == *applied {
        return;
    }
    for (slot, layout) in open.iter().enumerate() {
        if applied[slot] != *layout {
            world.0.set_arena(slot, *layout);
        }
    }
    *applied = open;
    if let Some(mut sea) = sea {
        sea.streamed_at = Vec3::splat(f32::INFINITY);
    }
}

/// Envia ao shader só a terra perto da câmera; refaz quando a câmera anda
/// ou o zoom muda o bastante.
fn stream_land(
    sea: Option<ResMut<Sea>>,
    world: Option<Res<ClientWorld>>,
    camera: Query<(&Transform, &OrthographicProjection), With<Camera2d>>,
    windows: Query<&Window>,
    mut materials: ResMut<Assets<SeaMaterial>>,
    seen: Option<Res<crate::seafaring::SeenIslands>>,
) {
    // Ilha oculta recém-avistada: refaz a terra mesmo com a câmera parada.
    let new_island = seen.as_ref().is_some_and(|seen| seen.is_changed());
    let (Some(mut sea), Some(world), Ok((transform, projection))) =
        (sea, world, camera.get_single())
    else {
        return;
    };
    let window = windows
        .get_single()
        .map(|w| Vec2::new(w.width(), w.height()))
        .unwrap_or(Vec2::new(1280.0, 720.0));
    let view = (
        transform.translation.x,
        transform.translation.y,
        projection.scale,
    );
    let moved = Vec2::new(view.0 - sea.streamed_at.x, view.1 - sea.streamed_at.y).length();
    if moved < 80.0 && (view.2 - sea.streamed_at.z).abs() < 0.05 && !new_island {
        return;
    }
    sea.streamed_at = Vec3::new(view.0, view.1, view.2);
    let view_radius = window.length() * 0.5 * projection.scale + 200.0;
    let hidden: Vec<LandMass> = seen
        .map(|seen| seen.0.values().map(crate::seafaring::island_land).collect())
        .unwrap_or_default();
    let fresh = sea_params(&world.0, &hidden, Vec2::new(view.0, view.1), view_radius);
    if let Some(material) = materials.get_mut(&sea.material) {
        let danger = material.params.info.z;
        let blood = material.params.blood;
        material.params = fresh;
        material.params.info.z = danger;
        material.params.blood = blood;
    }
}

/// Direção da terra a partir do cais: rumo ao disco de terra mais próximo
/// (a costa atrás da capital, o corpo da ilha atrás do porto pirata).
fn inland_from(land: &[LandMass], dock: Vec2) -> Vec2 {
    land.iter()
        .map(|m| Vec2::new(m.x, m.y))
        .min_by(|a, b| {
            dock.distance_squared(*a)
                .total_cmp(&dock.distance_squared(*b))
        })
        .map(|nearest| (nearest - dock).normalize_or(Vec2::NEG_X))
        .unwrap_or(Vec2::NEG_X)
}

/// Porto sobre a costa: cais de tábuas até a água, torres com bandeira,
/// carga no píer, lanternas e a vila atrás. Serra é madeira e verde; Mina é
/// pedra e canhão.
fn spawn_port(
    commands: &mut Commands,
    assets: &GameAssets,
    land: &[LandMass],
    name: &str,
    dock: Vec2,
    inland: Vec2,
) {
    let pirate = name == PIRATE_PORT;
    let mina = name.contains("Mina");
    let side = inland.perp();
    let at = |along: f32, across: f32| dock + inland * along + side * across;
    let facing = Quat::from_rotation_z(inland.to_angle());
    let fort_sprite = |index| atlas(&assets.fort, &assets.fort_parts, index);
    let deco_sprite = |index| atlas(&assets.water_and_islands, &assets.deco, index);

    // Cais: três seções de tábuas saindo da praia até a doca.
    for i in 0..3 {
        commands.spawn((
            fort_sprite(fort::DOCK),
            Transform::from_translation(at(18.0 + i as f32 * 31.0, 0.0).extend(layers::PROPS))
                .with_rotation(facing)
                .with_scale(Vec3::splat(0.5)),
        ));
    }
    // Plataforma de carga na praia.
    commands.spawn((
        fort_sprite(fort::BOARDWALK),
        Transform::from_translation(at(110.0, 0.0).extend(layers::PROPS))
            .with_rotation(facing)
            .with_scale(Vec3::splat(0.9)),
    ));

    // Torres: pedra na Mina (com canhões), madeira/pedra leve na Serra.
    let flag_color = if pirate {
        5
    } else if mina {
        4
    } else {
        2
    };
    let stone = mina || pirate;
    for (i, side_sign) in [-1.0_f32, 1.0].into_iter().enumerate() {
        let tower = at(70.0, side_sign * 46.0);
        let index = if stone {
            fort::TOWER
        } else {
            fort::TOWER_PLAIN
        };
        prop(
            commands,
            fort_sprite(index),
            tower,
            0.9,
            layers::PROPS + 0.1,
        );
        commands.spawn((
            fort_sprite(fort::FLAG + flag_color * 3),
            Transform::from_translation((tower + Vec2::new(4.0, 18.0)).extend(layers::PROPS + 0.3))
                .with_scale(Vec3::splat(1.6)),
            WavingFlag {
                color: flag_color,
                phase: i,
            },
        ));
        if stone {
            let mut cannon = fort_sprite(fort::CANNON);
            cannon.flip_x = inland.x > 0.0;
            prop(
                commands,
                cannon,
                tower - inland * 20.0,
                0.6,
                layers::PROPS + 0.2,
            );
        }
    }
    if mina {
        for side_sign in [-1.0_f32, 1.0] {
            for k in 0..3 {
                let block = at(98.0 + k as f32 * 28.0, side_sign * 62.0);
                prop(
                    commands,
                    fort_sprite(fort::WALL_BLOCK),
                    block,
                    0.9,
                    layers::PROPS,
                );
            }
        }
    }

    // Carga no píer: caixas e barris.
    for (offset, index, scale) in [
        (at(100.0, 10.0), fort::CRATE, 0.45),
        (at(118.0, -8.0), fort::CRATE, 0.4),
        (at(42.0, 9.0), fort::BARREL, 0.8),
        (at(48.0, -9.0), fort::BARREL, 0.8),
        (at(124.0, 14.0), fort::BARREL, 0.8),
    ] {
        prop(
            commands,
            fort_sprite(index),
            offset,
            scale,
            layers::PROPS + 0.2,
        );
    }
    // Lanternas na ponta do cais e bote amarrado.
    for side_sign in [-1.0_f32, 1.0] {
        prop(
            commands,
            deco_sprite(deco::LAMP),
            at(6.0, side_sign * 11.0),
            0.8,
            layers::PROPS + 0.3,
        );
    }
    prop(
        commands,
        deco_sprite(deco::ROWBOAT),
        at(30.0, -22.0),
        0.7,
        layers::PROPS,
    );

    // Vila: telhados vistos de cima, sem girar (a luz do sprite é fixa no
    // noroeste, como a do mar). Só onde há terra firme — a costa varia.
    let (house, tavern_roof) = if pirate {
        (building::HOUSE_THATCH, building::HOUSE_THATCH)
    } else if mina {
        (building::HOUSE_RED, building::TAVERN)
    } else {
        (building::HOUSE_THATCH, building::TAVERN)
    };
    for (spot, index) in [
        (at(150.0, -52.0), building::WAREHOUSE),
        (at(155.0, 58.0), tavern_roof),
        (at(128.0, 34.0), building::STALL),
        (at(196.0, -8.0), house),
        (at(205.0, 44.0), building::HOUSE_RED),
        (at(198.0, -92.0), house),
        (at(240.0, 16.0), building::HOUSE_RED),
    ] {
        if land_distance(land, spot) > -14.0 {
            continue;
        }
        commands.spawn((
            atlas(&assets.buildings, &assets.building_parts, index),
            Transform::from_translation(spot.extend(layers::PROPS + 0.4)),
        ));
    }

    label(
        commands,
        name,
        at(-38.0, 0.0),
        16.0,
        Color::srgb(1.0, 0.95, 0.8),
    );
}

/// Palmeiras e arbustos espalhados de forma determinística na faixa da
/// costa (longe da água e dos portos); rochas musgosas sobre os rochedos.
pub(crate) fn spawn_vegetation(
    commands: &mut Commands,
    assets: &GameAssets,
    land: &[LandMass],
    ports: &[Vec2],
) {
    let plants = [deco::PALM, deco::PALM_B, deco::BUSH, deco::BUSH_B];
    for (i, mass) in land.iter().enumerate() {
        let center = Vec2::new(mass.x, mass.y);
        if is_cliff(mass) {
            continue;
        }
        if mass.radius < 40.0 {
            let index = if i % 2 == 0 {
                deco::ROCK_MOSS
            } else {
                deco::ROCK_MOSS_B
            };
            prop(
                commands,
                atlas(&assets.water_and_islands, &assets.deco, index),
                center,
                mass.radius / 12.0,
                layers::PROPS,
            );
            continue;
        }
        // Espiral de ouro: pontos bem distribuídos dentro do disco.
        let count = (mass.radius / 7.0) as usize;
        for k in 0..count {
            let r = mass.radius * ((k as f32 + 0.5) / count as f32).sqrt();
            let angle = k as f32 * 2.399_963 + i as f32;
            let at = center + Vec2::from_angle(angle) * r;
            let clear_of_port = ports.iter().all(|port| port.distance(at) > 280.0);
            // Palmeira é da costa; o interior é mata desenhada no shader.
            let inland = -land_distance(land, at);
            if !(22.0..=70.0).contains(&inland) || !clear_of_port {
                continue;
            }
            let index = plants[(k + i) % plants.len()];
            // O verde do pack é neon perto da paleta do shader: tinta mais
            // sóbria para a palmeira casar com a mata.
            let mut plant = atlas(&assets.water_and_islands, &assets.deco, index);
            plant.color = Color::srgb(0.78, 0.86, 0.66);
            prop(commands, plant, at, 1.4, layers::PROPS);
        }
    }
}

/// Alto-mar sem lei escurece a água (transição suave ao cruzar a fronteira).
fn tint_sea_by_zone(
    time: Res<Time>,
    zone: Res<CurrentZone>,
    sea: Option<Res<Sea>>,
    mut materials: ResMut<Assets<SeaMaterial>>,
) {
    let Some(sea) = sea else { return };
    let sea = &sea.material;
    let goal = match zone.0.as_ref().map(|z| z.tier) {
        Some(RiskTier::Lawless) => 1.0,
        Some(RiskTier::Frontier) => 0.35,
        _ => 0.0,
    };
    let Some(material) = materials.get(sea) else {
        return;
    };
    let current = material.params.info.z;
    if (goal - current).abs() < 0.002 {
        return;
    }
    let next = current + (goal - current) * (1.0 - (-1.5 * time.delta_secs()).exp());
    if let Some(material) = materials.get_mut(sea) {
        material.params.info.z = next;
    }
}

/// v32: tinge a água da Maré Sangrenta no shader (sobe e desce suave).
fn tint_blood_tide(
    time: Res<Time>,
    events: Res<crate::seafaring::SeaEvents>,
    sea: Option<Res<Sea>>,
    mut materials: ResMut<Assets<SeaMaterial>>,
) {
    let Some(sea) = sea else { return };
    let tide = events
        .0
        .iter()
        .find(|event| event.kind == marvyr_protocol::SeaEventKind::BloodTide);
    let Some(current) = materials.get(&sea.material).map(|m| m.params.blood) else {
        return;
    };
    let goal = if tide.is_some() { 1.0 } else { 0.0 };
    if (goal - current.w).abs() < 0.002 && tide.is_none() {
        return;
    }
    let w = current.w + (goal - current.w) * (1.0 - (-1.2 * time.delta_secs()).exp());
    // Sem maré, a área antiga só esmaece (o centro fica onde estava).
    let (x, y, radius) = tide.map_or((current.x, current.y, current.z), |t| (t.x, t.y, t.radius));
    if let Some(material) = materials.get_mut(&sea.material) {
        material.params.blood = Vec4::new(x, y, radius, w);
    }
}

fn animate_flags(time: Res<Time>, mut flags: Query<(&WavingFlag, &mut Sprite)>) {
    let tick = (time.elapsed_secs() * 8.0) as usize;
    for (flag, mut sprite) in &mut flags {
        if let Some(atlas) = sprite.texture_atlas.as_mut() {
            atlas.index = fort::FLAG + flag.color * 3 + (tick + flag.phase) % 3;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn sea_params_stream_only_nearby_land() {
        let map = WorldMap::vertical_slice();
        let near_serra = sea_params(&map, &[], Vec2::new(-600.0, 0.0), 900.0);
        let count = near_serra.info.x as usize;
        assert!(count > 0 && count <= MAX_LAND);
        assert_eq!(near_serra.info.y, 2.0);
        // Nada das instâncias longínquas entra na vista do porto.
        assert!(near_serra.land[..count].iter().all(|disc| disc.w == 0.0));
        // Dentro de uma cerração, as paredes são penhasco.
        let (x, y) = marvyr_domain_world::map::FOG_SLOTS[1];
        let fog = sea_params(&map, &[], Vec2::new(x, y), 900.0);
        let count = fog.info.x as usize;
        assert!(count <= MAX_LAND);
        assert!(fog.land[..count].iter().any(|disc| disc.w == 1.0));
    }
}
