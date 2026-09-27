//! Feed de eventos e selo de procurado (MF-059), bevy_ui em espaço de tela.
//! O servidor decide tudo (notoriedade, faixa, recompensa); aqui só se
//! mostra: `WorldEvent` vira uma linha no feed da direita que some em ~6 s,
//! `ReputationUpdate` vira o selo sob o painel do navio.

use bevy::prelude::*;
use lightyear::prelude::ClientReceiveMessage;
use marvyr_protocol::{
    DockResult, ReputationUpdate, WorldEvent, WorldEventKind, TIER_PROCURADO, TIER_SUSPEITO,
};

use crate::hud::{spawn_faded_panel, SeaHud};
use crate::i18n::{tr, trf};
use crate::ui;

/// Linhas visíveis no feed; a mais antiga sai quando chega uma nova. Três
/// cabem entre o painel de vento e os toasts (40% da tela).
const FEED_MAX: usize = 3;
/// Abaixo do painel de vento (canto superior direito, ~140 px).
const FEED_TOP: f32 = 150.0;
/// Fade in, fim do hold, fim do fade out (s).
const FEED_FADE: (f32, f32, f32) = (0.3, 5.2, 6.0);
/// Logo abaixo do painel do navio (topo-esquerda).
const BADGE_TOP: f32 = 128.0;

/// Última ficha do PRÓPRIO capitão vinda do servidor.
#[derive(Resource, Debug, Default, Clone, Copy)]
pub struct MyReputation(pub Option<ReputationUpdate>);

#[derive(Component)]
pub struct KillFeed;

#[derive(Component)]
pub struct FeedEntry;

#[derive(Component)]
pub struct WantedBadge;

#[derive(Component)]
pub struct WantedBadgeText;

pub struct WantedHudPlugin;

impl Plugin for WantedHudPlugin {
    fn build(&self, app: &mut App) {
        app.init_resource::<MyReputation>()
            .add_systems(Startup, setup_wanted_hud)
            .add_systems(
                Update,
                (receive_reputation, update_feed, update_wanted_badge),
            );
    }
}

pub fn setup_wanted_hud(mut commands: Commands) {
    commands.spawn((
        Node {
            position_type: PositionType::Absolute,
            right: Val::Px(ui::MARGIN),
            top: Val::Px(FEED_TOP),
            flex_direction: FlexDirection::Column,
            align_items: AlignItems::End,
            row_gap: Val::Px(4.0),
            ..default()
        },
        SeaHud,
        KillFeed,
    ));
    commands
        .spawn((
            ui::panel(Node {
                position_type: PositionType::Absolute,
                left: Val::Px(ui::MARGIN),
                top: Val::Px(BADGE_TOP),
                display: Display::None,
                ..default()
            }),
            SeaHud,
            WantedBadge,
        ))
        .with_children(|badge| {
            badge.spawn((ui::text("", 13.0, ui::DANGER), WantedBadgeText));
        });
}

/// Texto e cor do selo; `None` = Honrado (sem selo).
pub fn badge_line(reputation: &ReputationUpdate) -> Option<(String, Color)> {
    match reputation.tier {
        TIER_PROCURADO => Some((
            trf(
                "PROCURADO - notoriedade {0}",
                &[&reputation.notoriety.to_string()],
            ),
            ui::DANGER,
        )),
        TIER_SUSPEITO => Some((
            trf(
                "SUSPEITO - notoriedade {0}",
                &[&reputation.notoriety.to_string()],
            ),
            ui::AMBER,
        )),
        _ => None,
    }
}

fn event_color(kind: WorldEventKind) -> Color {
    match kind {
        WorldEventKind::Kill => ui::GOLD,
        WorldEventKind::Alert => ui::AMBER,
        WorldEventKind::Bounty => ui::DANGER,
    }
}

fn receive_reputation(
    mut events: EventReader<ClientReceiveMessage<ReputationUpdate>>,
    mut mine: ResMut<MyReputation>,
) {
    if let Some(event) = events.read().last() {
        mine.0 = Some(*event.message());
    }
}

/// Um só sistema para eventos e recusas: com a lista de filhos do começo do
/// frame, dois leitores separados passavam de `FEED_MAX` e despawnavam a
/// mesma linha duas vezes.
fn update_feed(
    mut commands: Commands,
    mut world_events: EventReader<ClientReceiveMessage<WorldEvent>>,
    mut dock_results: EventReader<ClientReceiveMessage<DockResult>>,
    mut actions: EventReader<ClientReceiveMessage<marvyr_protocol::ActionResult>>,
    mut notices: EventReader<crate::net::PlayerNotice>,
    feed: Query<(Entity, Option<&Children>), With<KillFeed>>,
) {
    let mut lines: Vec<(String, Color)> = world_events
        .read()
        .map(|event| {
            let message = event.message();
            (tr(&message.text), event_color(message.kind))
        })
        .collect();
    // Porto da coroa recusou o Procurado: o motivo vai para o feed.
    if let Some(result) = dock_results
        .read()
        .map(|event| event.message())
        .filter(|result| !result.success && result.reason.starts_with("Procurado"))
        .last()
    {
        lines.push((tr(&result.reason), ui::DANGER));
    }
    // MV-061: veredito de reparo, abordagem, escavação e contratação.
    lines.extend(actions.read().map(|event| {
        let result = event.message();
        let color = if result.success {
            ui::OK_GREEN
        } else {
            ui::AMBER
        };
        (tr(&result.reason), color)
    }));
    // MV-062: recusas de coleta, saque e fabricação (e avisos do client).
    lines.extend(notices.read().map(|notice| (tr(&notice.0), ui::DANGER)));
    if lines.is_empty() {
        return;
    }
    let Ok((feed, children)) = feed.get_single() else {
        return;
    };
    let mut oldest: std::collections::VecDeque<Entity> = children
        .map(|c| c.iter().copied().collect())
        .unwrap_or_default();
    // Só as últimas FEED_MAX desta rajada importam.
    let skip = lines.len().saturating_sub(FEED_MAX);
    for (text, color) in lines.into_iter().skip(skip) {
        while oldest.len() >= FEED_MAX {
            if let Some(entry) = oldest.pop_front() {
                // A linha pode ter acabado de sumir pelo fade no mesmo frame.
                if let Some(entity) = commands.get_entity(entry) {
                    entity.despawn_recursive();
                }
            }
        }
        spawn_faded_panel(
            &mut commands,
            feed,
            FEED_FADE,
            color,
            FeedEntry,
            &[(&text, 13.0, color)],
        );
        // Placeholder: a linha nova ocupa uma vaga até o próximo frame.
        oldest.push_back(Entity::PLACEHOLDER);
    }
}

fn update_wanted_badge(
    mine: Res<MyReputation>,
    mut badge: Query<(&mut Node, &mut BorderColor), With<WantedBadge>>,
    mut text: Query<(&mut Text, &mut TextColor), With<WantedBadgeText>>,
) {
    if !mine.is_changed() {
        return;
    }
    let line = mine.0.as_ref().and_then(badge_line);
    // ReputationUpdate chega a cada 10 s igual: não suja o Node à toa.
    let display = if line.is_some() {
        Display::Flex
    } else {
        Display::None
    };
    for (mut node, mut border) in &mut badge {
        if node.display != display {
            node.display = display;
        }
        if let Some((_, color)) = &line {
            if border.0 != *color {
                border.0 = *color;
            }
        }
    }
    if let Some((value, color)) = line {
        for (mut text, mut text_color) in &mut text {
            if text.0 != value {
                text.0 = value.clone();
            }
            if text_color.0 != color {
                text_color.0 = color;
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use bevy::ecs::schedule::Schedule;

    use super::*;

    fn rep(notoriety: u32, tier: u8) -> ReputationUpdate {
        ReputationUpdate { notoriety, tier }
    }

    #[test]
    fn badge_shows_only_for_suspects_and_wanted() {
        assert!(badge_line(&rep(40, 0)).is_none());
        let (text, _) = badge_line(&rep(150, TIER_SUSPEITO)).unwrap();
        assert_eq!(text, "SUSPEITO - notoriedade 150");
        let (text, color) = badge_line(&rep(320, TIER_PROCURADO)).unwrap();
        assert_eq!(text, "PROCURADO - notoriedade 320");
        assert_eq!(color, ui::DANGER);
        assert_eq!(
            crate::i18n::trf_in(
                "PROCURADO - notoriedade {0}",
                &["320"],
                crate::i18n::Lang::En
            ),
            "WANTED - notoriety 320"
        );
    }

    #[test]
    fn badge_node_follows_my_reputation() {
        let mut world = World::new();
        world.init_resource::<MyReputation>();
        let mut setup = Schedule::default();
        setup.add_systems(setup_wanted_hud);
        setup.run(&mut world);

        let mut schedule = Schedule::default();
        schedule.add_systems(update_wanted_badge);
        world.resource_mut::<MyReputation>().0 = Some(rep(320, TIER_PROCURADO));
        schedule.run(&mut world);

        let mut nodes = world.query_filtered::<&Node, With<WantedBadge>>();
        assert_eq!(nodes.single(&world).display, Display::Flex);
        let mut texts = world.query_filtered::<&Text, With<WantedBadgeText>>();
        assert_eq!(texts.single(&world).0, "PROCURADO - notoriedade 320");

        world.resource_mut::<MyReputation>().0 = Some(rep(0, 0));
        schedule.run(&mut world);
        assert_eq!(nodes.single(&world).display, Display::None);
    }
}
