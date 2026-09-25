//! Rosa dos Ventos no client (MV-067): `I` abre a árvore. Clique num nó
//! liberado para aprender; no porto, "Redistribuir" devolve os pontos por
//! minério do armazém. O servidor valida tudo — aqui só se mostra e se pede.

use bevy::prelude::*;
use lightyear::prelude::client::*;
use lightyear::prelude::ClientReceiveMessage;
use marvyr_domain_ships::talents::{
    can_allocate, points_for_level, respec_cost, Branch, TalentNode, RESPEC_ITEM, TREE,
};
use marvyr_protocol::{AllocateTalent, RespecTalents, TalentsSnapshot};

use crate::i18n::{tr, trf};
use crate::net::{MyDocked, ReliableChannel};
use crate::renown::MyRenown;
use crate::session::ConnectionStatus;
use crate::ui;

const NODE_WIDTH: f32 = 116.0;
const NODE_HEIGHT: f32 = 54.0;
const ROWS: u8 = 5;

#[derive(Resource, Debug, Default)]
pub struct MyTalents(pub Vec<String>);

#[derive(Component)]
struct TalentOverlay;

#[derive(Component)]
enum TalentButton {
    Learn(&'static str),
    Respec,
}

pub struct TalentsPlugin;

impl Plugin for TalentsPlugin {
    fn build(&self, app: &mut App) {
        app.init_resource::<MyTalents>().add_systems(
            Update,
            (receive_talents, toggle_tree, click_tree, dev_autotalent),
        );
    }
}

fn receive_talents(
    mut events: EventReader<ClientReceiveMessage<TalentsSnapshot>>,
    mut mine: ResMut<MyTalents>,
) {
    if let Some(event) = events.read().last() {
        mine.0 = event.message().allocated.clone();
    }
}

fn level(renown: &MyRenown) -> u32 {
    renown.0.as_ref().map_or(1, |update| update.level)
}

/// "+3% giro · −5% casco".
pub fn effect_label(node: &TalentNode) -> String {
    node.effects
        .iter()
        .map(|(stat, pct)| {
            let sign = if *pct >= 0 { "+" } else { "−" };
            format!("{sign}{}% {}", pct.abs(), tr(stat.name()))
        })
        .collect::<Vec<_>>()
        .join(" · ")
}

// System Bevy: params são injeção de dependência, não assinatura.
#[allow(clippy::too_many_arguments)]
fn toggle_tree(
    mut commands: Commands,
    keys: Res<ButtonInput<KeyCode>>,
    status: Res<ConnectionStatus>,
    talents: Res<MyTalents>,
    renown: Res<MyRenown>,
    docked: Res<MyDocked>,
    open: Query<Entity, With<TalentOverlay>>,
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
    // Dev (captura de tela): MARVYR_SHOT_TALENTS=<s> abre a árvore uma vez.
    let after = shot_at.get_or_insert_with(|| {
        std::env::var("MARVYR_SHOT_TALENTS")
            .ok()
            .and_then(|raw| raw.parse::<f32>().ok())
    });
    let shot = after.is_some_and(|after| time.elapsed_secs() >= after);
    if shot {
        *after = None;
    }
    let is_open = !open.is_empty();
    let toggled = keys.just_pressed(KeyCode::KeyI) || shot;
    let changed = talents.is_changed() || renown.is_changed() || docked.is_changed();
    if toggled && is_open {
        close(&mut commands);
        return;
    }
    // Aberta e algo mudou: redesenha com o estado novo.
    if !(toggled || (is_open && changed)) {
        return;
    }
    close(&mut commands);
    spawn_tree(&mut commands, &talents.0, level(&renown), docked.0);
}

fn spawn_tree(commands: &mut Commands, allocated: &[String], level: u32, docked: bool) {
    let points = points_for_level(level);
    let free = points.saturating_sub(allocated.len() as u32);
    commands
        .spawn((
            TalentOverlay,
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
                align_items: AlignItems::Center,
                row_gap: Val::Px(10.0),
                ..default()
            }))
            .with_children(|frame| {
                frame.spawn(ui::display(tr("Rosa dos Ventos"), 28.0, ui::INK));
                frame.spawn(ui::text(
                    trf(
                        "Renome {0} · {1} ponto(s) livre(s)",
                        &[&level.to_string(), &free.to_string()],
                    ),
                    15.0,
                    if free > 0 {
                        ui::BRASS_INK
                    } else {
                        ui::TEXT_DIM
                    },
                ));
                frame
                    .spawn(Node {
                        column_gap: Val::Px(18.0),
                        ..default()
                    })
                    .with_children(|branches| {
                        for branch in Branch::ALL {
                            spawn_branch(branches, branch, allocated, points);
                        }
                    });
                frame
                    .spawn(Node {
                        column_gap: Val::Px(16.0),
                        align_items: AlignItems::Center,
                        ..default()
                    })
                    .with_children(|footer| {
                        footer.spawn(ui::text(
                            tr("Clique num nó liberado para aprender · I fecha"),
                            12.0,
                            ui::TEXT_DIM,
                        ));
                        if !allocated.is_empty() {
                            let label = if docked {
                                trf(
                                    "Redistribuir ({0} {1})",
                                    &[&respec_cost(allocated.len()).to_string(), &tr(RESPEC_ITEM)],
                                )
                            } else {
                                tr("Redistribuir só no porto")
                            };
                            let mut button =
                                footer.spawn(ui::button(Node::default(), ui::BUTTON_BG));
                            if docked {
                                button.insert(TalentButton::Respec);
                            }
                            button.with_children(|b| {
                                b.spawn(ui::text(label, 13.0, ui::TEXT));
                            });
                        }
                    });
            });
        });
}

fn spawn_branch(parent: &mut ChildBuilder, branch: Branch, allocated: &[String], points: u32) {
    parent
        .spawn(Node {
            flex_direction: FlexDirection::Column,
            align_items: AlignItems::Center,
            row_gap: Val::Px(6.0),
            ..default()
        })
        .with_children(|column| {
            column.spawn(ui::display(tr(branch.name()), 18.0, ui::INK));
            for row in 0..ROWS {
                column
                    .spawn(Node {
                        column_gap: Val::Px(4.0),
                        ..default()
                    })
                    .with_children(|line| {
                        for lane in 0..3 {
                            let node = TREE
                                .iter()
                                .find(|n| n.branch == branch && n.row == row && n.lane == lane);
                            match node {
                                Some(node) => spawn_node(line, node, allocated, points),
                                None => {
                                    line.spawn(Node {
                                        width: Val::Px(NODE_WIDTH),
                                        height: Val::Px(NODE_HEIGHT),
                                        ..default()
                                    });
                                }
                            }
                        }
                    });
            }
        });
}

fn spawn_node(
    parent: &mut ChildBuilder,
    node: &'static TalentNode,
    allocated: &[String],
    points: u32,
) {
    let is_learned = allocated.iter().any(|id| id == node.id);
    let is_open = can_allocate(allocated, node.id, points).is_ok();
    // Liberado mas sem ponto também aparece como caminho (borda), só não
    // aceita o clique — o servidor diria "sem pontos".
    let is_reachable = node
        .requires
        .map_or(true, |parent| allocated.iter().any(|id| id == parent));
    let (background, text) = match (is_learned, is_open, is_reachable) {
        (true, _, _) => (ui::BRASS, ui::INK),
        (_, true, _) => (ui::BUTTON_SELECTED.with_alpha(0.55), ui::INK),
        (_, _, true) => (ui::BUTTON_BG, ui::INK),
        _ => (ui::BAR_TRACK, ui::TEXT_DIM),
    };
    let mut entity = parent.spawn(ui::button(
        Node {
            width: Val::Px(NODE_WIDTH),
            height: Val::Px(NODE_HEIGHT),
            flex_direction: FlexDirection::Column,
            border: UiRect::all(Val::Px(if node.notable { 2.0 } else { 1.0 })),
            ..default()
        },
        background,
    ));
    if node.notable {
        entity.insert(BorderColor(ui::BRASS_INK));
    }
    if is_open {
        entity.insert(TalentButton::Learn(node.id));
    }
    entity.with_children(|b| {
        b.spawn(ui::text(tr(node.name), 12.0, text));
        b.spawn(ui::text(effect_label(node), 10.0, text));
    });
}

fn click_tree(
    buttons: Query<(&Interaction, &TalentButton), Changed<Interaction>>,
    mut connection_manager: ResMut<ConnectionManager>,
) {
    for (interaction, button) in &buttons {
        if *interaction != Interaction::Pressed {
            continue;
        }
        let _ = match button {
            TalentButton::Learn(id) => {
                connection_manager.send_message::<ReliableChannel, _>(&AllocateTalent {
                    node: (*id).to_owned(),
                })
            }
            TalentButton::Respec => {
                connection_manager.send_message::<ReliableChannel, _>(&RespecTalents)
            }
        };
    }
}

/// Dev (§39): MARVYR_AUTOTALENT=a,b,c aprende esses nós, em ordem, assim
/// que houver ponto — captura de tela sem mouse.
fn dev_autotalent(
    talents: Res<MyTalents>,
    renown: Res<MyRenown>,
    mut asked: Local<Vec<String>>,
    mut connection_manager: ResMut<ConnectionManager>,
) {
    let Ok(wanted) = std::env::var("MARVYR_AUTOTALENT") else {
        return;
    };
    let points = points_for_level(level(&renown));
    let next = wanted
        .split(',')
        .map(str::trim)
        .find(|id| !talents.0.iter().any(|taken| taken == id));
    let Some(id) = next else {
        return;
    };
    if asked.iter().any(|done| done == id) || can_allocate(&talents.0, id, points).is_err() {
        return;
    }
    asked.push(id.to_owned());
    let _ = connection_manager.send_message::<ReliableChannel, _>(&AllocateTalent {
        node: id.to_owned(),
    });
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn effect_label_shows_trades_with_signs() {
        let popa = marvyr_domain_ships::talents::find("nav.popa").unwrap();
        let label = effect_label(popa);
        assert!(label.contains("+5% velocidade"), "{label}");
        assert!(label.contains("−5% casco"), "{label}");
    }
}
