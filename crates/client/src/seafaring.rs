//! Mar profundo no client (MV-061): teclas das ações novas, marcas no mar
//! (evento em curso, X do tesouro, tentáculos do Kraken), ilhas ocultas que
//! aparecem quando avistadas e a linha de estado de bordo no HUD.
//!
//! Tudo aqui é apresentação: o servidor decide se o reparo, a abordagem ou
//! a escavação acontecem (`ActionResult` cai no feed).

use std::collections::HashMap;

use bevy::prelude::*;
use lightyear::prelude::client::*;
use lightyear::prelude::ClientReceiveMessage;
use marvyr_protocol::{
    BoardShip, DigTreasure, Faction, HireCrew, IslandState, IslandsInSight, SeaEventKind,
    SeaEventState, SeaEventsUpdate, SetRepair, ShipState, TreasureHint, TreasureHints,
};

use crate::assets::layers;
use crate::hud::SeaHud;
use crate::net::{MyDocked, MyShip, ReliableChannel};
use crate::ship::ShipVisual;
use crate::ui;

/// Alcance da abordagem no client (45 m do servidor, com folga do lerp).
const BOARD_RANGE: f32 = 44.0;

#[derive(Resource, Debug, Default)]
pub struct SeaEvents(pub Vec<SeaEventState>);

#[derive(Resource, Debug, Default)]
pub struct TreasureMarks(pub Vec<TreasureHint>);

/// Ilhas ocultas já avistadas nesta sessão (ficam no mapa depois).
#[derive(Resource, Debug, Default)]
pub struct SeenIslands(pub HashMap<u32, IslandState>);

/// Linha de bordo (tripulação, leme, reparo, escavação) — vive no bilhete
/// do navio, montado pelo `hud`.
#[derive(Component)]
pub struct SeaStatusText;
#[derive(Component)]
struct EventHeadline;
#[derive(Component)]
struct EventStrip;
#[derive(Component)]
struct EventBannerText;

pub struct SeafaringPlugin;

impl Plugin for SeafaringPlugin {
    fn build(&self, app: &mut App) {
        app.init_resource::<SeaEvents>()
            .init_resource::<TreasureMarks>()
            .init_resource::<SeenIslands>()
            .add_systems(Startup, setup_sea_hud)
            .add_systems(
                Update,
                (
                    receive_sea_state,
                    send_sea_input,
                    draw_sea_marks,
                    update_sea_hud,
                ),
            );
    }
}

/// Evento de mar como manchete de cartaz: uma tira de papel abaixo do
/// carimbo de região, o nome do evento em tipo de madeira com a segunda cor
/// fora de registro e, embaixo, relógio, distância e rumo. É o único
/// momento tipográfico monumental da tela; some quando o mar está calmo.
fn setup_sea_hud(mut commands: Commands) {
    commands
        .spawn((
            Node {
                position_type: PositionType::Absolute,
                top: Val::Px(96.0), // abaixo do carimbo de região
                width: Val::Percent(100.0),
                justify_content: JustifyContent::Center,
                ..default()
            },
            SeaHud,
        ))
        .with_children(|row| {
            row.spawn((
                ui::panel(Node {
                    flex_direction: FlexDirection::Column,
                    align_items: AlignItems::Center,
                    padding: UiRect::axes(Val::Px(22.0), Val::Px(8.0)),
                    row_gap: Val::Px(2.0),
                    display: Display::None,
                    ..default()
                }),
                EventStrip,
            ))
            .with_children(|strip| {
                strip.spawn(Node::default()).with_children(|stack| {
                    stack.spawn((
                        ui::display("", 30.0, ui::VERMILION),
                        Node {
                            position_type: PositionType::Absolute,
                            left: Val::Px(2.5),
                            top: Val::Px(2.5),
                            ..default()
                        },
                        EventHeadline,
                    ));
                    stack.spawn((ui::display("", 30.0, ui::INK), EventHeadline));
                });
                strip.spawn((
                    ui::face("", ui::FONT_BOLD, 14.0, ui::INK_SOFT),
                    TextLayout::new_with_justify(JustifyText::Center),
                    EventBannerText,
                ));
            });
        });
}

#[allow(clippy::too_many_arguments)]
fn receive_sea_state(
    mut commands: Commands,
    assets: Res<crate::assets::GameAssets>,
    mut event_updates: EventReader<ClientReceiveMessage<SeaEventsUpdate>>,
    mut hint_updates: EventReader<ClientReceiveMessage<TreasureHints>>,
    mut island_updates: EventReader<ClientReceiveMessage<IslandsInSight>>,
    mut events: ResMut<SeaEvents>,
    mut marks: ResMut<TreasureMarks>,
    mut seen: ResMut<SeenIslands>,
) {
    if let Some(update) = event_updates.read().last() {
        events.0 = update.message().events.clone();
    }
    if let Some(update) = hint_updates.read().last() {
        marks.0 = update.message().hints.clone();
    }
    for update in island_updates.read() {
        for island in &update.message().islands {
            if seen.0.contains_key(&island.island_id) {
                continue;
            }
            info!(island = %island.name, "ilha oculta avistada");
            spawn_island(&mut commands, &assets, island);
            seen.0.insert(island.island_id, island.clone());
        }
    }
}

fn spawn_island(commands: &mut Commands, assets: &crate::assets::GameAssets, island: &IslandState) {
    // A terra em si é pintada pelo shader do mar (`SeenIslands` entra na
    // lista de terra do `stream_land`); aqui só as palmeiras e o nome.
    crate::world::spawn_vegetation(commands, assets, &[island_land(island)], &[]);
    commands.spawn((
        Text2d::new(island.name.clone()),
        TextFont {
            font_size: 14.0,
            ..default()
        },
        TextColor(ui::TEXT),
        Transform::from_translation(Vec3::new(
            island.x,
            island.y + island.radius + 14.0,
            layers::LABELS,
        )),
    ));
}

/// Ilha oculta avistada como terra comum (praia e mata no shader).
pub fn island_land(island: &IslandState) -> marvyr_domain_world::LandMass {
    marvyr_domain_world::LandMass::new(island.x, island.y, island.radius)
}

fn my_state<'a>(my_ship: &MyShip, visuals: &'a Query<&ShipVisual>) -> Option<&'a ShipState> {
    let id = my_ship.0?;
    visuals
        .iter()
        .find(|visual| visual.target.ship_id == id)
        .map(|visual| &visual.target)
}

/// Navio mais próximo ao alcance da abordagem (o servidor revalida).
pub fn board_target(mine: &ShipState, others: &[ShipState]) -> Option<u32> {
    let at = Vec2::new(mine.x, mine.y);
    others
        .iter()
        .filter(|other| other.ship_id != mine.ship_id && other.faction != Faction::Monster)
        .map(|other| (other.ship_id, at.distance(Vec2::new(other.x, other.y))))
        .filter(|(_, distance)| *distance <= BOARD_RANGE)
        .min_by(|a, b| a.1.total_cmp(&b.1))
        .map(|(id, _)| id)
}

/// K reparo · H abordagem · J cavar · P contratar marujos (no porto).
fn send_sea_input(
    keys: Res<ButtonInput<KeyCode>>,
    my_ship: Res<MyShip>,
    docked: Res<MyDocked>,
    visuals: Query<&ShipVisual>,
    mut connection_manager: ResMut<ConnectionManager>,
) {
    let Some(mine) = my_state(&my_ship, &visuals) else {
        return;
    };
    if keys.just_pressed(KeyCode::KeyK) {
        let _ = connection_manager.send_message::<ReliableChannel, _>(&SetRepair {
            active: !mine.repairing,
        });
    }
    if keys.just_pressed(KeyCode::KeyH) {
        let others: Vec<ShipState> = visuals.iter().map(|visual| visual.target).collect();
        if let Some(target_ship_id) = board_target(mine, &others) {
            let _ = connection_manager
                .send_message::<ReliableChannel, _>(&BoardShip { target_ship_id });
        } else {
            // Sem alvo por perto: o servidor responde com o motivo.
            let _ = connection_manager.send_message::<ReliableChannel, _>(&BoardShip {
                target_ship_id: u32::MAX,
            });
        }
    }
    if keys.just_pressed(KeyCode::KeyJ) {
        let _ = connection_manager.send_message::<ReliableChannel, _>(&DigTreasure);
    }
    if keys.just_pressed(KeyCode::KeyP) && docked.0 {
        let count = mine.crew_max.saturating_sub(mine.crew).max(1);
        let _ = connection_manager.send_message::<ReliableChannel, _>(&HireCrew { count });
    }
}

fn event_color(kind: SeaEventKind) -> Color {
    match kind {
        SeaEventKind::Tempest => Color::srgb(0.55, 0.60, 0.95),
        SeaEventKind::TreasureFleet => ui::GOLD,
        SeaEventKind::Kraken => Color::srgb(0.75, 0.30, 0.85),
        SeaEventKind::ContestedTide => Color::srgb(0.45, 0.90, 0.90),
    }
}

/// "[Raro] Guardado · Covil do Kraken · +120% no baú".
pub fn map_mods_line(
    rarity: marvyr_domain_items::Rarity,
    mods: &[marvyr_domain_items::MapMod],
) -> String {
    let names: Vec<String> = mods.iter().map(|m| crate::i18n::tr(m.label())).collect();
    format!(
        "[{}] {} · {}",
        crate::i18n::tr(crate::affixes::rarity_label(rarity)),
        names.join(" · "),
        crate::i18n::trf(
            "+{0}% no baú",
            &[&marvyr_domain_items::map_mod::treasure_bonus_pct(mods).to_string()]
        )
    )
}

fn draw_sea_marks(
    mut gizmos: Gizmos,
    time: Res<Time>,
    events: Res<SeaEvents>,
    marks: Res<TreasureMarks>,
    visuals: Query<&ShipVisual>,
) {
    let t = time.elapsed_secs();
    for event in &events.0 {
        let pulse = 0.35 + 0.15 * (t * 2.0).sin();
        gizmos.circle_2d(
            Isometry2d::from_translation(Vec2::new(event.x, event.y)),
            event.radius,
            event_color(event.kind).with_alpha(pulse),
        );
    }
    for mark in &marks.0 {
        let at = Vec2::new(mark.x, mark.y);
        // v26: mapa Mágico/Raro marca na cor da raridade, maior e pulsando —
        // perigo à vista antes de descer o escaler.
        let (color, arm) = match mark.rarity {
            marvyr_domain_items::Rarity::Normal => (ui::DANGER, 14.0),
            rarity => (
                crate::affixes::rarity_color(rarity),
                if rarity == marvyr_domain_items::Rarity::Rare {
                    22.0
                } else {
                    18.0
                },
            ),
        };
        gizmos.line_2d(at - Vec2::splat(arm), at + Vec2::splat(arm), color);
        gizmos.line_2d(at + Vec2::new(-arm, arm), at + Vec2::new(arm, -arm), color);
        gizmos.circle_2d(
            Isometry2d::from_translation(at),
            40.0,
            color.with_alpha(0.4),
        );
        if !mark.mods.is_empty() {
            let pulse = 0.35 + 0.25 * (t * 3.0).sin().abs();
            gizmos.circle_2d(
                Isometry2d::from_translation(at),
                40.0 + 12.0 * mark.mods.len() as f32,
                color.with_alpha(pulse),
            );
        }
    }
    // Tentáculos do Kraken: seis braços ondulando em volta do casco.
    for visual in &visuals {
        let state = &visual.target;
        if state.faction != Faction::Monster {
            continue;
        }
        let center = Vec2::new(state.x, state.y);
        for arm in 0..6 {
            let base = arm as f32 * std::f32::consts::TAU / 6.0 + t * 0.4;
            let mut previous = center;
            for segment in 1..=5 {
                let s = segment as f32;
                let angle = base + (t * 3.0 + s * 0.8 + arm as f32).sin() * 0.35;
                let point = center + Vec2::from_angle(angle) * (s * 9.0);
                gizmos.line_2d(previous, point, Color::srgb(0.55, 0.20, 0.65));
                previous = point;
            }
        }
    }
}

/// Rumo em pontos cardeais (convenção do mapa: +Y norte, +X leste).
pub fn bearing_label(from: Vec2, to: Vec2) -> &'static str {
    let delta = to - from;
    let angle = delta.y.atan2(delta.x).to_degrees().rem_euclid(360.0);
    const LABELS: [&str; 8] = ["L", "NE", "N", "NO", "O", "SO", "S", "SE"];
    LABELS[(((angle + 22.5) / 45.0) as usize) % 8]
}

fn distance_label(meters: f32) -> String {
    if meters >= 1000.0 {
        format!("{:.1} km", meters / 1000.0)
    } else {
        format!("{meters:.0} m")
    }
}

/// Linha de bordo: tripulação, leme, reparo e escavação. As teclas moram
/// nos bilhetes de contexto e no livreto (F1), não aqui.
pub fn sea_status_line(state: &ShipState) -> String {
    use crate::i18n::trf;
    let mut line = format!(
        "{}  ·  {}",
        trf(
            "Tripulação {0}/{1}",
            &[&state.crew.to_string(), &state.crew_max.to_string()]
        ),
        trf("Leme {0}%", &[&format!("{:.0}", state.rudder_hp)]),
    );
    if state.repairing {
        line.push_str("  ·  ");
        line.push_str(&crate::i18n::tr("REPARANDO"));
    }
    if state.dig_progress > 0.0 {
        line.push_str("  ·  ");
        line.push_str(&trf(
            "Cavando {0}%",
            &[&format!("{:.0}", state.dig_progress * 100.0)],
        ));
    }
    line
}

// System Bevy: params são injeção de dependência, não assinatura.
#[allow(clippy::type_complexity, clippy::too_many_arguments)]
fn update_sea_hud(
    my_ship: Res<MyShip>,
    world: Option<Res<crate::world::ClientWorld>>,
    events: Res<SeaEvents>,
    marks: Res<TreasureMarks>,
    visuals: Query<&ShipVisual>,
    mut texts: ParamSet<(
        Query<&mut Text, With<SeaStatusText>>,
        Query<&mut Text, With<EventBannerText>>,
        Query<&mut Text, With<EventHeadline>>,
    )>,
    mut strip: Query<&mut Node, With<EventStrip>>,
    announcements: Query<
        (),
        Or<(
            With<crate::hud::ZoneBannerPanel>,
            With<crate::hud::PvpWarningPanel>,
        )>,
    >,
) {
    let Some(mine) = my_state(&my_ship, &visuals) else {
        return;
    };
    // Distância e rumo no mapa náutico: um evento em outra zona fica a
    // quantos portões de viagem, não a quanto no plano do mundo.
    let on_chart = |x: f32, y: f32| {
        let (cx, cy) = world.as_ref().map_or((x, y), |w| w.0.chart_position(x, y));
        Vec2::new(cx, cy)
    };
    let here = on_chart(mine.x, mine.y);
    let status = sea_status_line(mine);
    for mut text in &mut texts.p0() {
        if text.0 != status {
            text.0 = status.clone();
        }
    }
    let where_is = |x: f32, y: f32| {
        let there = on_chart(x, y);
        format!(
            "{} {}",
            distance_label(here.distance(there)),
            crate::i18n::tr(bearing_label(here, there))
        )
    };
    // Manchete: o primeiro evento; o resto (e os mapas) vira linha miúda.
    let headline = events
        .0
        .first()
        .map(|event| crate::i18n::tr(&event.name).to_uppercase())
        .or_else(|| marks.0.first().map(|_| crate::i18n::tr("MAPA DO TESOURO")))
        .unwrap_or_default();
    let mut lines: Vec<String> = Vec::new();
    for (index, event) in events.0.iter().enumerate() {
        let secs = event.remaining_secs as u32;
        let clock = format!("{}:{:02}", secs / 60, secs % 60);
        let detail = format!("{clock}  ·  {}", where_is(event.x, event.y));
        lines.push(if index == 0 {
            detail
        } else {
            format!("{} — {detail}", crate::i18n::tr(&event.name))
        });
    }
    for mark in &marks.0 {
        let detail = format!("{}  ·  {}", mark.island, where_is(mark.x, mark.y));
        lines.push(if events.0.is_empty() && lines.is_empty() {
            detail
        } else {
            format!("{} — {detail}", crate::i18n::tr("Mapa do tesouro"))
        });
        // v26: perigos e o tamanho do baú, logo abaixo do destino.
        if !mark.mods.is_empty() {
            lines.push(map_mods_line(mark.rarity, &mark.mods));
        }
    }
    let joined = lines.join("\n");
    for mut text in &mut texts.p1() {
        if text.0 != joined {
            text.0 = joined.clone();
        }
    }
    for mut text in &mut texts.p2() {
        if text.0 != headline {
            text.0 = headline.clone();
        }
    }
    // Uma manchete por vez: o aviso de zona tem a vez enquanto está na tela.
    let display = if headline.is_empty() || !announcements.is_empty() {
        Display::None
    } else {
        Display::Flex
    };
    for mut node in &mut strip {
        if node.display != display {
            node.display = display;
        }
    }
}

#[cfg(test)]
pub(crate) fn init_systems_for_tests(world: &mut World) {
    fn init<M>(world: &mut World, system: impl IntoSystem<(), (), M>) {
        let mut system = IntoSystem::into_system(system);
        system.initialize(world);
    }
    init(world, update_sea_hud);
    init(world, send_sea_input);
    init(world, receive_sea_state);
    init(world, draw_sea_marks);
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn map_line_names_the_dangers_and_the_prize() {
        use marvyr_domain_items::{MapMod, Rarity};
        assert_eq!(
            map_mods_line(
                Rarity::Rare,
                &[MapMod::Guarded, MapMod::Kraken, MapMod::Bedrock]
            ),
            "[Raro] Guardado · Covil do Kraken · Rocha Dura · +145% no baú"
        );
    }

    fn state(id: u32, x: f32, faction: Faction) -> ShipState {
        ShipState {
            ship_id: id,
            kind: marvyr_domain_ships::ShipKind::SmallMerchant,
            x,
            y: 0.0,
            heading: 0.0,
            speed: 0.0,
            cargo_weight: 0,
            hp: 100,
            max_hp: 100,
            max_speed: 30.0,
            weapon_damage: 10,
            weapon_range: 50.0,
            port_cooldown_secs: 0.0,
            starboard_cooldown_secs: 0.0,
            is_npc: false,
            cargo_capacity: 100,
            sail_hp: 100.0,
            ammo: Default::default(),
            faction,
            notoriety_tier: 0,
            rudder_hp: 80.0,
            crew: 6,
            crew_max: 8,
            repairing: true,
            dig_progress: 0.5,
            sail_cosmetic: 0,
            flag_cosmetic: 0,
            black_flag: 0,
            fire_target: None,
            aura: 0,
            flasks: Default::default(),
            elite: 0,
        }
    }

    #[test]
    fn boarding_picks_nearest_ship_in_reach_never_a_monster() {
        let mine = state(1, 0.0, Faction::Player);
        let others = [
            mine,
            state(2, 40.0, Faction::Merchant),
            state(3, 20.0, Faction::Monster),
            state(4, 30.0, Faction::Pirate),
            state(5, 90.0, Faction::Player),
        ];
        assert_eq!(board_target(&mine, &others), Some(4));
        assert_eq!(
            board_target(&mine, &[mine, state(9, 200.0, Faction::Player)]),
            None
        );
    }

    #[test]
    fn bearings_follow_the_map_compass() {
        assert_eq!(bearing_label(Vec2::ZERO, Vec2::new(0.0, 100.0)), "N");
        assert_eq!(bearing_label(Vec2::ZERO, Vec2::new(100.0, 0.0)), "L");
        assert_eq!(bearing_label(Vec2::ZERO, Vec2::new(-100.0, -100.0)), "SO");
    }

    #[test]
    fn status_line_shows_crew_rudder_and_work() {
        let line = sea_status_line(&state(1, 0.0, Faction::Player));
        assert!(line.contains("Tripulação 6/8"));
        assert!(line.contains("Leme 80%"));
        assert!(line.contains("REPARANDO"));
        assert!(line.contains("Cavando 50%"));
        assert!(!line.contains('\n'), "teclas saíram da linha de bordo");
    }
}
