//! v35: Diário de Bordo no client. `F2` abre o painel com as metas de hoje
//! e da semana; meta cumprida estoura festa no casco. Tudo vem do
//! `ProgressSnapshot` do servidor — aqui só se mostra.

use bevy::prelude::*;
use lightyear::prelude::ClientReceiveMessage;
use marvyr_protocol::{GoalLine, ProgressSnapshot};

use crate::camera::CameraShake;
use crate::i18n::{tr, trf};
use crate::net::MyShip;
use crate::session::ConnectionStatus;
use crate::ship::ShipVisual;
use crate::ui;

#[derive(Resource, Debug, Default)]
pub struct MyProgress(pub Option<ProgressSnapshot>);

#[derive(Component)]
struct LogbookOverlay;

pub struct LogbookPlugin;

impl Plugin for LogbookPlugin {
    fn build(&self, app: &mut App) {
        app.init_resource::<MyProgress>()
            .add_systems(Update, (receive_progress, toggle_logbook).chain());
    }
}

/// Posição do meu navio (festas no casco).
pub fn my_position(my_ship: &MyShip, visuals: &Query<&ShipVisual>) -> Option<Vec2> {
    visuals
        .iter()
        .find(|visual| Some(visual.target.ship_id) == my_ship.0)
        .map(|visual| Vec2::new(visual.target.x, visual.target.y))
}

/// Metas que passaram pelo alvo entre o snapshot velho e o novo.
fn newly_done<'a>(old: &ProgressSnapshot, new: &'a ProgressSnapshot) -> Vec<&'a GoalLine> {
    new.goals
        .iter()
        .zip(&old.goals)
        .filter(|(now, before)| {
            now.template == before.template
                && now.progress >= now.target
                && before.progress < before.target
        })
        .map(|(now, _)| now)
        .collect()
}

fn receive_progress(
    mut commands: Commands,
    mut events: EventReader<ClientReceiveMessage<ProgressSnapshot>>,
    mut mine: ResMut<MyProgress>,
    my_ship: Res<MyShip>,
    visuals: Query<&ShipVisual>,
    mut shake: ResMut<CameraShake>,
) {
    for event in events.read() {
        let new = event.message().clone();
        if let (Some(old), Some(at)) = (&mine.0, my_position(&my_ship, &visuals)) {
            for goal in newly_done(old, &new) {
                crate::juice::celebrate_burst(
                    &mut commands,
                    &mut shake,
                    at,
                    tr("META CUMPRIDA!"),
                    (ui::BRASS_INK, Color::srgb(0.55, 0.85, 1.0)),
                );
                info!(template = goal.template, "meta do Diário cumprida");
            }
        }
        mine.0 = Some(new);
    }
}

fn toggle_logbook(
    mut commands: Commands,
    keys: Res<ButtonInput<KeyCode>>,
    status: Res<ConnectionStatus>,
    progress: Res<MyProgress>,
    open: Query<Entity, With<LogbookOverlay>>,
    (time, mut shot_at): (Res<Time>, Local<Option<Option<f32>>>),
) {
    let close = |commands: &mut Commands| {
        for entity in &open {
            commands.entity(entity).despawn_recursive();
        }
    };
    if *status != ConnectionStatus::InGame {
        close(&mut commands);
        return;
    }
    let is_open = !open.is_empty();
    // Dev (captura de tela): MARVYR_SHOT_LOGBOOK=<s> abre o Diário uma vez.
    let after = shot_at.get_or_insert_with(|| {
        std::env::var("MARVYR_SHOT_LOGBOOK")
            .ok()
            .and_then(|raw| raw.parse::<f32>().ok())
    });
    let shot = after.is_some_and(|after| time.elapsed_secs() >= after);
    if shot {
        *after = None;
    }
    let toggled = keys.just_pressed(KeyCode::F2) || shot;
    if toggled && is_open {
        close(&mut commands);
        return;
    }
    if !(toggled || (is_open && progress.is_changed())) {
        return;
    }
    close(&mut commands);
    debug!(loaded = progress.0.is_some(), "Diário aberto");
    spawn_panel(&mut commands, progress.0.as_ref());
}

fn spawn_panel(commands: &mut Commands, progress: Option<&ProgressSnapshot>) {
    commands
        .spawn((
            LogbookOverlay,
            Node {
                width: Val::Percent(100.0),
                height: Val::Percent(100.0),
                position_type: PositionType::Absolute,
                align_items: AlignItems::Center,
                justify_content: JustifyContent::Center,
                ..default()
            },
            BackgroundColor(Color::srgba(0.02, 0.05, 0.09, 0.6)),
            GlobalZIndex(40),
        ))
        .with_children(|root| {
            root.spawn(ui::panel(Node {
                padding: UiRect::all(Val::Px(18.0)),
                flex_direction: FlexDirection::Column,
                row_gap: Val::Px(8.0),
                min_width: Val::Px(420.0),
                ..default()
            }))
            .with_children(|frame| {
                frame.spawn(ui::display(tr("Diário de Bordo"), 28.0, ui::INK));
                let Some(progress) = progress else {
                    frame.spawn(ui::text(tr("Carregando o Diário..."), 14.0, ui::TEXT_DIM));
                    return;
                };
                frame.spawn(ui::text(tr("Hoje"), 16.0, ui::BRASS_INK));
                for goal in progress.goals.iter().filter(|g| !g.weekly) {
                    spawn_goal(frame, goal);
                }
                frame.spawn(ui::text(tr("Esta semana"), 16.0, ui::BRASS_INK));
                for goal in progress.goals.iter().filter(|g| g.weekly) {
                    spawn_goal(frame, goal);
                }
                if progress.abyss_best > 0 {
                    frame.spawn(ui::text(
                        trf(
                            "Abismo: recorde na camada {0}",
                            &[&progress.abyss_best.to_string()],
                        ),
                        14.0,
                        crate::seafaring::ABYSS_VIOLET,
                    ));
                }
                if !progress.unpaid.is_empty() {
                    let owed = progress
                        .unpaid
                        .iter()
                        .map(|(item, quantity)| format!("{quantity} {}", tr(item)))
                        .collect::<Vec<_>>()
                        .join(", ");
                    frame.spawn(ui::text(
                        trf("Esperando o próximo porto: {0}", &[&owed]),
                        13.0,
                        ui::BRASS_INK,
                    ));
                }
                frame.spawn(ui::text(tr("F2 fecha"), 12.0, ui::TEXT_DIM));
            });
        });
}

fn spawn_goal(frame: &mut ChildBuilder, goal: &GoalLine) {
    let done = goal.progress >= goal.target;
    let ratio = goal.progress as f32 / goal.target.max(1) as f32;
    frame
        .spawn(Node {
            flex_direction: FlexDirection::Column,
            row_gap: Val::Px(3.0),
            ..default()
        })
        .with_children(|row| {
            let title = trf(&goal.template, &[&goal.target.to_string()]);
            let reward = format!("{} {}", goal.reward_quantity, tr(&goal.reward_item));
            row.spawn(ui::text(
                format!(
                    "{}{title}  ·  {}/{}  ·  {reward}",
                    if done { "[OK] " } else { "" },
                    goal.progress.min(goal.target),
                    goal.target
                ),
                14.0,
                if done { ui::BRASS_INK } else { ui::TEXT },
            ));
            row.spawn((
                Node {
                    width: Val::Px(400.0),
                    height: Val::Px(6.0),
                    ..default()
                },
                BackgroundColor(ui::BAR_TRACK),
            ))
            .with_children(|track| {
                track.spawn((
                    Node {
                        width: Val::Percent(ratio.min(1.0) * 100.0),
                        height: Val::Percent(100.0),
                        ..default()
                    },
                    BackgroundColor(if done {
                        ui::BRASS
                    } else {
                        ui::BRASS.with_alpha(0.6)
                    }),
                ));
            });
        });
}

#[cfg(test)]
mod tests {
    use super::*;

    fn goal(progress: u32) -> GoalLine {
        GoalLine {
            template: String::from("Afunde {0} navios"),
            target: 3,
            progress,
            reward_item: String::from("Minério"),
            reward_quantity: 10,
            weekly: false,
        }
    }

    #[test]
    fn only_the_crossing_counts_as_done() {
        let snap = |p| ProgressSnapshot {
            goals: vec![goal(p)],
            unpaid: Vec::new(),
            abyss_best: 0,
        };
        assert_eq!(newly_done(&snap(2), &snap(3)).len(), 1);
        assert!(newly_done(&snap(3), &snap(3)).is_empty());
        assert!(newly_done(&snap(0), &snap(1)).is_empty());
    }
}
