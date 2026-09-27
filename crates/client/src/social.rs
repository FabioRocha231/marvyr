//! Quem é quem e party no client (v55). Cada navio ganha uma placa embaixo
//! do casco: nome do capitão (ou o tipo do NPC) e a barra de casco, na cor
//! da relação — party verde, pirata e monstro vermelho, marinha azul,
//! mercador dourado, outros capitães em papel. V convida o capitão sob o
//! cursor (ou o mais perto) e aceita convite; Shift+V recusa ou sai.

use std::collections::HashMap;

use bevy::prelude::*;
use lightyear::prelude::client::*;
use lightyear::prelude::*;
use marvyr_protocol::{
    Faction, PartyAnswer, PartyInvite, PartyLeave, PartyUpdate, ShipNames, ShipState,
};

use crate::assets::layers;
use crate::net::{MyDocked, MyShip, ReliableChannel};
use crate::ship::ShipVisual;
use crate::ui;

/// Convite por tecla: capitão mais perto até esta distância (m) quando o
/// cursor não está em cima de ninguém.
const INVITE_REACH: f32 = 400.0;
/// Cursor "em cima" de um navio: raio em metros.
const HOVER_RADIUS: f32 = 30.0;
const PLATE_GAP: f32 = 10.0;
const BAR_WIDTH: f32 = 34.0;

const PARTY_GREEN: Color = Color::srgb(0.45, 0.95, 0.55);
const HOSTILE_RED: Color = Color::srgb(1.0, 0.42, 0.36);
const NAVY_BLUE: Color = Color::srgb(0.55, 0.75, 1.0);
const MERCHANT_GOLD: Color = Color::srgb(1.0, 0.85, 0.45);
const CAPTAIN_PAPER: Color = Color::srgb(0.95, 0.92, 0.84);

pub struct SocialPlugin;

impl Plugin for SocialPlugin {
    fn build(&self, app: &mut App) {
        app.init_resource::<KnownNames>()
            .init_resource::<KnownParty>()
            .add_systems(Startup, spawn_party_panel)
            .add_systems(
                Update,
                (
                    receive_names,
                    receive_party,
                    send_party_input,
                    update_nameplates.after(crate::ship::lerp_ship_visuals),
                    update_party_panel,
                ),
            );
    }
}

/// Nome do capitão por navio (`ShipNames`).
#[derive(Resource, Debug, Default)]
pub struct KnownNames(pub HashMap<u32, String>);

/// A minha party (sem eu) e o convite pendente.
#[derive(Resource, Debug, Default)]
pub struct KnownParty(pub PartyUpdate);

impl KnownParty {
    pub fn has(&self, ship_id: u32) -> bool {
        self.0
            .members
            .iter()
            .any(|member| member.ship_id == ship_id)
    }
}

fn receive_names(
    mut events: EventReader<ClientReceiveMessage<ShipNames>>,
    mut names: ResMut<KnownNames>,
) {
    if let Some(event) = events.read().last() {
        names.0 = event.message().names.iter().cloned().collect();
    }
}

fn receive_party(
    mut events: EventReader<ClientReceiveMessage<PartyUpdate>>,
    mut party: ResMut<KnownParty>,
) {
    if let Some(event) = events.read().last() {
        party.0 = event.message().clone();
    }
}

/// Nome e cor da placa de um navio.
/// Aliado: companheiro de party ou de companhia (a tag `tag` no nome).
pub fn is_ally(ship_id: u32, names: &KnownNames, party: &KnownParty, tag: &str) -> bool {
    party.has(ship_id)
        || (!tag.is_empty()
            && names
                .0
                .get(&ship_id)
                .is_some_and(|name| name.starts_with(&format!("[{tag}] "))))
}

/// `tag` é a da minha companhia (v62): quem a traz no nome é aliado.
pub fn plate_of(
    state: &ShipState,
    names: &KnownNames,
    party: &KnownParty,
    tag: &str,
) -> (String, Color) {
    if let Some(kind) = marvyr_protocol::npc_kind_name(state.npc_kind) {
        let color = match state.faction {
            Faction::Navy => NAVY_BLUE,
            Faction::Merchant => MERCHANT_GOLD,
            _ => HOSTILE_RED,
        };
        return (crate::i18n::tr(kind), color);
    }
    let name = names
        .0
        .get(&state.ship_id)
        .cloned()
        .unwrap_or_else(|| crate::i18n::tr("Capitão"));
    let color = if is_ally(state.ship_id, names, party, tag) {
        PARTY_GREEN
    } else if state.notoriety_tier >= 2 || state.black_flag == marvyr_protocol::FLAG_RAISED {
        HOSTILE_RED
    } else {
        CAPTAIN_PAPER
    };
    (name, color)
}

/// Placa embaixo do casco (entidade própria: não gira com o navio).
#[derive(Component)]
struct Nameplate {
    ship_id: u32,
    label: String,
    color: Color,
}

#[derive(Component)]
struct PlateFill;

#[allow(clippy::type_complexity, clippy::too_many_arguments)]
fn update_nameplates(
    mut commands: Commands,
    names: Res<KnownNames>,
    party: Res<KnownParty>,
    company: Res<crate::company::KnownCompany>,
    my_ship: Res<MyShip>,
    ships: Query<(&ShipVisual, &Transform), Without<Nameplate>>,
    mut plates: Query<(Entity, &Nameplate, &mut Transform, &Children), Without<ShipVisual>>,
    mut fills: Query<
        (&mut Sprite, &mut Transform),
        (With<PlateFill>, Without<Nameplate>, Without<ShipVisual>),
    >,
) {
    let below = |visual: &ShipVisual, ship: &Transform| {
        Vec3::new(
            ship.translation.x,
            ship.translation.y - crate::ship::hull_length(visual.target.kind) * 0.5 - PLATE_GAP,
            layers::LABELS,
        )
    };
    let mut placed = std::collections::HashSet::new();
    for (entity, plate, mut transform, children) in &mut plates {
        let found = ships
            .iter()
            .find(|(visual, _)| visual.target.ship_id == plate.ship_id);
        let Some((visual, ship)) = found else {
            commands.entity(entity).despawn_recursive();
            continue;
        };
        let (label, color) = plate_of(&visual.target, &names, &party, &company.0.tag);
        if label != plate.label || color != plate.color {
            commands.entity(entity).despawn_recursive();
            continue;
        }
        transform.translation = below(visual, ship);
        let fraction = hp_fraction(&visual.target);
        for child in children.iter() {
            if let Ok((mut sprite, mut fill)) = fills.get_mut(*child) {
                sprite.custom_size = Some(Vec2::new(BAR_WIDTH * fraction, 2.5));
                fill.translation.x = -BAR_WIDTH * (1.0 - fraction) * 0.5;
            }
        }
        placed.insert(plate.ship_id);
    }
    for (visual, ship) in &ships {
        let state = &visual.target;
        if Some(state.ship_id) == my_ship.0 || placed.contains(&state.ship_id) {
            continue;
        }
        let (label, color) = plate_of(state, &names, &party, &company.0.tag);
        let fraction = hp_fraction(state);
        commands
            .spawn((
                Nameplate {
                    ship_id: state.ship_id,
                    label: label.clone(),
                    color,
                },
                Transform::from_translation(below(visual, ship)),
                Visibility::default(),
            ))
            .with_children(|plate| {
                plate.spawn((
                    Text2d::new(label),
                    TextFont {
                        font_size: 11.0,
                        ..default()
                    },
                    TextColor(color),
                    Transform::from_xyz(0.0, 0.0, 0.1),
                ));
                plate.spawn((
                    Sprite::from_color(
                        Color::srgba(0.0, 0.0, 0.0, 0.55),
                        Vec2::new(BAR_WIDTH, 2.5),
                    ),
                    Transform::from_xyz(0.0, -9.0, 0.05),
                ));
                plate.spawn((
                    Sprite::from_color(color, Vec2::new(BAR_WIDTH * fraction, 2.5)),
                    Transform::from_xyz(-BAR_WIDTH * (1.0 - fraction) * 0.5, -9.0, 0.08),
                    PlateFill,
                ));
            });
    }
}

fn hp_fraction(state: &ShipState) -> f32 {
    if state.max_hp == 0 {
        return 0.0;
    }
    (state.hp as f32 / state.max_hp as f32).clamp(0.0, 1.0)
}

/// V convida (ou aceita o convite pendente); Shift+V recusa ou sai.
#[allow(clippy::too_many_arguments)]
fn send_party_input(
    keys: Res<ButtonInput<KeyCode>>,
    docked: Res<MyDocked>,
    aim: Res<crate::combat::AimPoint>,
    my_ship: Res<MyShip>,
    party: Res<KnownParty>,
    visuals: Query<&ShipVisual>,
    time: Res<Time>,
    mut auto_clock: Local<f32>,
    mut connection_manager: ResMut<ConnectionManager>,
) {
    // Dev (teste ao vivo sem teclado): MARVYR_AUTOPARTY=1 aperta V a cada
    // 3 s enquanto não estiver numa party (convida ou aceita).
    let mut auto = false;
    if std::env::var_os("MARVYR_AUTOPARTY").is_some() && party.0.members.is_empty() {
        *auto_clock += time.delta_secs();
        if *auto_clock >= 3.0 {
            *auto_clock = 0.0;
            auto = true;
        }
    }
    if docked.0 || !(auto || keys.just_pressed(KeyCode::KeyV)) {
        return;
    }
    let shift = keys.pressed(KeyCode::ShiftLeft) || keys.pressed(KeyCode::ShiftRight);
    let pending = party.0.invite_from.is_some();
    if shift {
        if pending {
            let _ = connection_manager
                .send_message::<ReliableChannel, _>(&PartyAnswer { accept: false });
        } else {
            let _ = connection_manager.send_message::<ReliableChannel, _>(&PartyLeave);
        }
        return;
    }
    if pending {
        let _ =
            connection_manager.send_message::<ReliableChannel, _>(&PartyAnswer { accept: true });
        return;
    }
    let Some(me) = my_ship
        .0
        .and_then(|id| visuals.iter().find(|visual| visual.target.ship_id == id))
    else {
        return;
    };
    let mine = Vec2::new(me.target.x, me.target.y);
    let captains = || {
        visuals
            .iter()
            .map(|visual| &visual.target)
            .filter(|state| state.npc_kind == 0 && state.ship_id != me.target.ship_id)
    };
    let hovered = aim.0.and_then(|point| {
        captains().find(|state| Vec2::new(state.x, state.y).distance(point) <= HOVER_RADIUS)
    });
    let target = hovered.or_else(|| {
        captains()
            .filter(|state| Vec2::new(state.x, state.y).distance(mine) <= INVITE_REACH)
            .min_by(|a, b| {
                let da = Vec2::new(a.x, a.y).distance(mine);
                let db = Vec2::new(b.x, b.y).distance(mine);
                da.total_cmp(&db)
            })
    });
    if let Some(target) = target {
        let _ = connection_manager.send_message::<ReliableChannel, _>(&PartyInvite {
            target_ship_id: target.ship_id,
        });
    }
}

/// Painel da party (esquerda, abaixo do bilhete do navio).
#[derive(Component)]
struct PartyPanel;

fn spawn_party_panel(mut commands: Commands) {
    commands.spawn((
        ui::panel(Node {
            position_type: PositionType::Absolute,
            left: Val::Px(ui::MARGIN),
            top: Val::Px(190.0),
            flex_direction: FlexDirection::Column,
            row_gap: Val::Px(4.0),
            padding: UiRect::axes(Val::Px(10.0), Val::Px(6.0)),
            display: Display::None,
            ..default()
        }),
        crate::hud::SeaHud,
        PartyPanel,
    ));
}

/// Remonta o painel quando a party muda (poucos membros: barato).
fn update_party_panel(
    mut commands: Commands,
    party: Res<KnownParty>,
    docked: Res<MyDocked>,
    lang: Res<crate::i18n::Lang>,
    mut panels: Query<(Entity, &mut Node), With<PartyPanel>>,
) {
    if !(party.is_changed() || docked.is_changed() || lang.is_changed()) {
        return;
    }
    let show = !docked.0 && (!party.0.members.is_empty() || party.0.invite_from.is_some());
    for (entity, mut node) in &mut panels {
        node.display = if show { Display::Flex } else { Display::None };
        commands.entity(entity).despawn_descendants();
        if !show {
            continue;
        }
        commands.entity(entity).with_children(|panel| {
            if !party.0.members.is_empty() {
                panel.spawn(crate::i18n::label_face(
                    "PARTY",
                    ui::FONT_BOLD,
                    12.0,
                    ui::INK_SOFT,
                ));
            }
            for member in &party.0.members {
                let fraction = if member.max_hp == 0 {
                    0.0
                } else {
                    (member.hp as f32 / member.max_hp as f32).clamp(0.0, 1.0)
                };
                panel.spawn(ui::face(member.name.clone(), ui::FONT_BOLD, 13.0, ui::INK));
                panel
                    .spawn((
                        Node {
                            width: Val::Px(140.0),
                            height: Val::Px(5.0),
                            ..default()
                        },
                        BackgroundColor(ui::INK.with_alpha(0.15)),
                    ))
                    .with_children(|bar| {
                        bar.spawn((
                            Node {
                                width: Val::Percent(fraction * 100.0),
                                height: Val::Percent(100.0),
                                ..default()
                            },
                            BackgroundColor(if fraction > 0.35 {
                                ui::OK_GREEN
                            } else {
                                ui::DANGER
                            }),
                        ));
                    });
            }
            if let Some(from) = &party.0.invite_from {
                panel.spawn(ui::face(
                    crate::i18n::trf("Convite de {0}", &[from]),
                    ui::FONT_BOLD,
                    13.0,
                    ui::AMBER,
                ));
                panel.spawn(ui::text(
                    crate::i18n::tr("V aceita  ·  Shift+V recusa"),
                    12.0,
                    ui::INK_SOFT,
                ));
            }
        });
    }
}
