//! Visual dos navios e projéteis replicados (PRD MF-006/009). Cada estado do
//! snapshot tem uma entidade visual aqui; o client faz lerp suave entre
//! snapshots. Sem predição local — a verdade é o servidor (Pilar 4).
//!
//! AOI (MF-031): entidades que SAEM do snapshot não somem na hora — ficam
//! como last-known-state por [`STALE_VISUAL_TTL`] segundos e só então o
//! visual sai. Wrecks chegam pelo snapshot (protocolo v8), com o mesmo TTL.
//!
//! MF-058: o navio é montado peça a peça do pack modular (casco, verga,
//! vela, cesto, bandeira). A vela enche com o seguimento, o casco racha
//! abaixo de 50% de HP, a proa levanta onda e a popa deixa espuma.

use std::collections::HashSet;
// web-time no browser (o `std` dá panic no wasm); `std` no nativo.
use bevy::utils::Instant;

use bevy::ecs::prelude::*;
use bevy::prelude::*;
use lightyear::prelude::ClientReceiveMessage;
use marvyr_domain_ships::{cosmetic_by_code, CosmeticSlot, ShipKind};
use marvyr_protocol::{Faction, ProjectileState, ShipState, WorldSnapshot, TIER_PROCURADO};

use crate::assets::{deco, fort, layers, parts, GameAssets, HullSize};
use crate::vfx::{spawn_animation, spawn_particle, Particle, VfxHandles};
use crate::world::WavingFlag;

/// Quanto tempo um visual sobrevive sem aparecer no snapshot (ADR-0009:
/// last-known-state até expirar). Curto de propósito: é máscara de pop do
/// AOI, não verdade sobre o mundo.
pub const STALE_VISUAL_TTL: f32 = 2.0;

/// Distância que só um portal explica (nenhum casco anda isso num snapshot).
const TELEPORT_SNAP: f32 = 150.0;

/// Metros por pixel do atlas. O casco médio (80 px) vira 40 m.
pub const WORLD_PER_PX: f32 = 0.5;

/// A proa dos cascos do atlas aponta para BAIXO na imagem (o gurupés fica
/// embaixo e a vela enfunada abre para baixo). `heading` 0 = +X, então o
/// sprite gira +90°: -Y local vira +X.
const SHIP_HEADING_OFFSET: f32 = std::f32::consts::FRAC_PI_2;

/// Entidade visual de um navio autoritativo. `target` é o último estado
/// autoritativo conhecido (alvo do lerp visual).
#[derive(Component)]
pub struct ShipVisual {
    pub target: ShipState,
    pub last_seen: Instant,
}

#[derive(Component)]
pub struct ShipHull {
    size: HullSize,
    color: usize,
}

#[derive(Component)]
pub struct ShipSail {
    size: HullSize,
    color: usize,
}

#[derive(Component)]
pub struct BowWave;

/// Bandeira do navio: a cor da facção, a menos que o capitão use outra.
#[derive(Component)]
pub struct ShipFlag {
    base: usize,
}

/// Cor do atlas vinda do cosmético à mostra (MV-066: só aparência).
fn cosmetic_color(code: u8, slot: CosmeticSlot) -> Option<usize> {
    cosmetic_by_code(code)
        .filter(|cosmetic| cosmetic.slot == slot)
        .map(|cosmetic| usize::from(cosmetic.color))
}

/// Casco afundando depois do `ShipDestroyed`: aderna, afunda e some.
#[derive(Component, Default)]
pub struct Sinking {
    age: f32,
    /// O `ShipVisual` sai no mesmo comando; o id fica para o juice.
    pub ship_id: u32,
}

impl Sinking {
    pub fn of(ship_id: u32) -> Self {
        Self { age: 0.0, ship_id }
    }
}

/// Acumulador de espuma de popa por navio.
#[derive(Component, Default)]
pub struct FoamEmitter(f32);

/// Aparência de cada tipo: casco, cores e posição dos mastros (px locais,
/// +Y = popa).
#[derive(Clone, Copy)]
struct ShipLook {
    hull: HullSize,
    hull_color: usize,
    sail_color: usize,
    trim_color: usize,
    masts: &'static [f32],
}

/// O casco (tamanho, mastros) vem do tipo; as cores vêm da bandeira, para
/// pirata, marinha e mercador se reconhecerem de longe. Jogador mantém as
/// cores do tipo.
fn ship_look(kind: ShipKind, faction: Faction) -> ShipLook {
    let base = match kind {
        // Cargueiro bojudo: casco médio claro, vela creme, dois mastros.
        ShipKind::SmallMerchant => ShipLook {
            hull: HullSize::Medium,
            hull_color: 1,
            sail_color: 1,
            trim_color: 1,
            masts: &[14.0, -12.0],
        },
        // Escolta: casco grande azul-marinho, três mastros, velas brancas.
        ShipKind::Patrol => ShipLook {
            hull: HullSize::Large,
            hull_color: 3,
            sail_color: 0,
            trim_color: 4,
            masts: &[30.0, 0.0, -30.0],
        },
        // Interceptador: casco pequeno, um mastro, rápido.
        ShipKind::Corsair => ShipLook {
            hull: HullSize::Small,
            hull_color: 2,
            sail_color: 2,
            trim_color: 1,
            masts: &[2.0],
        },
    };
    match faction {
        Faction::Player => base,
        // Casco escuro, vela vermelha.
        Faction::Pirate => ShipLook {
            hull_color: 0,
            sail_color: 5,
            ..base
        },
        // Azul-marinho, vela branca, verga clara (prata).
        Faction::Navy => ShipLook {
            hull_color: 3,
            sail_color: 0,
            trim_color: 0,
            ..base
        },
        // Madeira clara, vela creme.
        Faction::Merchant => ShipLook {
            hull_color: 1,
            sail_color: 1,
            ..base
        },
        // MV-061: o Kraken é casco escuro sem mastro — os tentáculos são
        // desenhados por cima (`seafaring::draw_sea_marks`).
        Faction::Monster => ShipLook {
            hull_color: 0,
            sail_color: 5,
            masts: &[],
            ..base
        },
    }
}

/// Cor da bandeira no atlas (creme, verde, dourada, azul, vermelha,
/// branca): dourada no próprio navio, vermelha no pirata, azul na marinha,
/// branca nos mercadores e nos demais jogadores.
fn flag_color(faction: Faction, mine: bool) -> usize {
    match faction {
        Faction::Pirate | Faction::Monster => 4,
        Faction::Navy => 3,
        Faction::Merchant => 5,
        Faction::Player if mine => 2,
        Faction::Player => 5,
    }
}

fn hull_px(size: HullSize) -> Vec2 {
    match size {
        HullSize::Small => Vec2::new(30.0, 64.0),
        HullSize::Medium => Vec2::new(44.0, 80.0),
        HullSize::Large => Vec2::new(46.0, 128.0),
    }
}

/// Comprimento do casco em metros (usado por VFX e HUD).
pub fn hull_length(kind: ShipKind) -> f32 {
    hull_px(ship_look(kind, Faction::Player).hull).y * WORLD_PER_PX
}

fn part_sprite(assets: &GameAssets, index: usize) -> Sprite {
    Sprite::from_atlas_image(
        assets.ships.clone(),
        TextureAtlas {
            layout: assets.ship_parts.clone(),
            index,
        },
    )
}

fn damaged(state: &ShipState) -> bool {
    state.max_hp > 0 && state.hp * 2 < state.max_hp
}

fn sail_full(state: &ShipState) -> bool {
    state.speed > state.max_speed.max(1.0) * 0.25
}

/// Altura do mastro principal (o do meio); `None` em casco sem mastro.
fn main_mast(look: &ShipLook) -> Option<f32> {
    look.masts.get(look.masts.len() / 2).copied()
}

/// Navios que já afundaram: snapshots em voo não podem ressuscitá-los.
#[derive(Resource, Debug, Default)]
pub struct DestroyedShips(pub HashSet<u32>);

fn spawn_ship(commands: &mut Commands, assets: &GameAssets, state: &ShipState, mine: bool) {
    let look = ship_look(state.kind, state.faction);
    let size = hull_px(look.hull);
    let mut entity = commands.spawn((
        ShipVisual {
            target: *state,
            last_seen: Instant::now(),
        },
        FoamEmitter::default(),
        Transform {
            translation: Vec3::new(state.x, state.y, layers::SHIPS),
            rotation: Quat::from_rotation_z(state.heading + SHIP_HEADING_OFFSET),
            scale: Vec3::splat(WORLD_PER_PX),
        },
        Visibility::default(),
    ));
    entity.with_children(|ship| {
        // Sombra: silhueta do próprio casco, escura e deslocada — dá
        // altura ao navio sobre a água.
        ship.spawn((
            Sprite {
                color: Color::srgba(0.0, 0.04, 0.1, 0.3),
                ..part_sprite(assets, parts::hull(look.hull, look.hull_color, false))
            },
            Transform::from_xyz(4.0, -5.0, -0.3),
        ));
        ship.spawn((
            part_sprite(
                assets,
                parts::hull(look.hull, look.hull_color, damaged(state)),
            ),
            Transform::from_xyz(0.0, 0.0, 0.0),
            ShipHull {
                size: look.hull,
                color: look.hull_color,
            },
        ));
        for (i, mast_y) in look.masts.iter().enumerate() {
            let z = 0.1 + i as f32 * 0.05;
            ship.spawn((
                part_sprite(assets, parts::yard(look.hull, look.trim_color)),
                Transform::from_xyz(0.0, *mast_y + 3.0, z),
            ));
            ship.spawn((
                part_sprite(
                    assets,
                    parts::sail(
                        look.hull,
                        cosmetic_color(state.sail_cosmetic, CosmeticSlot::Sail)
                            .unwrap_or(look.sail_color),
                        sail_full(state),
                    ),
                ),
                Transform::from_xyz(0.0, *mast_y - 2.0, z + 0.01),
                ShipSail {
                    size: look.hull,
                    color: look.sail_color,
                },
            ));
        }
        // Cesto de gávea no mastro principal dos cascos maiores.
        // O Kraken não tem mastro: sem cesto, bandeira no centro do casco.
        let main_mast = main_mast(&look);
        if let Some(mast_y) = main_mast.filter(|_| look.hull != HullSize::Small) {
            ship.spawn((
                part_sprite(assets, parts::nest(look.trim_color)),
                Transform::from_xyz(0.0, mast_y + 4.0, 0.4).with_scale(Vec3::splat(0.6)),
            ));
        }
        let main_mast = main_mast.unwrap_or(0.0);
        let base_flag = flag_color(state.faction, mine);
        let flag_color =
            cosmetic_color(state.flag_cosmetic, CosmeticSlot::Flag).unwrap_or(base_flag);
        ship.spawn((
            Sprite::from_atlas_image(
                assets.fort.clone(),
                TextureAtlas {
                    layout: assets.fort_parts.clone(),
                    index: fort::FLAG + flag_color * 3,
                },
            ),
            Transform::from_xyz(7.0, main_mast + 8.0, 0.5).with_scale(Vec3::splat(1.6)),
            WavingFlag {
                color: flag_color,
                phase: state.ship_id as usize,
            },
            ShipFlag { base: base_flag },
        ));
        // Onda de proa, um sprite por bordo (o outro espelhado).
        for flip in [false, true] {
            let side = if flip { 1.0 } else { -1.0 };
            ship.spawn((
                Sprite {
                    flip_x: flip,
                    color: Color::srgba(1.0, 1.0, 1.0, 0.0),
                    ..part_sprite(assets, parts::BOW_WAVE)
                },
                Transform::from_xyz(side * (size.x * 0.5 - 2.0), -size.y * 0.5 + 22.0, -0.2),
                BowWave,
            ));
        }
    });
    info!(ship_id = state.ship_id, kind = ?state.kind, "navio visível no horizonte");
}

#[allow(clippy::too_many_arguments)]
pub fn upsert_ship_visuals(
    mut commands: Commands,
    my_ship: Res<crate::net::MyShip>,
    destroyed: Res<DestroyedShips>,
    mut snapshot_events: EventReader<ClientReceiveMessage<WorldSnapshot>>,
    mut existing: Query<(Entity, &mut ShipVisual)>,
    assets: Res<GameAssets>,
) {
    // Antes do AssignShip não sabemos qual navio é nosso; processar snapshot
    // agora pintaria o próprio navio com a bandeira errada (decide no spawn).
    if my_ship.0.is_none() {
        return;
    }
    // Só o snapshot mais recente importa: eventos antigos nasceram velhos e
    // processar vários no mesmo frame spawnaria duplicatas (spawn é deferido).
    let Some(event) = snapshot_events.read().last() else {
        return;
    };
    let mut seen_ship_ids = HashSet::new();
    for state in &event.message().ships {
        // `Commands` aplica spawns depois do sistema: sem este filtro, uma
        // repetição do mesmo estado no snapshot agenda vários visuais.
        if !seen_ship_ids.insert(state.ship_id) || destroyed.0.contains(&state.ship_id) {
            continue;
        }
        if let Some((_, mut visual)) = existing
            .iter_mut()
            .find(|(_, visual)| visual.target.ship_id == state.ship_id)
        {
            // Tomou dano desde o último snapshot: fogo e lascas no casco.
            if state.hp < visual.target.hp {
                let at = Vec2::new(state.x, state.y);
                spawn_animation(&mut commands, &assets, parts::FIRE, 4, at, 1.3);
                for i in 0..6 {
                    let angle = i as f32 * 1.05 + state.heading;
                    spawn_particle(
                        &mut commands,
                        at,
                        Particle::debris(Vec2::from_angle(angle) * 14.0),
                    );
                }
            }
            visual.target = *state;
            visual.last_seen = Instant::now();
            continue;
        }
        spawn_ship(
            &mut commands,
            &assets,
            state,
            my_ship.0 == Some(state.ship_id),
        );
    }
}

/// Casco, vela, bandeira e ondas acompanham o estado autoritativo.
#[allow(clippy::type_complexity)]
pub fn animate_ship_parts(
    time: Res<Time>,
    ships: Query<(&ShipVisual, &Children)>,
    mut hulls: Query<(&ShipHull, &mut Sprite), (Without<ShipSail>, Without<BowWave>)>,
    mut sails: Query<(&ShipSail, &mut Sprite), (Without<ShipHull>, Without<BowWave>)>,
    mut waves: Query<&mut Sprite, (With<BowWave>, Without<ShipHull>, Without<ShipSail>)>,
    mut flags: Query<(&ShipFlag, &mut WavingFlag)>,
) {
    let t = time.elapsed_secs();
    for (visual, children) in &ships {
        let state = &visual.target;
        let speed_ratio = (state.speed / state.max_speed.max(1.0)).clamp(0.0, 1.0);
        for child in children.iter() {
            if let Ok((hull, mut sprite)) = hulls.get_mut(*child) {
                set_index(
                    &mut sprite,
                    parts::hull(hull.size, hull.color, damaged(state)),
                );
            } else if let Ok((sail, mut sprite)) = sails.get_mut(*child) {
                let color =
                    cosmetic_color(state.sail_cosmetic, CosmeticSlot::Sail).unwrap_or(sail.color);
                set_index(&mut sprite, parts::sail(sail.size, color, sail_full(state)));
            } else if let Ok((flag, mut waving)) = flags.get_mut(*child) {
                // Troca de visual no porto aparece sem recriar o navio.
                waving.color =
                    cosmetic_color(state.flag_cosmetic, CosmeticSlot::Flag).unwrap_or(flag.base);
            } else if let Ok(mut sprite) = waves.get_mut(*child) {
                let frame = ((t * 7.0) as usize + state.ship_id as usize) % 3;
                set_index(&mut sprite, parts::BOW_WAVE + frame);
                sprite.color = Color::srgba(1.0, 1.0, 1.0, (speed_ratio * 1.4).min(0.9));
            }
        }
    }
}

fn set_index(sprite: &mut Sprite, index: usize) {
    if let Some(atlas) = sprite.texture_atlas.as_mut() {
        if atlas.index != index {
            atlas.index = index;
        }
    }
}

/// Espuma de popa: partículas brancas que abrem e somem atrás do casco.
pub fn emit_foam(
    mut commands: Commands,
    time: Res<Time>,
    mut ships: Query<(&ShipVisual, &Transform, &mut FoamEmitter)>,
) {
    let dt = time.delta_secs();
    for (visual, transform, mut emitter) in &mut ships {
        let state = &visual.target;
        if state.speed < 1.5 {
            continue;
        }
        emitter.0 += dt * (4.0 + state.speed * 0.5);
        let back = -Vec2::from_angle(state.heading);
        let stern = transform.translation.truncate() + back * hull_length(state.kind) * 0.45;
        while emitter.0 >= 1.0 {
            emitter.0 -= 1.0;
            let jitter = Vec2::new(rand_unit(emitter.0 + stern.x), rand_unit(stern.y)) * 2.0;
            spawn_particle(&mut commands, stern + jitter, Particle::foam(back * 3.0));
        }
    }
}

/// Pseudo-aleatório barato e determinístico em [-1, 1] (só para jitter).
fn rand_unit(seed: f32) -> f32 {
    ((seed * 12.9898).sin() * 43_758.547).fract() * 2.0 - 1.0
}

/// Last-known-state (MF-031): visual que parou de aparecer no snapshot
/// permanece por TTL curto e depois sai — sem pop brusco na fronteira do AOI.
pub fn expire_stale_visuals(
    mut commands: Commands,
    ships: Query<(Entity, &ShipVisual)>,
    wrecks: Query<(Entity, &WreckVisual)>,
) {
    for (entity, visual) in &ships {
        if visual.last_seen.elapsed().as_secs_f32() > STALE_VISUAL_TTL {
            commands.entity(entity).despawn_recursive();
        }
    }
    for (entity, visual) in &wrecks {
        if visual.last_seen.elapsed().as_secs_f32() > STALE_VISUAL_TTL {
            commands.entity(entity).despawn_recursive();
        }
    }
}

/// Naufrágio: aderna, afunda (encolhe e escurece) e sai de cena.
pub fn animate_sinking(
    mut commands: Commands,
    time: Res<Time>,
    assets: Res<GameAssets>,
    mut ships: Query<(Entity, &mut Sinking, &mut Transform, &Children)>,
    mut sprites: Query<&mut Sprite>,
) {
    const DURATION: f32 = 2.6;
    let dt = time.delta_secs();
    for (entity, mut sinking, mut transform, children) in &mut ships {
        if sinking.age == 0.0 {
            let at = transform.translation.truncate();
            spawn_animation(&mut commands, &assets, parts::FIRE, 4, at, 2.0);
            spawn_animation(&mut commands, &assets, parts::SMOKE, 4, at, 2.4);
        }
        sinking.age += dt;
        let k = (sinking.age / DURATION).min(1.0);
        transform.rotate_z(dt * 0.35);
        transform.scale = Vec3::splat(WORLD_PER_PX * (1.0 - 0.35 * k));
        for child in children.iter() {
            if let Ok(mut sprite) = sprites.get_mut(*child) {
                let alpha = sprite.color.alpha().min(1.0 - k);
                sprite.color = Color::srgba(0.55, 0.62, 0.75, alpha);
            }
        }
        if k >= 1.0 {
            commands.entity(entity).despawn_recursive();
        }
    }
}

pub fn lerp_ship_visuals(time: Res<Time>, mut ships: Query<(&mut Transform, &ShipVisual)>) {
    // Fator por segundo independente de framerate; ~90% da diferença
    // desaparece em ~0.15s, o bastante para disfarçar 30 Hz sem atrasar.
    let factor = 1.0 - (-20.0 * time.delta_secs()).exp();
    for (mut transform, visual) in &mut ships {
        // Salto de portal (MF-059): reaparece do outro lado, sem deslizar
        // pelo mapa.
        let target = Vec2::new(visual.target.x, visual.target.y);
        if transform.translation.truncate().distance(target) > TELEPORT_SNAP {
            transform.translation = target.extend(transform.translation.z);
        }
        apply_lerp(
            &mut transform,
            visual.target.x,
            visual.target.y,
            visual.target.heading,
            factor,
            SHIP_HEADING_OFFSET,
        );
    }
}

/// Faixa de tiro dos bordos do próprio navio (MF-058): mostra por onde a
/// salva vai passar e se o canhão daquele lado está pronto.
pub fn draw_broadside_lanes(
    mut gizmos: Gizmos,
    my_ship: Res<crate::net::MyShip>,
    docked: Res<crate::net::MyDocked>,
    ships: Query<(&ShipVisual, &Transform)>,
) {
    if docked.0 {
        return;
    }
    let Some((visual, transform)) = ships
        .iter()
        .find(|(visual, _)| Some(visual.target.ship_id) == my_ship.0)
    else {
        return;
    };
    let state = &visual.target;
    let center = transform.translation.truncate();
    let forward = Vec2::from_angle(state.heading);
    let half_len = hull_length(state.kind) * 0.3;
    for (normal, cooldown) in [
        (forward.perp(), state.port_cooldown_secs),
        (-forward.perp(), state.starboard_cooldown_secs),
    ] {
        let ready = cooldown <= 0.0;
        let color = if ready {
            Color::srgba(1.0, 0.86, 0.45, 0.55)
        } else {
            Color::srgba(0.8, 0.85, 0.95, 0.15)
        };
        let near = center + normal * 10.0;
        let far = center + normal * state.weapon_range;
        for offset in [-half_len, half_len] {
            gizmos.line_2d(near + forward * offset, far + forward * offset, color);
        }
        gizmos.line_2d(far - forward * half_len, far + forward * half_len, color);
    }
}

/// Entidade visual de um projétil autoritativo (PRD §20: client interpola).
#[derive(Component)]
pub struct ProjectileVisual {
    pub target: ProjectileState,
}

#[allow(clippy::too_many_arguments)]
pub fn upsert_projectile_visuals(
    mut commands: Commands,
    mut snapshot_events: EventReader<ClientReceiveMessage<WorldSnapshot>>,
    mut existing: Query<(Entity, &mut ProjectileVisual)>,
    ships: Query<&ShipVisual>,
    assets: Res<GameAssets>,
    vfx: Option<Res<VfxHandles>>,
) {
    let Some(event) = snapshot_events.read().last() else {
        return;
    };
    let message = event.message();
    let seen: HashSet<u32> = message
        .projectiles
        .iter()
        .map(|p| p.projectile_id)
        .collect();

    // Projéteis que saíram do snapshot (impacto ou expiração) somem. Perto de
    // um casco = acerto (o fogo vem do dano); senão, borrifo na água.
    for (entity, visual) in existing.iter() {
        if seen.contains(&visual.target.projectile_id) {
            continue;
        }
        commands.entity(entity).despawn_recursive();
        let at = Vec2::new(visual.target.x, visual.target.y);
        let hit = ships
            .iter()
            .any(|ship| Vec2::new(ship.target.x, ship.target.y).distance(at) < 24.0);
        if !hit {
            for i in 0..7 {
                let dir = Vec2::from_angle(i as f32 * 0.9 + visual.target.heading);
                spawn_particle(&mut commands, at, Particle::splash(dir * 9.0));
            }
        }
    }

    for state in &message.projectiles {
        if let Some((_, mut visual)) = existing
            .iter_mut()
            .find(|(_, visual)| visual.target.projectile_id == state.projectile_id)
        {
            visual.target = *state;
            continue;
        }
        // Projétil novo: fumaça de boca de canhão onde ele nasceu.
        let at = Vec2::new(state.x, state.y);
        spawn_animation(&mut commands, &assets, parts::SMOKE, 4, at, 0.7);
        let mut entity = commands.spawn((
            ProjectileVisual { target: *state },
            Transform::from_xyz(state.x, state.y, layers::PROJECTILES),
            Visibility::default(),
        ));
        if let Some(vfx) = &vfx {
            entity.insert((
                Mesh2d(vfx.ball_mesh.clone()),
                MeshMaterial2d(vfx.ball_material.clone()),
            ));
        }
    }
}

pub fn lerp_projectile_visuals(
    time: Res<Time>,
    mut projectiles: Query<(&mut Transform, &ProjectileVisual)>,
) {
    // Projéteis voam rápido: lerp mais agressivo que navios.
    let factor = 1.0 - (-40.0 * time.delta_secs()).exp();
    for (mut transform, projectile) in &mut projectiles {
        apply_lerp(
            &mut transform,
            projectile.target.x,
            projectile.target.y,
            projectile.target.heading,
            factor,
            0.0,
        );
    }
}

fn apply_lerp(
    transform: &mut Transform,
    x: f32,
    y: f32,
    heading: f32,
    factor: f32,
    heading_offset: f32,
) {
    transform.translation = transform
        .translation
        .lerp(Vec3::new(x, y, transform.translation.z), factor);
    let target_rotation = Quat::from_rotation_z(heading + heading_offset);
    transform.rotation = transform.rotation.slerp(target_rotation, factor);
}

/// Destroço flutuando no mar (PRD §26): carga esperando um saqueador.
/// Aparece/desaparece pelo snapshot AOI (protocolo v8) com TTL visual.
#[derive(Component)]
pub struct WreckVisual {
    pub wreck_num: u32,
    pub last_seen: Instant,
}

fn deco_sprite(assets: &GameAssets, index: usize) -> Sprite {
    Sprite::from_atlas_image(
        assets.water_and_islands.clone(),
        TextureAtlas {
            layout: assets.deco.clone(),
            index,
        },
    )
}

/// Wrecks vindos do snapshot do destinatário (MF-031): upsert de visuais e
/// reconstrução do `KnownWrecks` (o saque do client usa as posições).
pub fn upsert_wreck_visuals(
    mut commands: Commands,
    mut snapshot_events: EventReader<ClientReceiveMessage<WorldSnapshot>>,
    mut known: ResMut<crate::net::KnownWrecks>,
    mut existing: Query<&mut WreckVisual>,
    assets: Res<GameAssets>,
) {
    let Some(event) = snapshot_events.read().last() else {
        return;
    };
    known.0.clear();
    for wreck in &event.message().wrecks {
        known.0.insert(wreck.wreck_id, Vec2::new(wreck.x, wreck.y));
        if let Some(mut visual) = existing
            .iter_mut()
            .find(|visual| visual.wreck_num == wreck.wreck_id)
        {
            visual.last_seen = Instant::now();
            continue;
        }
        // Destroço = tábuas à deriva em volta de um baú de carga.
        commands
            .spawn((
                WreckVisual {
                    wreck_num: wreck.wreck_id,
                    last_seen: Instant::now(),
                },
                Transform::from_xyz(wreck.x, wreck.y, layers::WRECKS).with_scale(Vec3::splat(0.9)),
                Visibility::default(),
            ))
            .with_children(|parent| {
                for (index, offset, angle) in [
                    (deco::PLANK, Vec2::new(-12.0, 6.0), 0.4),
                    (deco::PLANK_B, Vec2::new(11.0, -7.0), -0.7),
                    (deco::PLANK_DIAG, Vec2::new(-4.0, -12.0), 0.0),
                    (deco::PLANK, Vec2::new(8.0, 12.0), 2.2),
                ] {
                    parent.spawn((
                        deco_sprite(&assets, index),
                        Transform::from_translation(offset.extend(0.0))
                            .with_rotation(Quat::from_rotation_z(angle)),
                    ));
                }
                parent.spawn((
                    deco_sprite(&assets, deco::CHEST_GOLD),
                    Transform::from_xyz(0.0, 0.0, 0.1),
                ));
            });
        info!(wreck_id = wreck.wreck_id, "destroço visível no mar");
    }
}

/// Marca de cabeça a prêmio sobre um navio Procurado (visível a todos):
/// losango vermelho e "PROCURADO". Entidade própria, fora da hierarquia do
/// navio, para não girar com o casco.
#[derive(Component)]
pub struct WantedMarker {
    ship_id: u32,
}

/// Folga entre a proa (meio casco) e o losango de procurado.
const WANTED_MARKER_GAP: f32 = 10.0;

fn is_wanted(state: &ShipState) -> bool {
    state.notoriety_tier >= TIER_PROCURADO
}

pub fn update_wanted_markers(
    mut commands: Commands,
    ships: Query<(&ShipVisual, &Transform), Without<WantedMarker>>,
    mut markers: Query<(Entity, &WantedMarker, &mut Transform), Without<ShipVisual>>,
) {
    // Acima do casco inteiro: um Patrol (64 m) engoliria um offset fixo.
    let above = |visual: &ShipVisual, ship: &Transform| {
        Vec3::new(
            ship.translation.x,
            ship.translation.y + hull_length(visual.target.kind) * 0.5 + WANTED_MARKER_GAP,
            layers::LABELS,
        )
    };
    let mut marked = HashSet::new();
    for (entity, marker, mut transform) in &mut markers {
        match ships
            .iter()
            .find(|(visual, _)| visual.target.ship_id == marker.ship_id)
        {
            Some((visual, ship)) if is_wanted(&visual.target) => {
                transform.translation = above(visual, ship);
                marked.insert(marker.ship_id);
            }
            _ => commands.entity(entity).despawn_recursive(),
        }
    }
    for (visual, ship) in &ships {
        if !is_wanted(&visual.target) || marked.contains(&visual.target.ship_id) {
            continue;
        }
        commands
            .spawn((
                WantedMarker {
                    ship_id: visual.target.ship_id,
                },
                Transform::from_translation(above(visual, ship)),
                Visibility::default(),
            ))
            .with_children(|marker| {
                marker.spawn((
                    Sprite::from_color(Color::srgb(0.9, 0.12, 0.1), Vec2::splat(9.0)),
                    Transform::from_rotation(Quat::from_rotation_z(std::f32::consts::FRAC_PI_4)),
                ));
                marker.spawn((
                    Text2d::new(crate::i18n::tr("PROCURADO")),
                    TextFont {
                        font_size: 13.0,
                        ..default()
                    },
                    TextColor(Color::srgb(1.0, 0.25, 0.2)),
                    Transform::from_xyz(0.0, 13.0, 0.1),
                ));
            });
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn each_ship_kind_looks_distinct() {
        let kinds = [ShipKind::SmallMerchant, ShipKind::Patrol, ShipKind::Corsair];
        let hulls: HashSet<_> = kinds
            .iter()
            .map(|kind| format!("{:?}", ship_look(*kind, Faction::Player).hull))
            .collect();
        assert_eq!(hulls.len(), 3);
    }

    #[test]
    fn factions_fly_their_own_colors() {
        for kind in [ShipKind::SmallMerchant, ShipKind::Patrol, ShipKind::Corsair] {
            let pirate = ship_look(kind, Faction::Pirate);
            assert_eq!((pirate.hull_color, pirate.sail_color), (0, 5));
            let navy = ship_look(kind, Faction::Navy);
            assert_eq!((navy.hull_color, navy.sail_color), (3, 0));
            let merchant = ship_look(kind, Faction::Merchant);
            assert_eq!((merchant.hull_color, merchant.sail_color), (1, 1));
            assert_ne!(ship_look(kind, Faction::Player).sail_color, 5);
        }
        assert_eq!(flag_color(Faction::Pirate, false), 4);
        assert_eq!(flag_color(Faction::Navy, false), 3);
        assert_eq!(flag_color(Faction::Merchant, false), 5);
        assert_eq!(flag_color(Faction::Player, true), 2);
    }

    #[test]
    fn mastless_kraken_has_no_main_mast() {
        for kind in [ShipKind::SmallMerchant, ShipKind::Patrol, ShipKind::Corsair] {
            assert_eq!(main_mast(&ship_look(kind, Faction::Monster)), None);
            assert!(main_mast(&ship_look(kind, Faction::Player)).is_some());
        }
    }

    #[test]
    fn wanted_ship_gets_one_marker_that_leaves_when_pardoned() {
        use bevy::ecs::schedule::Schedule;
        let mut world = World::new();
        let mut state = ShipState {
            ship_id: 7,
            kind: ShipKind::Corsair,
            x: 0.0,
            y: 0.0,
            heading: 0.0,
            speed: 0.0,
            cargo_weight: 0,
            hp: 70,
            max_hp: 70,
            max_speed: 40.0,
            weapon_damage: 25,
            weapon_range: 55.0,
            port_cooldown_secs: 0.0,
            starboard_cooldown_secs: 0.0,
            is_npc: false,
            cargo_capacity: 40,
            sail_hp: 100.0,
            ammo: Default::default(),
            faction: Faction::Player,
            notoriety_tier: TIER_PROCURADO,
            rudder_hp: 100.0,
            crew: 0,
            crew_max: 0,
            repairing: false,
            dig_progress: 0.0,
            sail_cosmetic: 0,
            flag_cosmetic: 0,
        };
        let ship = world
            .spawn((
                ShipVisual {
                    target: state,
                    last_seen: Instant::now(),
                },
                Transform::from_xyz(100.0, 50.0, 0.0),
            ))
            .id();
        let mut schedule = Schedule::default();
        schedule.add_systems(update_wanted_markers);
        schedule.run(&mut world);
        schedule.run(&mut world);
        let mut markers = world.query::<(&WantedMarker, &Transform)>();
        let found: Vec<_> = markers.iter(&world).collect();
        assert_eq!(found.len(), 1);
        assert_eq!(
            found[0].1.translation.y,
            50.0 + hull_length(state.kind) * 0.5 + WANTED_MARKER_GAP
        );

        state.notoriety_tier = 0;
        world.get_mut::<ShipVisual>(ship).unwrap().target = state;
        schedule.run(&mut world);
        assert_eq!(markers.iter(&world).count(), 0);
    }

    #[test]
    fn bow_of_the_atlas_hull_points_along_heading() {
        // Proa do sprite = -Y local. Com heading 0 ela precisa apontar +X.
        let bow = Quat::from_rotation_z(SHIP_HEADING_OFFSET) * Vec3::NEG_Y;
        assert!((bow - Vec3::X).length() < 1e-5);
    }
}
