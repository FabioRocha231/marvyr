//! Duelo de abordagem no client (v61). O servidor decide rodada, baixas e
//! rendição; aqui só o painel (rodada, relógio, marujos, o que o NPC
//! anuncia), as três táticas (1/2/3 ou clique) e o tranco de cada rodada.
//! Enquanto o painel está aberto, 1-3 são táticas, não frascos.

use bevy::prelude::*;
use lightyear::prelude::client::*;
use lightyear::prelude::*;
use marvyr_domain_combat::melee::Tactic;
use marvyr_protocol::{BoardShip, BoardTactic, MeleeUpdate, ShipState};

use crate::camera::CameraShake;
use crate::net::{MyShip, ReliableChannel};
use crate::ship::ShipVisual;
use crate::ui;

const KEYS: [KeyCode; 3] = [KeyCode::Digit1, KeyCode::Digit2, KeyCode::Digit3];

pub struct MeleePlugin;

impl Plugin for MeleePlugin {
    fn build(&self, app: &mut App) {
        app.init_resource::<MeleeView>()
            .add_systems(Startup, spawn_panel)
            .add_systems(
                Update,
                (
                    receive_melee,
                    send_tactic,
                    auto_board,
                    rebuild_panel,
                    tick_clock,
                )
                    .chain(),
            );
    }
}

/// Último estado do duelo e quando chegou (o relógio corre aqui).
#[derive(Resource, Default)]
pub struct MeleeView {
    pub state: Option<MeleeUpdate>,
    received_at: f32,
}

impl MeleeView {
    /// Duelo em curso: 1-3 são táticas.
    pub fn active(&self) -> bool {
        self.state.as_ref().is_some_and(|state| state.active)
    }
}

#[derive(Component)]
struct MeleePanel;

#[derive(Component)]
struct MeleeClock;

#[derive(Component)]
struct TacticButton(Tactic);

fn spawn_panel(mut commands: Commands) {
    commands.spawn((
        ui::panel(Node {
            position_type: PositionType::Absolute,
            left: Val::Percent(50.0),
            top: Val::Percent(12.0),
            margin: UiRect::left(Val::Px(-190.0)),
            width: Val::Px(380.0),
            flex_direction: FlexDirection::Column,
            row_gap: Val::Px(6.0),
            padding: UiRect::axes(Val::Px(14.0), Val::Px(10.0)),
            display: Display::None,
            ..default()
        }),
        crate::hud::SeaHud,
        MeleePanel,
    ));
}

fn my_visual<'a>(my_ship: &MyShip, visuals: &'a Query<&ShipVisual>) -> Option<&'a ShipState> {
    let id = my_ship.0?;
    visuals
        .iter()
        .find(|visual| visual.target.ship_id == id)
        .map(|visual| &visual.target)
}

/// Guarda o estado; cada rodada resolvida vira letreiro e tranco.
fn receive_melee(
    mut commands: Commands,
    time: Res<Time>,
    mut events: EventReader<ClientReceiveMessage<MeleeUpdate>>,
    mut view: ResMut<MeleeView>,
    mut shake: ResMut<CameraShake>,
    my_ship: Res<MyShip>,
    visuals: Query<&ShipVisual>,
) {
    for event in events.read() {
        let update = event.message().clone();
        let new_round = update.last.is_some()
            && view
                .state
                .as_ref()
                .map(|old| (old.round, old.active, old.last))
                != Some((update.round, update.active, update.last));
        let at = my_visual(&my_ship, &visuals).map(|me| Vec2::new(me.x, me.y + 50.0));
        if let (true, Some(at), Some((_, _, score))) = (new_round, at, update.last) {
            let (text, color) = if !update.active {
                match (update.attacker, update.captured) {
                    (true, true) => ("NAVIO TOMADO!", ui::BRASS),
                    (true, false) => ("ABORDAGEM REPELIDA", ui::VERMILION),
                    (false, true) => ("NAVIO PERDIDO", ui::VERMILION),
                    (false, false) => ("CONVÉS DEFENDIDO!", ui::BRASS),
                }
            } else {
                match score {
                    1 => ("RODADA VENCIDA!", ui::BRASS),
                    0 => ("EMPATE NO CONVÉS", ui::PAPER),
                    _ => ("RODADA PERDIDA", ui::VERMILION),
                }
            };
            crate::juice::spawn_float_text(&mut commands, at, crate::i18n::tr(text), color);
            shake.add(if score == 1 { 0.35 } else { 0.25 });
        }
        view.state = Some(update);
        view.received_at = time.elapsed_secs();
    }
}

fn send_tactic(
    keys: Res<ButtonInput<KeyCode>>,
    time: Res<Time>,
    view: Res<MeleeView>,
    buttons: Query<(&Interaction, &TacticButton), Changed<Interaction>>,
    mut connection_manager: ResMut<ConnectionManager>,
) {
    let Some(state) = view.state.as_ref().filter(|state| state.active) else {
        return;
    };
    if state.my_pick != 0 {
        return;
    }
    let mut pick = Tactic::ALL
        .into_iter()
        .zip(KEYS)
        .find(|(_, key)| keys.just_pressed(*key))
        .map(|(tactic, _)| tactic);
    for (interaction, button) in &buttons {
        if *interaction == Interaction::Pressed {
            pick = Some(button.0);
        }
    }
    // Dev (teste ao vivo sem teclado): MARVYR_AUTOBOARD responde o anúncio
    // com o que o vence, 1 s depois da rodada abrir.
    if pick.is_none()
        && std::env::var_os("MARVYR_AUTOBOARD").is_some()
        && time.elapsed_secs() - view.received_at > 1.0
    {
        pick = Some(Tactic::from_code(state.hint).map_or(Tactic::Charge, Tactic::counter));
    }
    if let Some(tactic) = pick {
        let _ = connection_manager.send_message::<ReliableChannel, _>(&BoardTactic {
            tactic: tactic.code(),
        });
    }
}

/// Dev: MARVYR_AUTOBOARD também lança os ganchos quando há alvo abordável.
fn auto_board(
    time: Res<Time>,
    mut clock: Local<f32>,
    view: Res<MeleeView>,
    my_ship: Res<MyShip>,
    visuals: Query<&ShipVisual>,
    mut connection_manager: ResMut<ConnectionManager>,
) {
    if std::env::var_os("MARVYR_AUTOBOARD").is_none() || view.active() {
        return;
    }
    *clock += time.delta_secs();
    if *clock < 1.5 {
        return;
    }
    *clock = 0.0;
    let Some(mine) = my_visual(&my_ship, &visuals) else {
        return;
    };
    let others: Vec<ShipState> = visuals.iter().map(|visual| visual.target).collect();
    if let Some(target_ship_id) = crate::seafaring::board_target(mine, &others) {
        let _ =
            connection_manager.send_message::<ReliableChannel, _>(&BoardShip { target_ship_id });
    }
}

/// Remonta o painel a cada mensagem (poucos nós: barato).
fn rebuild_panel(
    mut commands: Commands,
    view: Res<MeleeView>,
    lang: Res<crate::i18n::Lang>,
    mut panels: Query<(Entity, &mut Node), With<MeleePanel>>,
) {
    if !(view.is_changed() || lang.is_changed()) {
        return;
    }
    use crate::i18n::{tr, trf};
    let state = view.state.as_ref().filter(|state| state.active);
    for (entity, mut node) in &mut panels {
        node.display = if state.is_some() {
            Display::Flex
        } else {
            Display::None
        };
        commands.entity(entity).despawn_descendants();
        let Some(state) = state else {
            continue;
        };
        commands.entity(entity).with_children(|panel| {
            panel
                .spawn(Node {
                    justify_content: JustifyContent::SpaceBetween,
                    align_items: AlignItems::Center,
                    ..default()
                })
                .with_children(|head| {
                    let title = if state.attacker {
                        "ABORDAGEM"
                    } else {
                        "DEFENDA O CONVÉS"
                    };
                    head.spawn(ui::display(tr(title), 20.0, ui::VERMILION_INK));
                    head.spawn((ui::face("", ui::FONT_BOLD, 16.0, ui::INK), MeleeClock));
                });
            panel.spawn(ui::face(
                format!(
                    "{}  ·  {}  ·  {}",
                    trf("Rodada {0}/3", &[&state.round.to_string()]),
                    trf(
                        "Marujos {0} × {1}",
                        &[&state.my_crew.to_string(), &state.their_crew.to_string()]
                    ),
                    trf(
                        "Placar {0} × {1}",
                        &[&state.my_wins.to_string(), &state.their_wins.to_string()]
                    ),
                ),
                ui::FONT_BOLD,
                13.0,
                ui::INK_SOFT,
            ));
            if let Some(hint) = Tactic::from_code(state.hint) {
                panel.spawn(ui::face(
                    trf("Eles gritam: {0}! (às vezes blefam)", &[&tr(hint.name())]),
                    ui::FONT_BOLD,
                    14.0,
                    ui::AMBER,
                ));
            }
            for (tactic, key) in Tactic::ALL.into_iter().zip(["1", "2", "3"]) {
                let chosen = state.my_pick == tactic.code();
                panel
                    .spawn((
                        Button,
                        Node {
                            align_items: AlignItems::Center,
                            column_gap: Val::Px(8.0),
                            padding: UiRect::axes(Val::Px(8.0), Val::Px(4.0)),
                            ..default()
                        },
                        BackgroundColor(if chosen {
                            ui::BUTTON_SELECTED
                        } else {
                            ui::BUTTON_BG
                        }),
                        TacticButton(tactic),
                    ))
                    .with_children(|row| {
                        ui::keycap(row, key);
                        row.spawn(ui::face(tr(tactic.name()), ui::FONT_BOLD, 15.0, ui::INK));
                        let beaten = Tactic::ALL
                            .into_iter()
                            .find(|other| tactic.beats(*other))
                            .unwrap_or(tactic);
                        row.spawn(ui::face(
                            trf("vence {0}", &[&tr(beaten.name())]),
                            ui::FONT_BOLD,
                            12.0,
                            ui::INK_SOFT,
                        ));
                    });
            }
            if let Some((mine, theirs, score)) = state.last {
                let (Some(mine), Some(theirs)) =
                    (Tactic::from_code(mine), Tactic::from_code(theirs))
                else {
                    return;
                };
                let verdict = match score {
                    1 => "venceu",
                    0 => "empate",
                    _ => "perdeu",
                };
                panel.spawn(ui::face(
                    trf(
                        "Rodada passada: {0} × {1} ({2})",
                        &[&tr(mine.name()), &tr(theirs.name()), &tr(verdict)],
                    ),
                    ui::FONT_BOLD,
                    12.0,
                    ui::INK_SOFT,
                ));
            }
        });
    }
}

fn tick_clock(
    time: Res<Time>,
    view: Res<MeleeView>,
    mut clocks: Query<&mut Text, With<MeleeClock>>,
) {
    let Some(state) = view.state.as_ref().filter(|state| state.active) else {
        return;
    };
    let left = (state.secs_left - (time.elapsed_secs() - view.received_at)).max(0.0);
    for mut text in &mut clocks {
        let value = format!("{left:.1}s");
        if text.0 != value {
            text.0 = value;
        }
    }
}
