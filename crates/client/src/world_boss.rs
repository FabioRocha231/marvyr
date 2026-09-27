//! v34: barra de vida do Leviatã (chefe de mundo). Aparece no topo quando
//! o chefe está perto do seu navio; a vida vem do `ShipState` do servidor.

use bevy::prelude::*;
use marvyr_domain_combat::elite;

use crate::net::MyShip;
use crate::ship::ShipVisual;
use crate::ui;

/// Roxo do Leviatã (placa, aura, barra).
pub const LEVIATHAN: Color = Color::srgb(0.62, 0.22, 0.85);
/// Distância (m) até onde a barra acompanha o chefe.
const BAR_RANGE: f32 = 900.0;
const BAR_WIDTH: f32 = 320.0;

#[derive(Component)]
struct BossBar;
#[derive(Component)]
struct BossBarFill;
#[derive(Component)]
struct BossBarText;

pub fn is_boss(state: &marvyr_protocol::ShipState) -> bool {
    state.elite & elite::BOSS != 0
}

pub struct WorldBossPlugin;

impl Plugin for WorldBossPlugin {
    fn build(&self, app: &mut App) {
        app.add_systems(Startup, spawn_bar)
            .add_systems(Update, update_bar);
    }
}

fn spawn_bar(mut commands: Commands) {
    commands
        .spawn((
            Node {
                position_type: PositionType::Absolute,
                top: Val::Px(176.0),
                width: Val::Percent(100.0),
                justify_content: JustifyContent::Center,
                display: Display::None,
                ..default()
            },
            BossBar,
        ))
        .with_children(|root| {
            root.spawn(ui::panel(Node {
                flex_direction: FlexDirection::Column,
                align_items: AlignItems::Center,
                row_gap: Val::Px(4.0),
                ..default()
            }))
            .with_children(|panel| {
                panel.spawn((ui::text("LEVIATÃ", 16.0, LEVIATHAN), BossBarText));
                panel
                    .spawn((
                        Node {
                            width: Val::Px(BAR_WIDTH),
                            height: Val::Px(12.0),
                            ..default()
                        },
                        BackgroundColor(Color::srgb(0.2, 0.12, 0.22)),
                    ))
                    .with_children(|track| {
                        track.spawn((
                            Node {
                                width: Val::Percent(100.0),
                                height: Val::Percent(100.0),
                                ..default()
                            },
                            BackgroundColor(LEVIATHAN),
                            BossBarFill,
                        ));
                    });
            });
        });
}

fn update_bar(
    time: Res<Time>,
    my_ship: Res<MyShip>,
    visuals: Query<&ShipVisual>,
    mut bar: Query<&mut Node, (With<BossBar>, Without<BossBarFill>)>,
    mut fill: Query<(&mut Node, &mut BackgroundColor), With<BossBarFill>>,
) {
    let me = visuals
        .iter()
        .find(|v| Some(v.target.ship_id) == my_ship.0)
        .map(|v| Vec2::new(v.target.x, v.target.y));
    let boss = visuals.iter().map(|v| &v.target).find(|state| {
        is_boss(state) && me.is_some_and(|me| me.distance(Vec2::new(state.x, state.y)) < BAR_RANGE)
    });
    let Ok(mut root) = bar.get_single_mut() else {
        return;
    };
    let display = if boss.is_some() {
        Display::Flex
    } else {
        Display::None
    };
    if root.display != display {
        root.display = display;
    }
    let (Some(boss), Ok((mut node, mut color))) = (boss, fill.get_single_mut()) else {
        return;
    };
    let ratio = boss.hp as f32 / boss.max_hp.max(1) as f32;
    node.width = Val::Percent(ratio * 100.0);
    // Abaixo de um quarto, a barra pulsa: é a hora de apertar.
    let alpha = if ratio < 0.25 {
        0.6 + 0.4 * (time.elapsed_secs() * 8.0).sin().abs()
    } else {
        1.0
    };
    color.0 = LEVIATHAN.with_alpha(alpha);
}
