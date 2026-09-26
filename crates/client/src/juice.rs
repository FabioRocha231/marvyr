//! Sensação de jogo (MF-059): eventos de mar derivados do snapshot (salva,
//! acerto, borrifo, naufrágio), tremor de câmera, flash de casco atingido,
//! números de dano e vinheta de HP baixo. Nada aqui é verdade de jogo — é
//! polimento que o client deduz comparando snapshots, como `ship.rs`/`vfx.rs`.

use std::collections::HashMap;

use bevy::prelude::*;
use bevy::ui::FocusPolicy;
use lightyear::prelude::ClientReceiveMessage;
use marvyr_protocol::WorldSnapshot;

use crate::assets::layers;
use crate::camera::{follow_camera, shake_camera, unshake_camera, CameraShake};
use crate::net::{MyDocked, MyShip};
use crate::ship::{ShipVisual, Sinking};

/// Balas novas (ou que caíram) a menos disto uma da outra no mesmo snapshot
/// são uma única salva (o servidor solta 3 balas espaçadas de 9 m).
pub const SALVO_RADIUS: f32 = 40.0;
/// Salva que nasce a menos disto do próprio navio é nossa.
const OWN_FIRE_RADIUS: f32 = 60.0;
/// Bala que some a menos disto de um casco acertou (mesma regra de ship.rs).
const HIT_RADIUS: f32 = 24.0;

/// Algo audível/visível aconteceu no mar.
#[derive(Event, Debug, Clone, Copy, PartialEq)]
pub enum SeaEvent {
    Salvo {
        at: Vec2,
        own: bool,
    },
    HullHit {
        at: Vec2,
        ship_id: u32,
        damage: u32,
        own: bool,
    },
    Splash {
        at: Vec2,
    },
    Sunk {
        at: Vec2,
        own: bool,
    },
}

/// O que o snapshot anterior dizia (para descobrir o que mudou).
#[derive(Resource, Default)]
struct SnapshotMemory {
    hp: HashMap<u32, u32>,
    projectiles: HashMap<u32, Vec2>,
}

/// Agrupa pontos próximos: devolve um representante por grupo. É a regra
/// "uma salva = um estrondo" (e um borrifo por grupo de balas na água).
pub fn cluster_points(points: &[Vec2], radius: f32) -> Vec<Vec2> {
    let mut groups: Vec<Vec2> = Vec::new();
    for point in points {
        if !groups.iter().any(|g| g.distance(*point) <= radius) {
            groups.push(*point);
        }
    }
    groups
}

fn detect_sea_events(
    mut snapshots: EventReader<ClientReceiveMessage<WorldSnapshot>>,
    my_ship: Res<MyShip>,
    mut memory: ResMut<SnapshotMemory>,
    mut out: EventWriter<SeaEvent>,
) {
    for event in snapshots.read() {
        let snap = event.message();
        let own_pos = snap
            .ships
            .iter()
            .find(|s| Some(s.ship_id) == my_ship.0)
            .map(|s| Vec2::new(s.x, s.y));

        let mut hp = HashMap::with_capacity(snap.ships.len());
        for s in &snap.ships {
            if let Some(&before) = memory.hp.get(&s.ship_id) {
                if s.hp < before {
                    out.send(SeaEvent::HullHit {
                        at: Vec2::new(s.x, s.y),
                        ship_id: s.ship_id,
                        damage: before - s.hp,
                        own: Some(s.ship_id) == my_ship.0,
                    });
                }
            }
            hp.insert(s.ship_id, s.hp);
        }
        memory.hp = hp;

        let current: HashMap<u32, Vec2> = snap
            .projectiles
            .iter()
            .map(|p| (p.projectile_id, Vec2::new(p.x, p.y)))
            .collect();
        let fired: Vec<Vec2> = current
            .iter()
            .filter(|(id, _)| !memory.projectiles.contains_key(id))
            .map(|(_, at)| *at)
            .collect();
        for at in cluster_points(&fired, SALVO_RADIUS) {
            let own = own_pos.is_some_and(|o| o.distance(at) < OWN_FIRE_RADIUS);
            out.send(SeaEvent::Salvo { at, own });
        }
        let landed: Vec<Vec2> = memory
            .projectiles
            .iter()
            .filter(|(id, _)| !current.contains_key(id))
            .map(|(_, at)| *at)
            .filter(|at| {
                !snap
                    .ships
                    .iter()
                    .any(|s| Vec2::new(s.x, s.y).distance(*at) < HIT_RADIUS)
            })
            .collect();
        for at in cluster_points(&landed, SALVO_RADIUS) {
            out.send(SeaEvent::Splash { at });
        }
        memory.projectiles = current;
    }
}

fn detect_sinking(
    my_ship: Res<MyShip>,
    sinking: Query<(&Sinking, &Transform), Added<Sinking>>,
    mut out: EventWriter<SeaEvent>,
) {
    for (sinking, transform) in &sinking {
        out.send(SeaEvent::Sunk {
            at: transform.translation.truncate(),
            own: Some(sinking.ship_id) == my_ship.0,
        });
    }
}

/// 1 perto da câmera, 0 a partir de `range` metros.
fn nearness(camera: Vec2, at: Vec2, range: f32) -> f32 {
    (1.0 - camera.distance(at) / range).clamp(0.0, 1.0)
}

fn add_trauma(
    mut events: EventReader<SeaEvent>,
    mut shake: ResMut<CameraShake>,
    camera: Query<&Transform, With<Camera2d>>,
) {
    let camera = camera
        .get_single()
        .map(|t| t.translation.truncate())
        .unwrap_or_default();
    for event in events.read() {
        let amount = match *event {
            SeaEvent::Salvo { own: true, .. } => 0.18,
            SeaEvent::HullHit { own: true, .. } => 0.55,
            SeaEvent::HullHit { at, .. } => 0.25 * nearness(camera, at, 300.0),
            SeaEvent::Sunk { own: true, .. } => 0.8,
            SeaEvent::Sunk { at, .. } => 0.6 * nearness(camera, at, 500.0),
            _ => 0.0,
        };
        shake.add(amount);
    }
}

/// Casco atingido estoura para branco e volta ao normal.
#[derive(Component)]
struct HitFlash(f32);

const FLASH_SECS: f32 = 0.15;
/// Cor do sprite multiplica a textura; > 1 em linear satura para branco.
const FLASH_GAIN: f32 = 4.0;

fn start_hit_flash(
    mut commands: Commands,
    mut events: EventReader<SeaEvent>,
    ships: Query<(Entity, &ShipVisual), Without<Sinking>>,
) {
    for event in events.read() {
        let SeaEvent::HullHit { ship_id, .. } = *event else {
            continue;
        };
        if let Some((entity, _)) = ships.iter().find(|(_, v)| v.target.ship_id == ship_id) {
            commands.entity(entity).try_insert(HitFlash(0.0));
        }
    }
}

fn animate_hit_flash(
    mut commands: Commands,
    time: Res<Time>,
    mut ships: Query<(Entity, &mut HitFlash, &Children), Without<Sinking>>,
    mut sprites: Query<&mut Sprite>,
) {
    for (entity, mut flash, children) in &mut ships {
        flash.0 += time.delta_secs();
        let k = (flash.0 / FLASH_SECS).min(1.0);
        let gain = FLASH_GAIN + (1.0 - FLASH_GAIN) * k;
        for child in children.iter() {
            if let Ok(mut sprite) = sprites.get_mut(*child) {
                let alpha = sprite.color.alpha();
                // Sombra (alpha baixo) fica como está.
                if alpha >= 0.5 {
                    sprite.color = Color::linear_rgba(gain, gain, gain, alpha);
                }
            }
        }
        if k >= 1.0 {
            commands.entity(entity).remove::<HitFlash>();
        }
    }
}

/// "-8" flutuando acima do casco atingido. `punch` = tamanho do estouro
/// (golpe grande cresce mais antes de assentar).
#[derive(Component)]
struct DamageNumber {
    age: f32,
    color: Color,
    punch: f32,
}

/// Golpe a partir daqui é "pesado": número maior e com exclamação.
const HEAVY_HIT: u32 = 25;
/// Dourado dos acertos do meu canhão.
const MY_HIT: Color = Color::srgb(1.0, 0.8, 0.2);

const DAMAGE_NUMBER_SECS: f32 = 0.8;

/// Texto que sobe e some sobre o mundo (dano, Renome ganho).
pub fn spawn_float_text(commands: &mut Commands, at: Vec2, text: String, color: Color) {
    commands.spawn((
        Text2d::new(text),
        TextFont {
            font_size: 22.0,
            ..default()
        },
        TextColor(color),
        Transform::from_translation((at + Vec2::new(0.0, 24.0)).extend(layers::LABELS)),
        DamageNumber {
            age: 0.0,
            color,
            punch: 0.6,
        },
    ));
}

/// Número de dano com estouro: tamanho pelo golpe, cor por quem apanhou
/// (vermelho no meu casco, dourado no alvo do meu canhão).
fn spawn_hit_number(commands: &mut Commands, at: Vec2, damage: u32, color: Color) {
    let heavy = damage >= HEAVY_HIT;
    let text = if heavy {
        format!("-{damage}!")
    } else {
        format!("-{damage}")
    };
    commands.spawn((
        Text2d::new(text),
        TextFont {
            font_size: if heavy { 30.0 } else { 22.0 },
            ..default()
        },
        TextColor(color),
        Transform::from_translation((at + Vec2::new(0.0, 24.0)).extend(layers::LABELS)),
        DamageNumber {
            age: 0.0,
            color,
            punch: if heavy { 1.2 } else { 0.7 },
        },
    ));
}

fn spawn_damage_numbers(
    mut commands: Commands,
    mut events: EventReader<SeaEvent>,
    my_ship: Res<MyShip>,
    visuals: Query<&crate::ship::ShipVisual>,
) {
    // Alvo do meu tiro automático: o acerto nele é meu (dourado).
    let my_target = my_ship.0.and_then(|me| {
        visuals
            .iter()
            .find(|v| v.target.ship_id == me)
            .and_then(|v| v.target.fire_target)
    });
    for event in events.read() {
        match *event {
            SeaEvent::HullHit {
                at,
                damage,
                own,
                ship_id,
            } => {
                let color = if own {
                    Color::srgb(1.0, 0.3, 0.25)
                } else if my_target == Some(ship_id) {
                    MY_HIT
                } else {
                    Color::WHITE
                };
                spawn_hit_number(&mut commands, at, damage, color);
            }
            // Naufrágio: letreiro grande no lugar do casco.
            SeaEvent::Sunk { at, own } => {
                let (text, color) = if own {
                    (crate::i18n::tr("NAUFRAGOU!"), Color::srgb(1.0, 0.3, 0.25))
                } else {
                    (crate::i18n::tr("AFUNDOU!"), MY_HIT)
                };
                commands.spawn((
                    Text2d::new(text),
                    TextFont {
                        font_size: 36.0,
                        ..default()
                    },
                    TextColor(color),
                    Transform::from_translation((at + Vec2::new(0.0, 40.0)).extend(layers::LABELS)),
                    DamageNumber {
                        age: -0.6,
                        color,
                        punch: 1.4,
                    },
                ));
            }
            _ => {}
        }
    }
}

fn animate_damage_numbers(
    mut commands: Commands,
    time: Res<Time>,
    camera: Query<&OrthographicProjection, With<Camera2d>>,
    mut numbers: Query<(Entity, &mut DamageNumber, &mut Transform, &mut TextColor)>,
) {
    let dt = time.delta_secs();
    // Tamanho constante na tela, qualquer que seja o zoom.
    let scale = camera.get_single().map(|o| o.scale).unwrap_or(1.0);
    for (entity, mut number, mut transform, mut color) in &mut numbers {
        number.age += dt;
        if number.age >= DAMAGE_NUMBER_SECS {
            commands.entity(entity).despawn();
            continue;
        }
        // Idade negativa = segura mais tempo na tela (letreiro de naufrágio).
        let k = number.age.max(0.0) / DAMAGE_NUMBER_SECS;
        transform.translation.y += 22.0 * dt;
        // Estouro: nasce grande e assenta em ~0,15 s.
        let pop = 1.0 + number.punch * (1.0 - (number.age.max(0.0) / 0.15).min(1.0)).powi(2);
        transform.scale = Vec3::splat(scale * pop);
        color.0 = number.color.with_alpha(1.0 - k * k);
    }
}

/// Faixa vermelha na borda da tela; `f32` = alpha base da faixa.
#[derive(Component)]
struct VignetteBand(f32);

#[derive(Component)]
struct LowHpVignette;

const LOW_HP_RATIO: f32 = 0.3;

fn setup_vignette(mut commands: Commands) {
    const BAND_PX: f32 = 14.0;
    const BANDS: usize = 4;
    commands
        .spawn((
            Node {
                position_type: PositionType::Absolute,
                width: Val::Percent(100.0),
                height: Val::Percent(100.0),
                ..default()
            },
            Visibility::Hidden,
            GlobalZIndex(50),
            FocusPolicy::Pass,
            PickingBehavior::IGNORE,
            LowHpVignette,
        ))
        .with_children(|root| {
            for band in 0..BANDS {
                let inset = Val::Px(band as f32 * BAND_PX);
                let thick = Val::Px(BAND_PX);
                let alpha = 0.32 * (1.0 - band as f32 / BANDS as f32);
                let zero = Val::Px(0.0);
                let horizontal = |top, bottom| Node {
                    left: zero,
                    right: zero,
                    top,
                    bottom,
                    height: thick,
                    ..default()
                };
                let vertical = |left, right| Node {
                    top: zero,
                    bottom: zero,
                    left,
                    right,
                    width: thick,
                    ..default()
                };
                for edge in [
                    horizontal(inset, Val::Auto),
                    horizontal(Val::Auto, inset),
                    vertical(inset, Val::Auto),
                    vertical(Val::Auto, inset),
                ] {
                    root.spawn((
                        Node {
                            position_type: PositionType::Absolute,
                            ..edge
                        },
                        BackgroundColor(Color::srgba(0.8, 0.05, 0.05, alpha)),
                        FocusPolicy::Pass,
                        PickingBehavior::IGNORE,
                        VignetteBand(alpha),
                    ));
                }
            }
        });
}

fn pulse_vignette(
    time: Res<Time>,
    my_ship: Res<MyShip>,
    docked: Res<MyDocked>,
    ships: Query<&ShipVisual>,
    mut root: Query<&mut Visibility, With<LowHpVignette>>,
    mut bands: Query<(&VignetteBand, &mut BackgroundColor)>,
) {
    let low = !docked.0
        && ships
            .iter()
            .find(|v| Some(v.target.ship_id) == my_ship.0)
            .is_some_and(|v| {
                let s = &v.target;
                s.max_hp > 0 && s.hp > 0 && (s.hp as f32) < s.max_hp as f32 * LOW_HP_RATIO
            });
    let Ok(mut visibility) = root.get_single_mut() else {
        return;
    };
    let wanted = if low {
        Visibility::Inherited
    } else {
        Visibility::Hidden
    };
    visibility.set_if_neq(wanted);
    if !low {
        return;
    }
    let pulse = 0.55 + 0.45 * (time.elapsed_secs() * 4.0).sin();
    for (band, mut bg) in &mut bands {
        bg.0 = bg.0.with_alpha(band.0 * pulse);
    }
}

pub struct JuicePlugin;

impl Plugin for JuicePlugin {
    fn build(&self, app: &mut App) {
        app.add_event::<SeaEvent>()
            .init_resource::<SnapshotMemory>()
            .init_resource::<CameraShake>()
            .add_systems(Startup, setup_vignette)
            .add_systems(
                Update,
                (
                    (detect_sea_events, detect_sinking),
                    (
                        add_trauma,
                        start_hit_flash,
                        spawn_damage_numbers,
                        animate_hit_flash,
                        animate_damage_numbers,
                        pulse_vignette,
                    ),
                )
                    .chain(),
            )
            .add_systems(
                Update,
                (
                    unshake_camera.before(follow_camera),
                    shake_camera.after(follow_camera),
                ),
            );
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn one_salvo_of_three_balls_is_one_event() {
        // 3 balas espaçadas de 9 m ao longo do casco = um estrondo.
        let salvo = [
            Vec2::new(0.0, 0.0),
            Vec2::new(9.0, 1.0),
            Vec2::new(18.0, 2.0),
        ];
        assert_eq!(cluster_points(&salvo, SALVO_RADIUS).len(), 1);
    }

    #[test]
    fn two_ships_firing_apart_are_two_salvos() {
        let points = [
            Vec2::new(0.0, 0.0),
            Vec2::new(9.0, 0.0),
            Vec2::new(300.0, 0.0),
            Vec2::new(309.0, 0.0),
        ];
        assert_eq!(cluster_points(&points, SALVO_RADIUS).len(), 2);
        assert!(cluster_points(&[], SALVO_RADIUS).is_empty());
    }

    fn ship(ship_id: u32, x: f32, hp: u32) -> marvyr_protocol::ShipState {
        marvyr_protocol::ShipState {
            ship_id,
            kind: marvyr_domain_ships::ShipKind::Corsair,
            x,
            y: 0.0,
            heading: 0.0,
            speed: 0.0,
            cargo_weight: 0,
            hp,
            max_hp: 100,
            max_speed: 30.0,
            weapon_damage: 10,
            weapon_range: 50.0,
            port_cooldown_secs: 0.0,
            starboard_cooldown_secs: 0.0,
            is_npc: false,
            cargo_capacity: 0,
            sail_hp: 100.0,
            ammo: marvyr_domain_combat::Ammo::Round,
            faction: marvyr_protocol::Faction::Player,
            notoriety_tier: 0,
            rudder_hp: 100.0,
            crew: 0,
            crew_max: 0,
            repairing: false,
            dig_progress: 0.0,
            sail_cosmetic: 0,
            flag_cosmetic: 0,
            black_flag: 0,
            fire_target: None,
            aura: 0,
            flasks: Default::default(),
            elite: 0,
        }
    }

    fn ball(projectile_id: u32, x: f32) -> marvyr_protocol::ProjectileState {
        marvyr_protocol::ProjectileState {
            projectile_id,
            x,
            y: 30.0,
            heading: 0.0,
        }
    }

    fn step(
        app: &mut App,
        ships: Vec<marvyr_protocol::ShipState>,
        projectiles: Vec<marvyr_protocol::ProjectileState>,
    ) -> Vec<SeaEvent> {
        let snapshot = WorldSnapshot {
            tick: 0,
            ships,
            projectiles,
            wrecks: vec![],
        };
        app.world_mut()
            .resource_mut::<Events<ClientReceiveMessage<WorldSnapshot>>>()
            .send(ClientReceiveMessage::new(
                snapshot,
                lightyear::prelude::ClientId::Local(0),
            ));
        app.update();
        app.world_mut()
            .resource_mut::<Events<SeaEvent>>()
            .drain()
            .collect()
    }

    #[test]
    fn snapshots_become_one_salvo_a_hit_and_one_splash() {
        let mut app = App::new();
        app.add_event::<ClientReceiveMessage<WorldSnapshot>>()
            .add_event::<SeaEvent>()
            .insert_resource(MyShip(Some(1)))
            .init_resource::<SnapshotMemory>()
            .add_systems(Update, detect_sea_events);
        let fleet = |hp_two| vec![ship(1, 0.0, 100), ship(2, 200.0, hp_two)];

        assert!(step(&mut app, fleet(100), vec![]).is_empty());
        // Nossa salva: 3 balas ao lado do casco = um estrondo, nosso.
        let events = step(
            &mut app,
            fleet(100),
            vec![ball(1, 0.0), ball(2, 9.0), ball(3, 18.0)],
        );
        assert_eq!(events.len(), 1);
        assert!(matches!(events[0], SeaEvent::Salvo { own: true, .. }));
        // O navio 2 perde 8 de HP; as 3 balas somem longe de cascos = 1 borrifo.
        let events = step(&mut app, fleet(92), vec![]);
        assert!(events.contains(&SeaEvent::HullHit {
            at: Vec2::new(200.0, 0.0),
            ship_id: 2,
            damage: 8,
            own: false
        }));
        let splashes = events
            .iter()
            .filter(|e| matches!(e, SeaEvent::Splash { .. }))
            .count();
        assert_eq!(splashes, 1);
    }

    #[test]
    fn nearness_is_linear_and_clamped() {
        assert_eq!(nearness(Vec2::ZERO, Vec2::ZERO, 300.0), 1.0);
        assert!((nearness(Vec2::ZERO, Vec2::new(150.0, 0.0), 300.0) - 0.5).abs() < 1e-6);
        assert_eq!(nearness(Vec2::ZERO, Vec2::new(900.0, 0.0), 300.0), 0.0);
    }
}
