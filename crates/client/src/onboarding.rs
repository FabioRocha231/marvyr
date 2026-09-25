//! Primeira viagem (MV-062): boas-vindas, guia de seis passos e dicas de
//! primeira vez. Ensina navegando — sem modo tutorial separado e sem
//! corrente de missões (a visão diz "não é theme park"): cada passo é algo
//! real do loop (içar, coletar, atracar, vender/guardar, fabricar, levar a
//! outro porto) e se completa quando o jogador de fato faz.
//!
//! O guia vive num bilhete no canto inferior esquerdo, com uma linha-guia
//! de tinta vermelha no mar até o alvo do passo. O progresso fica na pasta
//! de dados do jogador; quem pula o guia não vê de novo (F1 > Opções volta).

use bevy::prelude::*;
use lightyear::prelude::ClientReceiveMessage;
use marvyr_protocol::{CraftResult, GatherResult, ShipState};
use serde::{Deserialize, Serialize};

use crate::help::RestartGuide;
use crate::i18n::{self, Lang};
use crate::input::{glyph, InputDevice, ModalKeys, ModalOpen};
use crate::net::{MyDocked, MyShip, SailLevel};
use crate::nodes::KnownNodes;
use crate::port_screen::DockedPortName;
use crate::session::ConnectionStatus;
use crate::ship::ShipVisual;
use crate::ui;

/// Passos do guia, na ordem do loop econômico.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Step {
    SetSail = 1,
    Gather = 2,
    Dock = 3,
    SellOrStore = 4,
    Craft = 5,
    Voyage = 6,
}

pub const STEPS: u8 = 6;

impl Step {
    fn from_index(index: u8) -> Option<Self> {
        Some(match index {
            1 => Step::SetSail,
            2 => Step::Gather,
            3 => Step::Dock,
            4 => Step::SellOrStore,
            5 => Step::Craft,
            6 => Step::Voyage,
            _ => return None,
        })
    }

    /// Instrução curta e a tecla que resolve (quando há uma).
    fn instruction(self, docked: bool) -> (&'static str, Option<KeyCode>) {
        match self {
            // Atracado, W não faz nada: primeiro sai da tela de porto.
            Step::SetSail if docked => ("Desatraque para zarpar", Some(KeyCode::Escape)),
            Step::SetSail => ("Içe a vela para sair do porto", Some(KeyCode::KeyW)),
            Step::Gather => ("Colete um recurso no mar", Some(KeyCode::KeyG)),
            Step::Dock => ("Volte a um porto e atraque", Some(KeyCode::KeyE)),
            Step::SellOrStore => ("Guarde ou troque a carga no porto", Some(KeyCode::Enter)),
            Step::Craft => ("Fabrique algo no porto", Some(KeyCode::Enter)),
            Step::Voyage => ("Leve carga até outro porto", Some(KeyCode::KeyE)),
        }
    }
}

/// Progresso salvo (por máquina) em `onboarding.json`.
#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq)]
struct Progress {
    welcomed: bool,
    skipped: bool,
    /// Próximo passo a cumprir (1..=6); 7 = formado.
    step: u8,
    tips: Vec<String>,
    first_port: Option<String>,
}

impl Progress {
    fn guiding(&self) -> bool {
        self.welcomed && !self.skipped && (1..=STEPS).contains(&self.step)
    }
}

fn progress_path() -> std::path::PathBuf {
    crate::config::data_dir().join("onboarding.json")
}

fn load_progress() -> Progress {
    std::fs::read_to_string(progress_path())
        .ok()
        .and_then(|raw| serde_json::from_str(&raw).ok())
        .unwrap_or_default()
}

fn save_progress(progress: &Progress) {
    let path = progress_path();
    if let Some(dir) = path.parent() {
        let _ = std::fs::create_dir_all(dir);
    }
    if let Ok(json) = serde_json::to_string_pretty(progress) {
        let _ = std::fs::write(path, json);
    }
}

#[derive(Resource, Debug, Default)]
pub struct Onboarding {
    progress: Progress,
    welcome_open: bool,
    /// Carimbo "FEITO" do passo recém-cumprido (até o instante indicado).
    done_until: f32,
    /// Formatura na tela até o instante indicado.
    graduation_until: f32,
    /// Referência do passo 4: carga ao atracar.
    baseline: Option<u32>,
    /// Dica de primeira vez na tela e até quando.
    tip: Option<(&'static Tip, f32)>,
}

impl Onboarding {
    pub fn guiding(&self) -> bool {
        self.progress.guiding()
    }
}

/// Marco alcançado (0 = boas-vindas vistas). O envio para o servidor
/// (métrica de onboarding) fica em `net`.
#[derive(Event, Debug, Clone, Copy, PartialEq, Eq)]
pub struct OnboardingReached {
    pub step: u8,
    pub skipped: bool,
}

pub struct OnboardingPlugin;

impl Plugin for OnboardingPlugin {
    fn build(&self, app: &mut App) {
        app.insert_resource(Onboarding {
            progress: load_progress(),
            ..default()
        })
        .add_event::<OnboardingReached>()
        .add_systems(Startup, spawn_onboarding_ui)
        .add_systems(
            Update,
            (
                open_welcome,
                welcome_input,
                restart_guide,
                advance_guide,
                first_time_tips,
                (
                    draw_welcome,
                    draw_guide,
                    draw_tip,
                    draw_graduation,
                    draw_leader_line,
                ),
            )
                .chain(),
        )
        .add_systems(Update, report_progress);
    }
}

// ── Boas-vindas ────────────────────────────────────────────────────────

#[derive(Component)]
struct WelcomeRoot;
#[derive(Component)]
enum WelcomeButton {
    Start,
    Skip,
}
#[derive(Component)]
struct WelcomeKey(KeyCode);

fn my_state<'a>(my_ship: &MyShip, visuals: &'a Query<&ShipVisual>) -> Option<&'a ShipState> {
    let id = my_ship.0?;
    visuals
        .iter()
        .find(|visual| visual.target.ship_id == id)
        .map(|visual| &visual.target)
}

fn open_welcome(
    status: Res<ConnectionStatus>,
    my_ship: Res<MyShip>,
    mut onboarding: ResMut<Onboarding>,
    mut modal: ResMut<ModalOpen>,
    mut reached: EventWriter<OnboardingReached>,
) {
    let ready = *status == ConnectionStatus::InGame && my_ship.0.is_some();
    if ready && !onboarding.progress.welcomed && !onboarding.welcome_open {
        onboarding.welcome_open = true;
        reached.send(OnboardingReached {
            step: 0,
            skipped: false,
        });
    }
    let open = onboarding.welcome_open && ready;
    if modal.welcome != open {
        modal.welcome = open;
    }
}

fn welcome_input(
    raw_keys: Res<ButtonInput<KeyCode>>,
    modal: Res<ModalOpen>,
    captured: Res<ModalKeys>,
    buttons: Query<(&Interaction, &WelcomeButton), Changed<Interaction>>,
    mut onboarding: ResMut<Onboarding>,
    mut reached: EventWriter<OnboardingReached>,
) {
    if !onboarding.welcome_open {
        return;
    }
    let keys = crate::input::modal_keys(&modal, &captured, &raw_keys);
    let mut start = keys.just_pressed(KeyCode::Enter);
    let mut skip = keys.just_pressed(KeyCode::Escape);
    for (interaction, button) in &buttons {
        if *interaction == Interaction::Pressed {
            match button {
                WelcomeButton::Start => start = true,
                WelcomeButton::Skip => skip = true,
            }
        }
    }
    if !(start || skip) {
        return;
    }
    let progress = &mut onboarding.progress;
    progress.welcomed = true;
    progress.skipped = skip;
    progress.step = if skip { STEPS + 1 } else { 1 };
    onboarding.welcome_open = false;
    save_progress(&onboarding.progress);
    if skip {
        reached.send(OnboardingReached {
            step: 0,
            skipped: true,
        });
    }
}

fn restart_guide(mut requests: EventReader<RestartGuide>, mut onboarding: ResMut<Onboarding>) {
    if requests.read().last().is_some() {
        let tips = std::mem::take(&mut onboarding.progress.tips);
        onboarding.progress = Progress {
            tips,
            ..Progress::default()
        };
        onboarding.baseline = None;
        save_progress(&onboarding.progress);
    }
}

// ── Guia ───────────────────────────────────────────────────────────────

#[allow(clippy::too_many_arguments)]
fn advance_guide(
    time: Res<Time>,
    sail: Res<SailLevel>,
    docked: Res<MyDocked>,
    port_name: Res<DockedPortName>,
    my_ship: Res<MyShip>,
    visuals: Query<&ShipVisual>,
    mut gathers: EventReader<ClientReceiveMessage<GatherResult>>,
    mut crafts: EventReader<ClientReceiveMessage<CraftResult>>,
    mut onboarding: ResMut<Onboarding>,
    mut reached: EventWriter<OnboardingReached>,
) {
    let gathered = gathers.read().any(|event| event.message().success);
    let crafted = crafts.read().any(|event| event.message().success);
    if !onboarding.guiding() {
        return;
    }
    let Some(step) = Step::from_index(onboarding.progress.step) else {
        return;
    };
    let cargo = my_state(&my_ship, &visuals).map_or(0, |state| state.cargo_weight);
    let done = match step {
        Step::SetSail => sail.0 > 0,
        Step::Gather => gathered,
        Step::Dock => docked.0,
        Step::SellOrStore => {
            // Referência tirada já atracado: o snapshot do armazém que chega
            // ao atracar não conta como "guardou". Vale porão que esvazia
            // (depositou ou trocou).
            if !docked.0 {
                onboarding.baseline = None;
                false
            } else {
                cargo < *onboarding.baseline.get_or_insert(cargo)
            }
        }
        Step::Craft => crafted,
        Step::Voyage => {
            docked.0
                && !port_name.0.is_empty()
                && onboarding
                    .progress
                    .first_port
                    .as_ref()
                    .is_some_and(|first| *first != port_name.0)
        }
    };
    if step == Step::Dock && docked.0 && !port_name.0.is_empty() {
        onboarding.progress.first_port = Some(port_name.0.clone());
    }
    if !done {
        return;
    }
    let now = time.elapsed_secs();
    onboarding.progress.step += 1;
    onboarding.baseline = None;
    onboarding.done_until = now + 1.6;
    if onboarding.progress.step > STEPS {
        onboarding.graduation_until = now + 9.0;
    }
    save_progress(&onboarding.progress);
    reached.send(OnboardingReached {
        step: step as u8,
        skipped: false,
    });
}

/// Métrica de onboarding para o servidor (só telemetria: não concede nada).
fn report_progress(
    mut reached: EventReader<OnboardingReached>,
    connection: Option<ResMut<lightyear::prelude::client::ConnectionManager>>,
) {
    let Some(mut connection) = connection else {
        reached.clear();
        return;
    };
    for event in reached.read() {
        crate::net::send_onboarding(&mut connection, event.step, event.skipped);
    }
}

// ── Dicas de primeira vez ──────────────────────────────────────────────

/// Explicação curta do que o jogador acabou de ver pela primeira vez.
#[derive(Debug)]
pub struct Tip {
    id: &'static str,
    title: &'static str,
    body: &'static str,
}

const TIPS: &[Tip] = &[
    Tip {
        id: "frontier",
        title: "Fronteira",
        body: "Daqui em diante outros capitães podem atacar você. Carga valiosa pede escolta ou pressa.",
    },
    Tip {
        id: "lawless",
        title: "Águas sem lei",
        body: "Combate e saque total: afundou, a carga vira destroço de quem pegar. O recurso raro está aqui — e o risco também.",
    },
    Tip {
        id: "portal",
        title: "Cerração e Sorvedouro",
        body: "Cerração é névoa com tempo e vagas contados que leva a uma arena. Sorvedouro é um redemoinho que corta caminho por águas sem lei.",
    },
    Tip {
        id: "event",
        title: "Evento de mar",
        body: "Tormenta, frota do tesouro, kraken ou maré disputada. O anúncio diz onde; o livreto (F1) conta o que cada um faz.",
    },
    Tip {
        id: "treasure",
        title: "Mapa do tesouro",
        body: "Siga o X marcado no mar, pare o navio em cima e cave.",
    },
    Tip {
        id: "hull",
        title: "Casco avariado",
        body: "Parado e fora de combate, repare no mar gastando madeira do porão.",
    },
    Tip {
        id: "full",
        title: "Porão cheio",
        body: "Atraque para guardar no armazém do porto ou trocar no mercado.",
    },
];

fn tip(id: &str) -> &'static Tip {
    TIPS.iter().find(|tip| tip.id == id).unwrap_or(&TIPS[0])
}

#[allow(clippy::too_many_arguments)]
fn first_time_tips(
    time: Res<Time>,
    status: Res<ConnectionStatus>,
    docked: Res<MyDocked>,
    zone: Res<crate::zone::CurrentZone>,
    portals: Res<crate::portals::KnownPortals>,
    events: Res<crate::seafaring::SeaEvents>,
    marks: Res<crate::seafaring::TreasureMarks>,
    my_ship: Res<MyShip>,
    visuals: Query<&ShipVisual>,
    mut onboarding: ResMut<Onboarding>,
) {
    let now = time.elapsed_secs();
    if onboarding.tip.is_some_and(|(_, until)| until > now) {
        return;
    }
    onboarding.tip = None;
    if *status != ConnectionStatus::InGame || docked.0 || onboarding.welcome_open {
        return;
    }
    let Some(state) = my_state(&my_ship, &visuals) else {
        return;
    };
    let me = Vec2::new(state.x, state.y);
    let tier = zone.0.as_ref().map(|zone| zone.tier);
    let candidates = [
        (
            "frontier",
            tier == Some(marvyr_domain_world::RiskTier::Frontier),
        ),
        (
            "lawless",
            tier == Some(marvyr_domain_world::RiskTier::Lawless),
        ),
        (
            "portal",
            portals
                .portals
                .values()
                .any(|portal| me.distance(Vec2::new(portal.x, portal.y)) < 700.0),
        ),
        ("event", !events.0.is_empty()),
        ("treasure", !marks.0.is_empty()),
        ("hull", state.max_hp > 0 && state.hp * 2 < state.max_hp),
        (
            "full",
            state.cargo_capacity > 0 && state.cargo_weight >= state.cargo_capacity,
        ),
    ];
    let seen = &onboarding.progress.tips;
    let Some((id, _)) = candidates
        .iter()
        .find(|(id, active)| *active && !seen.iter().any(|done| done == id))
    else {
        return;
    };
    onboarding.progress.tips.push((*id).to_owned());
    onboarding.tip = Some((tip(id), now + 9.0));
    save_progress(&onboarding.progress);
}

// ── Desenho ────────────────────────────────────────────────────────────

#[derive(Component)]
struct GuideSlip;
#[derive(Component)]
struct GuidePaper;
#[derive(Component)]
struct GuideHeader;
#[derive(Component)]
struct GuideLine;
#[derive(Component)]
struct GuideHint;
#[derive(Component)]
struct GuideKey;
#[derive(Component)]
struct GuideDone;
#[derive(Component)]
struct TipSlip;
#[derive(Component)]
struct TipTitle;
#[derive(Component)]
struct TipBody;
#[derive(Component)]
struct GraduationRoot;

fn spawn_onboarding_ui(mut commands: Commands) {
    // Boas-vindas: um cartaz de porto no centro, sobre o mar escurecido.
    commands
        .spawn((
            Node {
                position_type: PositionType::Absolute,
                width: Val::Percent(100.0),
                height: Val::Percent(100.0),
                justify_content: JustifyContent::Center,
                align_items: AlignItems::Center,
                display: Display::None,
                ..default()
            },
            BackgroundColor(ui::INK.with_alpha(0.5)),
            GlobalZIndex(45),
            WelcomeRoot,
        ))
        .with_children(|scrim| {
            scrim
                .spawn(ui::panel(Node {
                    width: Val::Px(620.0),
                    max_width: Val::Percent(92.0),
                    flex_direction: FlexDirection::Column,
                    align_items: AlignItems::Center,
                    padding: UiRect::axes(Val::Px(36.0), Val::Px(26.0)),
                    row_gap: Val::Px(14.0),
                    ..default()
                }))
                .with_children(|poster| {
                    ui::masthead(poster, "BEM-VINDO A BORDO", 40.0);
                    ui::double_rule(poster);
                    for line in [
                        "Você é mercador. Tudo que vale algo neste mar foi fabricado por um jogador.",
                        "Riqueza só vale onde ela chega: carregue o porão e leve até o porto que paga mais.",
                        "No caminho, o mar cobra. Em águas sem lei, quem afunda perde a carga.",
                    ] {
                        poster.spawn((i18n::label_face(line, ui::FONT_REGULAR, 18.0, ui::INK),
                            TextLayout::new_with_justify(JustifyText::Center),
                        ));
                    }
                    ui::rule(poster);
                    poster.spawn(i18n::label(
                        "O guia da primeira viagem tem seis passos, uns dez minutos.",
                        15.0,
                        ui::INK_SOFT,
                    ));
                    poster
                        .spawn(Node {
                            column_gap: Val::Px(16.0),
                            flex_wrap: FlexWrap::Wrap,
                            justify_content: JustifyContent::Center,
                            row_gap: Val::Px(10.0),
                            ..default()
                        })
                        .with_children(|row| {
                            welcome_button(row, KeyCode::Enter, "Zarpar com o guia", WelcomeButton::Start, true);
                            welcome_button(row, KeyCode::Escape, "Já sei navegar", WelcomeButton::Skip, false);
                        });
                    poster.spawn((
                        WelcomeHelpLine,
                        i18n::label(HELP_LINE_KEYBOARD, 13.0, ui::INK_SOFT),
                    ));
                });
        });

    // Bilhete do guia: canto inferior esquerdo no mar; atracado, vira uma
    // tira compacta no rodapé, centralizada, sem cobrir a tela de porto.
    commands
        .spawn((
            Node {
                position_type: PositionType::Absolute,
                left: Val::Px(ui::MARGIN),
                right: Val::Px(ui::MARGIN),
                bottom: Val::Px(ui::MARGIN),
                display: Display::None,
                ..default()
            },
            GlobalZIndex(20),
            GuideSlip,
        ))
        .with_children(|row| {
            row.spawn((
                ui::panel(Node {
                    width: Val::Px(360.0),
                    flex_direction: FlexDirection::Column,
                    row_gap: Val::Px(6.0),
                    ..default()
                }),
                GuidePaper,
            ))
            .with_children(|slip| {
                slip.spawn(Node {
                    justify_content: JustifyContent::SpaceBetween,
                    align_items: AlignItems::Center,
                    column_gap: Val::Px(10.0),
                    ..default()
                })
                .with_children(|head| {
                    head.spawn((
                        ui::face("", ui::FONT_BOLD, 13.0, ui::VERMILION_INK),
                        GuideHeader,
                    ));
                    head.spawn((ui::stamp("FEITO", ui::TEAL), GuideDone, Visibility::Hidden));
                });
                slip.spawn(Node {
                    column_gap: Val::Px(10.0),
                    align_items: AlignItems::Center,
                    ..default()
                })
                .with_children(|row| {
                    row.spawn((Node::default(), GuideKey));
                    row.spawn((ui::face("", ui::FONT_BOLD, 17.0, ui::INK), GuideLine));
                });
                slip.spawn((
                    ui::face("", ui::FONT_REGULAR, 14.0, ui::INK_SOFT),
                    GuideHint,
                ));
            });
        });

    // Dica de primeira vez: direita, à altura dos olhos.
    commands
        .spawn((
            ui::panel(Node {
                position_type: PositionType::Absolute,
                right: Val::Px(ui::MARGIN),
                top: Val::Percent(30.0),
                width: Val::Px(320.0),
                flex_direction: FlexDirection::Column,
                row_gap: Val::Px(4.0),
                display: Display::None,
                ..default()
            }),
            GlobalZIndex(20),
            TipSlip,
        ))
        .with_children(|slip| {
            slip.spawn((ui::display("", 19.0, ui::INK), TipTitle));
            slip.spawn((ui::face("", ui::FONT_REGULAR, 15.0, ui::INK), TipBody));
        });

    // Formatura: manchete no alto do mar.
    commands
        .spawn((
            Node {
                position_type: PositionType::Absolute,
                top: Val::Percent(16.0),
                width: Val::Percent(100.0),
                justify_content: JustifyContent::Center,
                display: Display::None,
                ..default()
            },
            GlobalZIndex(30),
            GraduationRoot,
        ))
        .with_children(|row| {
            row.spawn(ui::panel(Node {
                flex_direction: FlexDirection::Column,
                align_items: AlignItems::Center,
                padding: UiRect::axes(Val::Px(30.0), Val::Px(16.0)),
                row_gap: Val::Px(8.0),
                max_width: Val::Px(560.0),
                ..default()
            }))
            .with_children(|card| {
                ui::masthead(card, "VOCÊ É MERCADOR", 38.0);
                card.spawn((
                    i18n::label(
                        "Agora é com você: leve o que sobra num porto para onde falta e escolha os seus riscos.",
                        16.0,
                        ui::INK,
                    ),
                    TextLayout::new_with_justify(JustifyText::Center),
                ));
            });
        });
}

fn welcome_button(
    parent: &mut ChildBuilder,
    key: KeyCode,
    label: &'static str,
    kind: WelcomeButton,
    primary: bool,
) {
    let base = if primary {
        ui::BUTTON_SELECTED
    } else {
        ui::PAPER_SHADE
    };
    parent
        .spawn((
            ui::button(
                Node {
                    column_gap: Val::Px(10.0),
                    padding: UiRect::axes(Val::Px(14.0), Val::Px(8.0)),
                    ..default()
                },
                base,
            ),
            kind,
        ))
        .with_children(|button| {
            button.spawn((Node::default(), WelcomeKey(key)));
            button.spawn((i18n::label_face(label, ui::FONT_BOLD, 17.0, ui::INK),));
        });
}

/// Troca o conteúdo de um nó-âncora por um quadrinho de tecla.
fn place_keycap(commands: &mut Commands, anchor: Entity, label: Option<&str>) {
    commands.entity(anchor).despawn_descendants();
    if let Some(label) = label {
        commands
            .entity(anchor)
            .with_children(|slot| ui::keycap(slot, label));
    }
}

/// Rodapé do cartão: a tecla do livreto muda com o dispositivo.
#[derive(Component)]
struct WelcomeHelpLine;

const HELP_LINE_KEYBOARD: &str = "F1 abre o livreto do marujo a qualquer hora.";
const HELP_LINE_GAMEPAD: &str = "Start abre o livreto do marujo a qualquer hora.";

fn draw_welcome(
    mut commands: Commands,
    onboarding: Res<Onboarding>,
    device: Res<InputDevice>,
    mut root: Query<&mut Node, With<WelcomeRoot>>,
    keys: Query<(Entity, &WelcomeKey)>,
    mut help_line: Query<(&mut i18n::Translated, &mut Text), With<WelcomeHelpLine>>,
) {
    if !(onboarding.is_changed() || device.is_changed()) {
        return;
    }
    let display = if onboarding.welcome_open {
        Display::Flex
    } else {
        Display::None
    };
    for mut node in &mut root {
        node.display = display;
    }
    let help_key = match *device {
        InputDevice::Gamepad => HELP_LINE_GAMEPAD,
        InputDevice::Keyboard => HELP_LINE_KEYBOARD,
    };
    for (mut translated, mut text) in &mut help_line {
        translated.0 = help_key;
        text.0 = i18n::tr(help_key);
    }
    for (anchor, key) in &keys {
        let label = match (*device, key.0) {
            (InputDevice::Gamepad, KeyCode::Escape) => "B",
            (InputDevice::Gamepad, _) => "A",
            (InputDevice::Keyboard, other) => glyph(other, InputDevice::Keyboard),
        };
        place_keycap(&mut commands, anchor, Some(label));
    }
}

/// Alvo do passo no mar (recurso, porto) para a dica e a linha-guia.
fn step_target(
    step: Step,
    me: Vec2,
    nodes: &KnownNodes,
    map: Option<&marvyr_domain_world::WorldMap>,
    first_port: Option<&str>,
) -> Option<(String, Vec2)> {
    match step {
        Step::Gather => nodes
            .0
            .values()
            .filter(|node| node.stock > 0)
            .min_by(|a, b| me.distance(a.pos).total_cmp(&me.distance(b.pos)))
            .map(|node| (node.resource_name.clone(), node.pos)),
        Step::Dock | Step::Voyage => {
            // MV-066: "mais perto" é na carta de zonas, e a linha aponta para
            // o próximo portão do caminho até o porto.
            let map = map?;
            let chart = |x: f32, y: f32| Vec2::from(map.chart_position(x, y));
            let here = chart(me.x, me.y);
            let port = map
                .regions()
                .iter()
                .filter_map(|region| region.port.as_ref())
                .filter(|port| step == Step::Dock || Some(port.name) != first_port)
                .min_by(|a, b| {
                    here.distance(chart(a.x, a.y))
                        .total_cmp(&here.distance(chart(b.x, b.y)))
                })?;
            let hop = map.next_hop((me.x, me.y), (port.x, port.y))?;
            Some((port.name.to_owned(), Vec2::from(hop)))
        }
        _ => None,
    }
}

/// "Madeira a 320 m, nordeste" no idioma ativo.
fn target_hint(name: &str, from: Vec2, to: Vec2) -> String {
    let meters = from.distance(to);
    let distance = if meters >= 1000.0 {
        format!("{:.1} km", meters / 1000.0)
    } else {
        format!("{meters:.0} m")
    };
    i18n::trf("{0} a {1}, {2}", &[name, &distance, &compass(from, to)])
}

fn compass(from: Vec2, to: Vec2) -> String {
    let delta = to - from;
    let angle = delta.y.atan2(delta.x).to_degrees().rem_euclid(360.0);
    const PT: [&str; 8] = [
        "leste", "nordeste", "norte", "noroeste", "oeste", "sudoeste", "sul", "sudeste",
    ];
    i18n::tr(PT[(((angle + 22.5) / 45.0) as usize) % 8])
}

fn step_hint(step: Step, target: Option<&(String, Vec2)>, me: Option<Vec2>) -> String {
    if let (Some((name, pos)), Some(me)) = (target, me) {
        return target_hint(&i18n::tr(name), me, *pos);
    }
    i18n::tr(match step {
        Step::SetSail => "W iça as velas, A e D viram o leme. Quanto mais pano, mais rápido.",
        Step::Gather => "Pontos de recurso aparecem no mar com o nome e o estoque.",
        Step::Dock => "Chegue perto do porto até o bilhete de atracar aparecer.",
        Step::SellOrStore => "Aba Porão: Depositar tudo guarda a carga. Mercado e Guilda: trocar.",
        Step::Craft => "Aba Fabricação: escolha uma receita com os materiais que você tem.",
        Step::Voyage => "Cada porto paga diferente: o lucro está na viagem.",
    })
}

#[allow(clippy::too_many_arguments, clippy::type_complexity)]
fn draw_guide(
    mut commands: Commands,
    time: Res<Time>,
    onboarding: Res<Onboarding>,
    device: Res<InputDevice>,
    lang: Res<Lang>,
    nodes: Res<KnownNodes>,
    world: Option<Res<crate::world::ClientWorld>>,
    my_ship: Res<MyShip>,
    visuals: Query<&ShipVisual>,
    docked: Res<MyDocked>,
    mut slip: Query<(&mut Node, Has<GuideSlip>), Or<(With<GuideSlip>, With<GuidePaper>)>>,
    mut hint_visibility: Query<&mut Visibility, (With<GuideHint>, Without<GuideDone>)>,
    mut texts: ParamSet<(
        Query<&mut Text, With<GuideHeader>>,
        Query<&mut Text, With<GuideLine>>,
        Query<&mut Text, With<GuideHint>>,
    )>,
    mut done: Query<&mut Visibility, (With<GuideDone>, Without<GuideHint>)>,
    key_slot: Query<Entity, With<GuideKey>>,
    mut shown: Local<Option<(u8, InputDevice, Lang, bool)>>,
) {
    let now = time.elapsed_secs();
    let stamping = now < onboarding.done_until;
    for mut visibility in &mut done {
        visibility.set_if_neq(if stamping {
            Visibility::Inherited
        } else {
            Visibility::Hidden
        });
    }
    let guiding = onboarding.guiding() && !onboarding.welcome_open;
    for (mut node, is_row) in &mut slip {
        if is_row {
            let display = if guiding {
                Display::Flex
            } else {
                Display::None
            };
            let justify = if docked.0 {
                JustifyContent::Center
            } else {
                JustifyContent::Start
            };
            if node.display != display || node.justify_content != justify {
                node.display = display;
                node.justify_content = justify;
            }
        } else {
            // No porto a dica some e o bilhete vira uma tira baixa.
            let direction = if docked.0 {
                FlexDirection::Row
            } else {
                FlexDirection::Column
            };
            if node.flex_direction != direction {
                node.flex_direction = direction;
                node.column_gap = Val::Px(14.0);
                node.width = if docked.0 { Val::Auto } else { Val::Px(360.0) };
                node.padding = if docked.0 {
                    UiRect::axes(Val::Px(12.0), Val::Px(5.0))
                } else {
                    UiRect::axes(Val::Px(12.0), Val::Px(8.0))
                };
            }
        }
    }
    if !guiding {
        *shown = None;
        return;
    }
    let Some(step) = Step::from_index(onboarding.progress.step) else {
        return;
    };
    let me = my_state(&my_ship, &visuals).map(|state| Vec2::new(state.x, state.y));
    let target = me.and_then(|me| {
        step_target(
            step,
            me,
            &nodes,
            world.as_ref().map(|world| &world.0),
            onboarding.progress.first_port.as_deref(),
        )
    });
    // A dica (distância) muda todo quadro; o resto só quando o passo muda.
    for mut visibility in &mut hint_visibility {
        visibility.set_if_neq(if docked.0 {
            Visibility::Hidden
        } else {
            Visibility::Inherited
        });
    }
    let hint = step_hint(step, target.as_ref(), me);
    for mut text in &mut texts.p2() {
        if text.0 != hint {
            text.0 = hint.clone();
        }
    }
    let key = (step as u8, *device, *lang, docked.0);
    if *shown == Some(key) {
        return;
    }
    *shown = Some(key);
    let header = i18n::trf(
        "PRIMEIRA VIAGEM · {0} DE {1}",
        &[&(step as u8).to_string(), &STEPS.to_string()],
    );
    for mut text in &mut texts.p0() {
        text.0 = header.clone();
    }
    let (line, keycode) = step.instruction(docked.0);
    for mut text in &mut texts.p1() {
        text.0 = i18n::tr(line);
    }
    if let Ok(slot) = key_slot.get_single() {
        place_keycap(
            &mut commands,
            slot,
            keycode.map(|code| glyph(code, *device)),
        );
    }
}

fn draw_tip(
    time: Res<Time>,
    onboarding: Res<Onboarding>,
    mut slip: Query<&mut Node, With<TipSlip>>,
    mut titles: Query<&mut Text, (With<TipTitle>, Without<TipBody>)>,
    mut bodies: Query<&mut Text, (With<TipBody>, Without<TipTitle>)>,
) {
    let live = onboarding
        .tip
        .filter(|(_, until)| *until > time.elapsed_secs());
    for mut node in &mut slip {
        let display = if live.is_some() {
            Display::Flex
        } else {
            Display::None
        };
        if node.display != display {
            node.display = display;
        }
    }
    let Some((tip, _)) = live else {
        return;
    };
    let title = i18n::tr(tip.title);
    let body = i18n::tr(tip.body);
    for mut text in &mut titles {
        if text.0 != title {
            text.0 = title.clone();
        }
    }
    for mut text in &mut bodies {
        if text.0 != body {
            text.0 = body.clone();
        }
    }
}

fn draw_graduation(
    time: Res<Time>,
    onboarding: Res<Onboarding>,
    mut root: Query<&mut Node, With<GraduationRoot>>,
) {
    let show = time.elapsed_secs() < onboarding.graduation_until;
    for mut node in &mut root {
        let display = if show { Display::Flex } else { Display::None };
        if node.display != display {
            node.display = display;
        }
    }
}

/// Linha-guia tracejada em vermelhão do navio até o alvo do passo — o
/// fio que liga o bilhete ao mundo.
fn draw_leader_line(
    onboarding: Res<Onboarding>,
    world: Option<Res<crate::world::ClientWorld>>,
    docked: Res<MyDocked>,
    nodes: Res<KnownNodes>,
    my_ship: Res<MyShip>,
    visuals: Query<&ShipVisual>,
    mut gizmos: Gizmos,
) {
    if !onboarding.guiding() || docked.0 || onboarding.welcome_open {
        return;
    }
    let Some(step) = Step::from_index(onboarding.progress.step) else {
        return;
    };
    let Some(me) = my_state(&my_ship, &visuals).map(|state| Vec2::new(state.x, state.y)) else {
        return;
    };
    let Some((_, target)) = step_target(
        step,
        me,
        &nodes,
        world.as_ref().map(|world| &world.0),
        onboarding.progress.first_port.as_deref(),
    ) else {
        return;
    };
    let ink = ui::VERMILION.with_alpha(0.85);
    ui::dashed_line(&mut gizmos, me, target, 30.0, ink);
    gizmos.circle_2d(Isometry2d::from_translation(target), 26.0, ink);
    gizmos.circle_2d(
        Isometry2d::from_translation(target),
        30.0,
        ink.with_alpha(0.45),
    );
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn steps_follow_the_economic_loop() {
        let order: Vec<Step> = (1..=STEPS).filter_map(Step::from_index).collect();
        assert_eq!(
            order,
            vec![
                Step::SetSail,
                Step::Gather,
                Step::Dock,
                Step::SellOrStore,
                Step::Craft,
                Step::Voyage
            ]
        );
        assert!(Step::from_index(0).is_none());
        assert!(Step::from_index(STEPS + 1).is_none());
    }

    #[test]
    fn skipped_or_finished_players_are_not_guided() {
        let mut progress = Progress {
            welcomed: true,
            step: 1,
            ..Progress::default()
        };
        assert!(progress.guiding());
        progress.skipped = true;
        assert!(!progress.guiding());
        progress.skipped = false;
        progress.step = STEPS + 1;
        assert!(!progress.guiding(), "formado");
    }

    #[test]
    fn voyage_target_is_a_different_port() {
        let map = marvyr_domain_world::WorldMap::vertical_slice();
        let ports: Vec<_> = map
            .regions()
            .iter()
            .filter_map(|region| region.port.as_ref())
            .collect();
        let first = ports[0];
        let (name, _) = step_target(
            Step::Voyage,
            Vec2::new(first.x, first.y),
            &KnownNodes::default(),
            Some(&map),
            Some(first.name),
        )
        .expect("outro porto");
        assert_ne!(name, first.name);
    }

    #[test]
    fn compass_points_the_right_way() {
        assert_eq!(compass(Vec2::ZERO, Vec2::new(0.0, 10.0)), "norte");
        assert_eq!(compass(Vec2::ZERO, Vec2::new(10.0, 0.0)), "leste");
    }

    /// Monta todas as telas novas num mundo sem janela: bundle com
    /// componente duplicado derruba o app na hora do spawn.
    #[test]
    fn every_new_screen_spawns_without_panicking() {
        use bevy::ecs::schedule::Schedule;
        let mut world = World::new();
        let mut schedule = Schedule::default();
        schedule.add_systems((
            spawn_onboarding_ui,
            crate::help::spawn_book_for_tests,
            crate::hud::setup_hud,
        ));
        schedule.run(&mut world);
        let mut texts = world.query::<&Text>();
        assert!(texts.iter(&world).count() > 20);
    }

    /// Conflito de acesso entre parâmetros (B0001) só aparece quando o
    /// sistema inicializa — aqui, sem abrir janela.
    #[test]
    fn new_systems_have_no_conflicting_params() {
        fn init<M>(world: &mut World, system: impl IntoSystem<(), (), M>) {
            let mut system = IntoSystem::into_system(system);
            system.initialize(world);
        }
        let mut world = World::new();
        init(&mut world, open_welcome);
        init(&mut world, welcome_input);
        init(&mut world, restart_guide);
        init(&mut world, advance_guide);
        init(&mut world, first_time_tips);
        init(&mut world, draw_welcome);
        init(&mut world, draw_guide);
        init(&mut world, draw_tip);
        init(&mut world, draw_graduation);
        init(&mut world, draw_leader_line);
        init(&mut world, report_progress);
        crate::help::init_systems_for_tests(&mut world);
        crate::hud::init_systems_for_tests(&mut world);
        crate::seafaring::init_systems_for_tests(&mut world);
        crate::session::init_systems_for_tests(&mut world);
        crate::input::init_systems_for_tests(&mut world);
    }

    #[test]
    fn every_tip_id_resolves() {
        for id in [
            "frontier", "lawless", "portal", "event", "treasure", "hull", "full",
        ] {
            assert_eq!(tip(id).id, id);
        }
    }
}
