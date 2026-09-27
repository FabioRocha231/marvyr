//! v51: mensagem na garrafa no client. F3 abre o bilhete: setas escolhem
//! as três peças da frase, Enter joga ao mar. Garrafas boiando aparecem
//! para todo mundo; perto de uma, o bilhete de ação (B) pesca e a frase
//! aparece num cartão. Frase e regras são do servidor e do protocolo.

use bevy::prelude::*;
use lightyear::prelude::client::*;
use lightyear::prelude::ClientReceiveMessage;
use marvyr_protocol::bottle::{pieces, OPENINGS, PLACES, SUBJECTS};
use marvyr_protocol::{
    ActionKind, ActionResult, BottleRead, BottlesUpdate, PickBottle, ThrowBottle,
};

use crate::assets::layers;
use crate::i18n::{tr, trf};
use crate::input::{modal_keys, ModalKeys, ModalOpen};
use crate::net::{MyDocked, MyShip, ReliableChannel};
use crate::ship::ShipVisual;
use crate::ui;

const GLASS: Color = Color::srgb(0.35, 0.75, 0.45);
const CORK: Color = Color::srgb(0.6, 0.42, 0.25);

#[derive(Resource, Default)]
pub struct KnownBottles(pub Vec<(u32, Vec2)>);

/// O bilhete de escrever: aberto e as três escolhas.
#[derive(Resource, Default)]
struct Composer {
    open: bool,
    row: usize,
    words: [u8; 3],
}

#[derive(Component)]
struct BottleVisual(u32);

#[derive(Component)]
struct ComposerRoot;

#[derive(Component)]
struct ComposerText;

#[derive(Component)]
struct ReadCard {
    age: f32,
}

pub struct BottlePlugin;

impl Plugin for BottlePlugin {
    fn build(&self, app: &mut App) {
        app.init_resource::<KnownBottles>()
            .init_resource::<Composer>()
            .add_systems(Startup, setup_composer)
            .add_systems(
                Update,
                (
                    receive,
                    sync_visuals,
                    bob,
                    compose,
                    show_composer,
                    send_pick,
                    show_read,
                    fade_cards,
                    splash_on_throw,
                )
                    .chain(),
            );
    }
}

/// A frase montada (traduzida peça por peça).
pub fn sentence(words: [u8; 3]) -> String {
    match pieces(words) {
        Some([a, b, c]) => format!("{} {} {}.", tr(a), tr(b), tr(c)),
        None => String::new(),
    }
}

/// A garrafa mais perto dentro do alcance de pesca (para o bilhete).
pub fn bottle_near(known: &KnownBottles, at: Vec2) -> Option<Vec2> {
    known
        .0
        .iter()
        .map(|(_, p)| *p)
        .filter(|p| p.distance(at) <= PICK_RANGE)
        .min_by(|a, b| a.distance(at).total_cmp(&b.distance(at)))
}

/// Mesmo alcance do servidor (só para mostrar o bilhete).
pub const PICK_RANGE: f32 = 60.0;

fn receive(
    mut updates: EventReader<ClientReceiveMessage<BottlesUpdate>>,
    mut known: ResMut<KnownBottles>,
) {
    if let Some(update) = updates.read().last() {
        known.0 = update
            .message()
            .list
            .iter()
            .map(|(id, x, y)| (*id, Vec2::new(*x, *y)))
            .collect();
    }
}

fn sync_visuals(
    mut commands: Commands,
    known: Res<KnownBottles>,
    mut visuals: Query<(Entity, &BottleVisual, &mut Transform)>,
) {
    if !known.is_changed() {
        return;
    }
    for (entity, visual, mut transform) in &mut visuals {
        match known.0.iter().find(|(id, _)| *id == visual.0) {
            Some((_, at)) => {
                transform.translation.x = at.x;
                transform.translation.y = at.y;
            }
            None => commands.entity(entity).despawn_recursive(),
        }
    }
    for (id, at) in &known.0 {
        if visuals.iter().any(|(_, v, _)| v.0 == *id) {
            continue;
        }
        commands
            .spawn((
                BottleVisual(*id),
                Transform::from_translation(at.extend(layers::RESOURCES)),
                Visibility::default(),
            ))
            .with_children(|bottle| {
                bottle.spawn(Sprite::from_color(GLASS, Vec2::new(5.0, 11.0)));
                bottle.spawn((
                    Sprite::from_color(CORK, Vec2::new(3.0, 3.0)),
                    Transform::from_xyz(0.0, 7.0, 0.1),
                ));
            });
    }
}

/// A garrafa balança na água.
fn bob(time: Res<Time>, mut visuals: Query<(&BottleVisual, &mut Transform)>) {
    let t = time.elapsed_secs();
    for (visual, mut transform) in &mut visuals {
        let phase = visual.0 as f32 * 1.7;
        transform.rotation = Quat::from_rotation_z(0.35 * (t * 1.6 + phase).sin());
    }
}

/// F3 abre; setas escolhem; Enter joga; Esc fecha. Dev (captura):
/// `MARVYR_AUTOBOTTLE=<s>` abre o bilhete em `s` segundos e joga 2 s depois.
#[allow(clippy::too_many_arguments)]
fn compose(
    time: Res<Time>,
    mut auto: Local<Option<Option<f32>>>,
    keys: Res<ButtonInput<KeyCode>>,
    captured: Res<ModalKeys>,
    docked: Res<MyDocked>,
    mut modal: ResMut<ModalOpen>,
    mut composer: ResMut<Composer>,
    mut connection_manager: ResMut<ConnectionManager>,
) {
    let input = modal_keys(&modal, &captured, &keys);
    let auto_at = *auto.get_or_insert_with(|| {
        std::env::var("MARVYR_AUTOBOTTLE")
            .ok()
            .and_then(|raw| raw.parse::<f32>().ok())
    });
    let now = time.elapsed_secs();
    let auto_open = auto_at.is_some_and(|at| now >= at);
    let auto_throw = auto_at.is_some_and(|at| now >= at + 2.0);
    if auto_throw && composer.open {
        *auto = Some(None);
        composer.words = [0, 1, 4];
        let _ = connection_manager.send_message::<ReliableChannel, _>(&ThrowBottle {
            words: composer.words,
        });
        composer.open = false;
        modal.bottle = false;
        return;
    }
    if !composer.open {
        if !docked.0 && !modal.any() && (keys.just_pressed(KeyCode::F3) || auto_open) {
            composer.open = true;
            modal.bottle = true;
        }
        return;
    }
    let sizes = [OPENINGS.len(), SUBJECTS.len(), PLACES.len()];
    if input.just_pressed(KeyCode::Escape) || input.just_pressed(KeyCode::F3) || docked.0 {
        composer.open = false;
        modal.bottle = false;
        return;
    }
    if input.just_pressed(KeyCode::ArrowUp) {
        composer.row = (composer.row + 2) % 3;
    }
    if input.just_pressed(KeyCode::ArrowDown) {
        composer.row = (composer.row + 1) % 3;
    }
    let row = composer.row;
    let size = sizes[row] as u8;
    if input.just_pressed(KeyCode::ArrowRight) {
        composer.words[row] = (composer.words[row] + 1) % size;
    }
    if input.just_pressed(KeyCode::ArrowLeft) {
        composer.words[row] = (composer.words[row] + size - 1) % size;
    }
    if input.just_pressed(KeyCode::Enter) {
        let _ = connection_manager.send_message::<ReliableChannel, _>(&ThrowBottle {
            words: composer.words,
        });
        composer.open = false;
        modal.bottle = false;
    }
}

fn setup_composer(mut commands: Commands) {
    commands
        .spawn((
            ComposerRoot,
            Node {
                position_type: PositionType::Absolute,
                width: Val::Percent(100.0),
                top: Val::Percent(30.0),
                justify_content: JustifyContent::Center,
                display: Display::None,
                ..default()
            },
            GlobalZIndex(25),
        ))
        .with_children(|root| {
            root.spawn(ui::panel(Node {
                padding: UiRect::all(Val::Px(14.0)),
                flex_direction: FlexDirection::Column,
                row_gap: Val::Px(6.0),
                min_width: Val::Px(420.0),
                ..default()
            }))
            .with_children(|panel| {
                panel.spawn(ui::display("Mensagem na garrafa", 22.0, ui::INK));
                panel.spawn((ui::text("", 15.0, ui::INK), ComposerText));
                panel.spawn(ui::text(
                    "Setas escolhem · Enter joga ao mar · Esc fecha",
                    12.0,
                    ui::INK_SOFT,
                ));
            });
        });
}

fn show_composer(
    composer: Res<Composer>,
    mut root: Query<&mut Node, With<ComposerRoot>>,
    mut text: Query<&mut Text, With<ComposerText>>,
) {
    if !composer.is_changed() {
        return;
    }
    let display = if composer.open {
        Display::Flex
    } else {
        Display::None
    };
    for mut node in &mut root {
        node.display = display;
    }
    let lists: [&[&str]; 3] = [&OPENINGS, &SUBJECTS, &PLACES];
    let lines: Vec<String> = lists
        .iter()
        .enumerate()
        .map(|(row, list)| {
            let word = tr(list[usize::from(composer.words[row])]);
            if row == composer.row {
                format!("‹ {word} ›")
            } else {
                format!("  {word}")
            }
        })
        .collect();
    let preview = sentence(composer.words);
    for mut text in &mut text {
        text.0 = format!("{}\n\n{}", lines.join("\n"), preview);
    }
}

fn send_pick(
    keys: Res<ButtonInput<KeyCode>>,
    docked: Res<MyDocked>,
    context: Res<crate::hud::PromptContext>,
    mut connection_manager: ResMut<ConnectionManager>,
) {
    if docked.0
        || context.0 != crate::hud::HudContext::NearBottle
        || !keys.just_pressed(KeyCode::KeyB)
    {
        return;
    }
    let _ = connection_manager.send_message::<ReliableChannel, _>(&PickBottle);
}

/// A frase pescada sobe num cartão de papel sobre o navio e some devagar.
fn show_read(
    mut commands: Commands,
    mut reads: EventReader<ClientReceiveMessage<BottleRead>>,
    my_ship: Res<MyShip>,
    visuals: Query<&ShipVisual>,
) {
    for read in reads.read() {
        let read = read.message();
        let Some(at) = crate::logbook::my_position(&my_ship, &visuals) else {
            continue;
        };
        commands.spawn((
            ReadCard { age: 0.0 },
            Text2d::new(format!(
                "\"{}\"\n{}",
                sentence(read.words),
                trf("— {0}", &[&tr(&read.author)])
            )),
            TextFont {
                font_size: 18.0,
                ..default()
            },
            TextColor(Color::srgb(1.0, 0.96, 0.85)),
            Transform::from_translation((at + Vec2::new(0.0, 60.0)).extend(layers::LABELS)),
        ));
    }
}

fn fade_cards(
    mut commands: Commands,
    time: Res<Time>,
    mut cards: Query<(Entity, &mut ReadCard, &mut TextColor)>,
) {
    const LIFE: f32 = 8.0;
    for (entity, mut card, mut color) in &mut cards {
        card.age += time.delta_secs();
        if card.age >= LIFE {
            commands.entity(entity).despawn();
            continue;
        }
        color.0.set_alpha((LIFE - card.age).min(1.0));
    }
}

/// Garrafa ao mar: um respingo e o letreiro no casco.
fn splash_on_throw(
    mut commands: Commands,
    mut results: EventReader<ClientReceiveMessage<ActionResult>>,
    my_ship: Res<MyShip>,
    visuals: Query<&ShipVisual>,
) {
    for result in results.read() {
        let result = result.message();
        if result.action != ActionKind::Bottle || !result.success {
            continue;
        }
        let Some(at) = crate::logbook::my_position(&my_ship, &visuals) else {
            continue;
        };
        crate::juice::spawn_float_text(
            &mut commands,
            at + Vec2::new(0.0, 18.0),
            tr(&result.reason),
            GLASS,
        );
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_sentence_has_three_pieces_and_bad_words_are_empty() {
        assert_eq!(sentence([0, 1, 4]), "Cuidado com o Kraken aqui perto.");
        assert_eq!(sentence([99, 0, 0]), "");
    }

    #[test]
    fn only_bottles_in_reach_count() {
        let known = KnownBottles(vec![(1, Vec2::new(50.0, 0.0)), (2, Vec2::new(500.0, 0.0))]);
        assert_eq!(bottle_near(&known, Vec2::ZERO), Some(Vec2::new(50.0, 0.0)));
        assert_eq!(bottle_near(&known, Vec2::new(1_000.0, 0.0)), None);
    }
}
