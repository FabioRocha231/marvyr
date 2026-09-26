//! HUD do mar (MF-057A/B/C, MF-058, MV-062) em bevy_ui: espaço de tela,
//! independente da câmera, do zoom e do tamanho da janela. Tudo é papel
//! impresso preso sobre o mar (ver `ui`): bilhete do navio (topo-esq.),
//! carimbo de zona (topo-centro), bilhete de ação com linha-guia até o alvo
//! e bilhete dos canhões + velas (base-centro), avisos (direita-meio) e o
//! lembrete do livreto (base-dir.). Atracado, o HUD do mar esconde; o Port
//! Screen assume.

use bevy::prelude::*;
use marvyr_domain_world::RiskTier;
use marvyr_shared::ids::ItemDefinitionId;

use crate::input::{ContextKey, KeySlot};
use crate::market::KnownCatalog;
use crate::net::{KnownWrecks, MyShip, GATHER_RADIUS_SQ, LOOT_RADIUS_SQ};
use crate::nodes::KnownNodes;
use crate::ui::{self, UiFade};
use crate::zone::{risk_tag, CurrentZone};

/// Tudo que vive no HUD do mar. Atracado, todos os paineis escondem juntos.
#[derive(Component)]
pub struct SeaHud;

#[derive(Component)]
pub struct ShipPanel;
#[derive(Component)]
pub struct ZonePanel;
#[derive(Component)]
pub struct CooldownPanel;
#[derive(Component)]
pub struct PromptPanel;

/// Texto do indicador de velas/marcha; o texto é dirigido por outro sistema.
#[derive(Component)]
pub struct SailIndicator;

/// Âncora (centro-topo) onde o banner de zona nasce.
#[derive(Component)]
pub struct ZoneBannerAnchor;
/// Âncora (centro) onde a placa de PvP nasce.
#[derive(Component)]
pub struct PvpWarningAnchor;
/// Pilha de toasts de contexto (direita-meio).
#[derive(Component)]
pub struct ToastStack;

#[derive(Component)]
pub struct ZoneBannerPanel;
#[derive(Component)]
pub struct PvpWarningPanel;
/// Toast curto de feedback de ação de contexto (atracar, coletar, saquear).
#[derive(Component)]
pub struct ContextToast;

/// Qual texto do HUD este nó mostra (um único `Query<&mut Text>` por sistema).
#[derive(Component, Debug, Clone, Copy, PartialEq, Eq)]
pub enum HudText {
    ShipName,
    Hp,
    Cargo,
    ZoneName,
    ZoneTag,
    ZoneRisk,
    Reload,
    /// v21: estado da Bandeira Negra.
    Flag,
    Prompt,
    /// MV-067: nível e progresso de Renome.
    Renown,
}

/// Qual barra este nó de preenchimento representa.
#[derive(Component, Debug, Clone, Copy, PartialEq, Eq)]
pub enum HudFill {
    Hp,
    Cargo,
    Renown,
    Reload,
    Flag,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum HudContext {
    Idle,
    NearPort,
    NearWreck,
    NearNode(ItemDefinitionId),
    /// Em cima do X de um mapa do tesouro.
    AtDigSpot,
    /// Navio avariado (ou parado) ao alcance da abordagem.
    CanBoard,
    /// Casco ferido, navio quase parado e sem reparo em curso.
    CanRepair,
    /// v46: colado num farol aceso.
    NearLighthouse,
    /// v46: parado perto da costa, sem farol perto.
    CanRaiseLighthouse,
    /// v51: garrafa boiando ao alcance.
    NearBottle,
}

/// O bilhete de ação da vez (quem age pela mesma tecla confere o contexto).
#[derive(Resource, Debug, Clone, Copy, PartialEq, Eq)]
pub struct PromptContext(pub HudContext);

impl Default for PromptContext {
    fn default() -> Self {
        Self(HudContext::Idle)
    }
}

/// Alvo do bilhete de ação no mar, para a linha-guia.
#[derive(Resource, Debug, Default, Clone, Copy, PartialEq)]
pub struct PromptTarget(pub Option<Vec2>);

/// Vaga da tecla do bilhete de ação.
#[derive(Component)]
pub struct PromptKey;

/// Casinha do nível de pano (0..3): cheia = pano armado.
#[derive(Component)]
pub struct SailCell(u8);

/// Recarga de bordo do servidor (`server::net` tuning.cooldown_secs).
// ponytail: espelha a constante do servidor; mandar no snapshot se virar por navio.
const BROADSIDE_RELOAD_SECS: f32 = 3.0;

pub struct HudPlugin;

impl Plugin for HudPlugin {
    fn build(&self, app: &mut App) {
        app.init_resource::<PromptTarget>()
            .init_resource::<PromptContext>()
            .add_systems(Startup, setup_hud)
            .add_systems(
                Update,
                (
                    update_ship_panel,
                    update_zone_panel,
                    update_cooldown_panel,
                    update_prompt_panel,
                    update_sail_indicator,
                    draw_prompt_leader,
                    ui::tick_ui_fades,
                ),
            );
    }
}

/// Nível de pano armado (W/S), lido do `SailLevel` do client: três casas
/// sempre desenhadas (as vazias também) e o nome do pano embaixo.
fn update_sail_indicator(
    sail: Res<crate::net::SailLevel>,
    lang: Option<Res<crate::i18n::Lang>>,
    mut texts: Query<&mut Text, With<SailIndicator>>,
    mut cells: Query<(&SailCell, &mut BackgroundColor)>,
) {
    let lang_changed = lang.is_some_and(|lang| lang.is_changed());
    if !(sail.is_changed() || lang_changed) {
        return;
    }
    for mut text in &mut texts {
        text.0 = sail_indicator_text(*sail);
    }
    for (cell, mut bg) in &mut cells {
        bg.0 = if cell.0 < sail.0 {
            ui::INK
        } else {
            Color::NONE
        };
    }
}

fn sail_indicator_text(sail: crate::net::SailLevel) -> String {
    crate::i18n::tr(sail.label())
}

fn anchored(node: Node) -> Node {
    Node {
        position_type: PositionType::Absolute,
        ..node
    }
}

fn spawn_stat_row(parent: &mut ChildBuilder, label: &str, text: HudText, fill: HudFill) {
    parent
        .spawn(Node {
            align_items: AlignItems::Center,
            column_gap: Val::Px(8.0),
            ..default()
        })
        .with_children(|row| {
            row.spawn((
                ui::face(label, ui::FONT_BOLD, 12.0, ui::INK_SOFT),
                Node {
                    width: Val::Px(52.0),
                    ..default()
                },
            ));
            ui::spawn_bar(row, 150.0, ui::OK_GREEN, fill);
            row.spawn((ui::face("-", ui::FONT_BOLD, 14.0, ui::INK), text));
        });
}

/// Bloco tecla + rótulo + valor + barra (recarga dos canhões, bandeira).
fn spawn_gauge(
    parent: &mut ChildBuilder,
    label: &'static str,
    key: KeyCode,
    text: HudText,
    fill: HudFill,
) {
    parent
        .spawn(Node {
            flex_direction: FlexDirection::Column,
            row_gap: Val::Px(5.0),
            width: Val::Px(160.0),
            flex_shrink: 0.0,
            ..default()
        })
        .with_children(|col| {
            col.spawn(Node {
                justify_content: JustifyContent::SpaceBetween,
                align_items: AlignItems::Center,
                column_gap: Val::Px(6.0),
                ..default()
            })
            .with_children(|row| {
                row.spawn((Node::default(), KeySlot(key)));
                row.spawn(crate::i18n::label_face(
                    label,
                    ui::FONT_BOLD,
                    12.0,
                    ui::INK_SOFT,
                ));
                row.spawn((
                    ui::face(cooldown_label(0.0), ui::FONT_BOLD, 13.0, ui::OK_GREEN),
                    text,
                ));
            });
            ui::spawn_bar(col, 160.0, ui::OK_GREEN, fill);
        });
}

pub fn setup_hud(mut commands: Commands) {
    // Topo-esquerda: bilhete do navio — tipo do casco em tipo de madeira,
    // fio duplo, casco e carga, e a linha de bordo.
    commands
        .spawn((
            ui::panel(anchored(Node {
                left: Val::Px(ui::MARGIN),
                top: Val::Px(ui::MARGIN),
                flex_direction: FlexDirection::Column,
                row_gap: Val::Px(6.0),
                padding: UiRect::axes(Val::Px(14.0), Val::Px(10.0)),
                ..default()
            })),
            SeaHud,
            ShipPanel,
        ))
        .with_children(|panel| {
            panel
                .spawn(Node {
                    justify_content: JustifyContent::SpaceBetween,
                    align_items: AlignItems::FlexEnd,
                    column_gap: Val::Px(18.0),
                    ..default()
                })
                .with_children(|head| {
                    head.spawn((ui::display("-", 22.0, ui::INK), HudText::ShipName));
                });
            ui::double_rule(panel);
            spawn_stat_row(panel, "CASCO", HudText::Hp, HudFill::Hp);
            spawn_stat_row(panel, "CARGA", HudText::Cargo, HudFill::Cargo);
            spawn_stat_row(panel, "RENOME", HudText::Renown, HudFill::Renown);
            panel.spawn((
                ui::face("", ui::FONT_REGULAR, 14.0, ui::INK_SOFT),
                crate::seafaring::SeaStatusText,
            ));
        });

    // Topo-centro: nome das águas e o carimbo do risco.
    commands
        .spawn((
            anchored(Node {
                top: Val::Px(ui::MARGIN),
                left: Val::Px(0.0),
                right: Val::Px(0.0),
                justify_content: JustifyContent::Center,
                ..default()
            }),
            SeaHud,
        ))
        .with_children(|row| {
            row.spawn((
                ui::panel(Node {
                    flex_direction: FlexDirection::Column,
                    align_items: AlignItems::Center,
                    row_gap: Val::Px(4.0),
                    padding: UiRect::axes(Val::Px(18.0), Val::Px(8.0)),
                    ..default()
                }),
                ZonePanel,
            ))
            .with_children(|panel| {
                panel.spawn((ui::display("-", 19.0, ui::INK), HudText::ZoneName));
                panel
                    .spawn(Node {
                        column_gap: Val::Px(8.0),
                        align_items: AlignItems::Center,
                        ..default()
                    })
                    .with_children(|tags| {
                        tags.spawn((ui::stamp("", ui::OK_GREEN), HudText::ZoneTag));
                        tags.spawn((ui::text("", 13.0, ui::INK_SOFT), HudText::ZoneRisk));
                    });
            });
        });

    // Base-centro: bilhete de ação acima do bilhete dos canhões e velas.
    commands
        .spawn((
            anchored(Node {
                bottom: Val::Px(ui::MARGIN),
                left: Val::Px(0.0),
                right: Val::Px(0.0),
                flex_direction: FlexDirection::Column,
                align_items: AlignItems::Center,
                row_gap: Val::Px(10.0),
                ..default()
            }),
            SeaHud,
        ))
        .with_children(|col| {
            col.spawn((
                ui::panel(Node {
                    display: Display::None,
                    column_gap: Val::Px(10.0),
                    align_items: AlignItems::Center,
                    padding: UiRect::axes(Val::Px(12.0), Val::Px(7.0)),
                    ..default()
                }),
                PromptPanel,
            ))
            .with_children(|panel| {
                panel.spawn((Node::default(), PromptKey, KeySlot(KeyCode::KeyE)));
                panel.spawn((ui::face("", ui::FONT_BOLD, 17.0, ui::INK), HudText::Prompt));
            });
            // v25: cinto de frascos (teclas 1-4); os ícones entram quando o
            // atlas carregar (`flasks::rebuild_belt_with_icons`).
            crate::flasks::spawn_belt(col, None);
            col.spawn((
                ui::panel(Node {
                    align_items: AlignItems::Center,
                    column_gap: Val::Px(22.0),
                    padding: UiRect::axes(Val::Px(16.0), Val::Px(9.0)),
                    ..default()
                }),
                CooldownPanel,
            ))
            .with_children(|panel| {
                // v21: Q trava o alvo dos canhões (o tiro é automático).
                spawn_gauge(
                    panel,
                    "CANHÕES",
                    KeyCode::KeyQ,
                    HudText::Reload,
                    HudFill::Reload,
                );
                panel
                    .spawn(Node {
                        flex_direction: FlexDirection::Column,
                        align_items: AlignItems::Center,
                        row_gap: Val::Px(4.0),
                        min_width: Val::Px(110.0),
                        flex_shrink: 0.0,
                        ..default()
                    })
                    .with_children(|sails| {
                        sails
                            .spawn(Node {
                                column_gap: Val::Px(4.0),
                                ..default()
                            })
                            .with_children(|cells| {
                                for index in 0..crate::net::SailLevel::MAX {
                                    cells.spawn((
                                        Node {
                                            width: Val::Px(16.0),
                                            height: Val::Px(10.0),
                                            border: UiRect::all(Val::Px(1.5)),
                                            ..default()
                                        },
                                        BackgroundColor(Color::NONE),
                                        BorderColor(ui::INK),
                                        SailCell(index),
                                    ));
                                }
                            });
                        sails.spawn((
                            ui::face("-", ui::FONT_BOLD, 13.0, ui::INK),
                            TextLayout::new_with_justify(JustifyText::Center),
                            SailIndicator,
                        ));
                    });
                spawn_gauge(
                    panel,
                    "BANDEIRA",
                    KeyCode::KeyR,
                    HudText::Flag,
                    HudFill::Flag,
                );
            });
        });

    // Base-direita: só o lembrete do livreto — as teclas moram nos
    // bilhetes de contexto e no F1.
    commands
        .spawn((
            ui::panel(anchored(Node {
                right: Val::Px(ui::MARGIN),
                bottom: Val::Px(ui::MARGIN),
                column_gap: Val::Px(8.0),
                align_items: AlignItems::Center,
                padding: UiRect::axes(Val::Px(10.0), Val::Px(6.0)),
                ..default()
            })),
            SeaHud,
        ))
        .with_children(|panel| {
            panel.spawn((Node::default(), KeySlot(KeyCode::F1)));
            panel.spawn(crate::i18n::label_face(
                "Livreto",
                ui::FONT_BOLD,
                14.0,
                ui::INK,
            ));
        });

    // Direita-meio: pilha de avisos (o mais novo em cima, os antigos somem).
    commands.spawn((
        anchored(Node {
            right: Val::Px(ui::MARGIN),
            top: Val::Percent(48.0),
            flex_direction: FlexDirection::Column,
            align_items: AlignItems::End,
            row_gap: Val::Px(6.0),
            ..default()
        }),
        SeaHud,
        ToastStack,
    ));

    // Âncoras de banner de zona e aviso de PvP (acima de tudo).
    for (top, anchor_zone) in [(24.0, true), (38.0, false)] {
        let mut anchor = commands.spawn((
            anchored(Node {
                top: Val::Percent(top),
                left: Val::Px(0.0),
                right: Val::Px(0.0),
                justify_content: JustifyContent::Center,
                ..default()
            }),
            GlobalZIndex(10),
        ));
        if anchor_zone {
            anchor.insert(ZoneBannerAnchor);
        } else {
            anchor.insert(PvpWarningAnchor);
        }
    }
}

fn cooldown_label(secs: f32) -> String {
    if secs <= 0.0 {
        crate::i18n::tr("PRONTO")
    } else {
        format!("{:.0}s", secs.ceil())
    }
}

/// Rótulo, cor e barra da Bandeira Negra (`ShipState.black_flag`).
fn flag_gauge(flag: u8) -> (&'static str, Color, f32) {
    match flag {
        marvyr_protocol::FLAG_RAISED => ("NEGRA", ui::DANGER, 1.0),
        marvyr_protocol::FLAG_HOISTING => ("IÇANDO", ui::AMBER, 0.5),
        _ => ("ARRIADA", ui::TEXT_DIM, 0.0),
    }
}

fn reload_fraction(secs: f32) -> f32 {
    1.0 - (secs / BROADSIDE_RELOAD_SECS).clamp(0.0, 1.0)
}

fn hp_color(current: u32, max: u32) -> Color {
    if max == 0 {
        return ui::DANGER;
    }
    let ratio = current as f32 / max as f32;
    if ratio > 0.6 {
        ui::OK_GREEN
    } else if ratio > 0.3 {
        ui::AMBER
    } else {
        ui::DANGER
    }
}

fn ratio(current: u32, max: u32) -> f32 {
    if max == 0 {
        0.0
    } else {
        current as f32 / max as f32
    }
}

fn zone_tag(tier: RiskTier) -> (&'static str, Color) {
    match tier {
        RiskTier::Protected => ("PROTEGIDO", ui::OK_GREEN),
        RiskTier::Frontier => ("FRONTEIRA", ui::AMBER),
        RiskTier::Lawless => ("SEM LEI", ui::DANGER),
    }
}

/// "Águas do Porto da Serra" -> Some("Porto da Serra").
fn port_of_zone(zone_name: &str) -> Option<&str> {
    let rest = zone_name
        .strip_prefix("Águas do ")
        .or_else(|| zone_name.strip_prefix("Aguas do "))?;
    rest.starts_with("Porto").then_some(rest)
}

fn hud_context(
    pos: Vec2,
    zone: &CurrentZone,
    wrecks: &KnownWrecks,
    nodes: &KnownNodes,
    catalog: &KnownCatalog,
) -> HudContext {
    if zone
        .0
        .as_ref()
        .is_some_and(|zone| port_of_zone(&zone.name).is_some())
    {
        return HudContext::NearPort;
    }
    if wrecks
        .0
        .values()
        .any(|wreck| pos.distance_squared(*wreck) <= LOOT_RADIUS_SQ)
    {
        return HudContext::NearWreck;
    }
    let nearest = nodes
        .0
        .values()
        .filter(|info| info.stock > 0 && pos.distance_squared(info.pos) <= GATHER_RADIUS_SQ)
        .min_by(|a, b| {
            let da = pos.distance_squared(a.pos);
            let db = pos.distance_squared(b.pos);
            da.total_cmp(&db)
        });
    if let Some(node) = nearest {
        if let Some(line) = catalog.0.get(&node.resource_name) {
            return HudContext::NearNode(line.id);
        }
    }
    HudContext::Idle
}

/// Texto do bilhete de ação (a tecla vai num quadrinho ao lado).
fn context_prompt(context: &HudContext, catalog: &KnownCatalog, port: Option<&str>) -> String {
    use crate::i18n::{tr, trf};
    match context {
        HudContext::Idle => String::new(),
        HudContext::NearPort => trf("Atracar em {0}", &[&tr(port.unwrap_or("porto"))]),
        HudContext::NearWreck => tr("Saquear destroço"),
        HudContext::NearNode(item) => {
            let name = catalog
                .0
                .values()
                .find(|line| line.id == *item)
                .map(|line| line.name.as_str())
                .unwrap_or("recurso");
            trf("Coletar {0}", &[&tr(name)])
        }
        HudContext::AtDigSpot => tr("Cavar o tesouro"),
        HudContext::CanBoard => tr("Abordar"),
        HudContext::CanRepair => tr("Reparar o casco"),
        HudContext::NearLighthouse => tr("Reforçar o farol"),
        HudContext::CanRaiseLighthouse => tr("Erguer um farol"),
        HudContext::NearBottle => tr("Pescar a garrafa"),
    }
}

/// Tecla que resolve o bilhete (e que o botão A do controle aciona).
fn context_key(context: &HudContext) -> Option<KeyCode> {
    Some(match context {
        HudContext::Idle => return None,
        HudContext::NearPort => KeyCode::KeyE,
        HudContext::NearWreck => KeyCode::KeyF,
        HudContext::NearNode(_) => KeyCode::KeyG,
        HudContext::AtDigSpot => KeyCode::KeyJ,
        HudContext::CanBoard => KeyCode::KeyH,
        HudContext::CanRepair => KeyCode::KeyK,
        HudContext::NearLighthouse | HudContext::CanRaiseLighthouse | HudContext::NearBottle => {
            KeyCode::KeyB
        }
    })
}

fn my_visual<'a>(
    my_ship: &MyShip,
    visuals: &'a Query<&crate::ship::ShipVisual>,
) -> Option<&'a marvyr_protocol::ShipState> {
    let my_id = my_ship.0?;
    visuals
        .iter()
        .find(|v| v.target.ship_id == my_id)
        .map(|v| &v.target)
}

pub fn update_ship_panel(
    my_ship: Res<MyShip>,
    renown: Option<Res<crate::renown::MyRenown>>,
    progress: Option<Res<crate::logbook::MyProgress>>,
    visuals: Query<&crate::ship::ShipVisual>,
    mut texts: Query<(&mut Text, &HudText)>,
    mut fills: Query<(&mut Node, &mut BackgroundColor, &HudFill)>,
) {
    let Some(state) = my_visual(&my_ship, &visuals) else {
        return;
    };
    for (mut text, kind) in &mut texts {
        let value = match kind {
            // v42: o casco com a maestria dele ("Corsário · M3").
            HudText::ShipName => {
                let name = crate::i18n::tr(state.kind.name());
                match progress
                    .as_ref()
                    .map(|p| crate::logbook::mastery_of(&p.0, state.kind.name()))
                {
                    Some(level) if level > 0 => format!("{name} · M{level}"),
                    _ => name,
                }
            }
            HudText::Hp => format!("{}/{}", state.hp, state.max_hp),
            HudText::Cargo => format!("{}/{}", state.cargo_weight, state.cargo_capacity),
            HudText::Renown => renown
                .as_ref()
                .and_then(|renown| renown.0.as_ref())
                .map(crate::renown::level_label)
                .unwrap_or_else(|| String::from("-")),
            _ => continue,
        };
        if text.0 != value {
            text.0 = value;
        }
    }
    for (mut node, mut bg, fill) in &mut fills {
        match fill {
            // Só escreve quando muda: Node/Text marcados como mudados
            // forçam relayout e reshaping de texto no frame inteiro.
            HudFill::Hp => {
                set_width(&mut node, ui::bar_width(ratio(state.hp, state.max_hp)));
                bg.set_if_neq(BackgroundColor(hp_color(state.hp, state.max_hp)));
            }
            HudFill::Cargo => {
                set_width(
                    &mut node,
                    ui::bar_width(ratio(state.cargo_weight, state.cargo_capacity)),
                );
                bg.set_if_neq(BackgroundColor(ui::GOLD.with_alpha(0.85)));
            }
            HudFill::Renown => {
                let fraction = renown
                    .as_ref()
                    .and_then(|renown| renown.0.as_ref())
                    .map_or(0.0, crate::renown::level_fraction);
                set_width(&mut node, ui::bar_width(fraction));
                bg.set_if_neq(BackgroundColor(ui::BRASS));
            }
            HudFill::Reload | HudFill::Flag => {}
        }
    }
}

pub fn update_zone_panel(
    zone: Res<CurrentZone>,
    lang: Option<Res<crate::i18n::Lang>>,
    mut texts: Query<(
        &mut Text,
        &mut TextColor,
        &HudText,
        Option<&mut BorderColor>,
    )>,
) {
    let lang_changed = lang.is_some_and(|lang| lang.is_changed());
    if !(zone.is_changed() || lang_changed) {
        return;
    }
    let Some(zone) = zone.0.as_ref() else {
        return;
    };
    let (tag, tag_color) = zone_tag(zone.tier);
    for (mut text, mut color, kind, border) in &mut texts {
        match kind {
            HudText::ZoneName => text.0 = short_zone_name(&zone.name),
            HudText::ZoneTag => {
                text.0 = crate::i18n::tr(tag);
                color.0 = tag_color;
                // O carimbo muda de tinta com o risco.
                if let Some(mut border) = border {
                    border.0 = tag_color;
                }
            }
            HudText::ZoneRisk => text.0 = crate::i18n::tr(risk_tag(zone.tier)),
            _ => {}
        }
    }
}

fn short_zone_name(full: &str) -> String {
    // Servidor manda "Águas do Porto da Serra" etc. — encurtamos para caber
    // no carimbo do HUD (a fonte agora desenha acentos).
    // O nome do porto vai inteiro e traduzido, igual ao rótulo do mapa.
    for prefix in ["Águas do Porto ", "Aguas do Porto "] {
        if let Some(rest) = full.strip_prefix(prefix) {
            return crate::i18n::tr(&format!("Porto {rest}"));
        }
    }
    for prefix in ["Águas da Ilha do ", "Aguas da Ilha do "] {
        if let Some(rest) = full.strip_prefix(prefix) {
            return crate::i18n::tr(rest);
        }
    }
    crate::i18n::tr(full)
}

pub fn update_cooldown_panel(
    my_ship: Res<MyShip>,
    visuals: Query<&crate::ship::ShipVisual>,
    mut texts: Query<(&mut Text, &mut TextColor, &HudText)>,
    mut fills: Query<(&mut Node, &mut BackgroundColor, &HudFill)>,
) {
    let Some(state) = my_visual(&my_ship, &visuals) else {
        return;
    };
    // Recarga única (v21): os dois bordos carregam juntos.
    let s = state.port_cooldown_secs.max(state.starboard_cooldown_secs);
    let (flag_label, flag_color, flag_fill) = flag_gauge(state.black_flag);
    for (mut text, mut color, kind) in &mut texts {
        let (label, tint) = match kind {
            HudText::Reload => (
                cooldown_label(s),
                if s <= 0.0 { ui::OK_GREEN } else { ui::TEXT_DIM },
            ),
            HudText::Flag => (crate::i18n::tr(flag_label), flag_color),
            _ => continue,
        };
        if text.0 != label {
            text.0 = label;
        }
        if color.0 != tint {
            color.0 = tint;
        }
    }
    for (mut node, mut bg, fill) in &mut fills {
        let (fraction, tint) = match fill {
            HudFill::Reload => (
                reload_fraction(s),
                if s <= 0.0 { ui::OK_GREEN } else { ui::AMBER },
            ),
            HudFill::Flag => (flag_fill, flag_color),
            _ => continue,
        };
        set_width(&mut node, ui::bar_width(fraction));
        bg.set_if_neq(BackgroundColor(tint));
    }
}

#[allow(clippy::too_many_arguments, clippy::type_complexity)]
pub fn update_prompt_panel(
    my_ship: Res<MyShip>,
    zone: Res<CurrentZone>,
    wrecks: Res<KnownWrecks>,
    nodes: Res<KnownNodes>,
    catalog: Res<KnownCatalog>,
    marks: Res<crate::seafaring::TreasureMarks>,
    visuals: Query<&crate::ship::ShipVisual>,
    mut panel: Query<&mut Node, With<PromptPanel>>,
    mut texts: Query<(&mut Text, &HudText)>,
    mut slot: Query<&mut KeySlot, With<PromptKey>>,
    mut context_key_res: ResMut<ContextKey>,
    mut target_res: ResMut<PromptTarget>,
    world: Option<Res<crate::world::ClientWorld>>,
    lighthouses: Res<crate::lighthouse::KnownLighthouses>,
    (bottles, mut prompt_context): (Res<crate::bottle::KnownBottles>, ResMut<PromptContext>),
) {
    let Some(state) = my_visual(&my_ship, &visuals) else {
        return;
    };
    let port_at = |name: &str| {
        world
            .as_ref()?
            .0
            .regions()
            .iter()
            .filter_map(|region| region.port.as_ref())
            .find(|port| port.name == name)
            .map(|port| Vec2::new(port.x, port.y))
    };
    let pos = Vec2::new(state.x, state.y);
    let port = zone.0.as_ref().and_then(|zone| port_of_zone(&zone.name));
    let nearest = |points: &mut dyn Iterator<Item = Vec2>| {
        points.min_by(|a, b| {
            pos.distance_squared(*a)
                .total_cmp(&pos.distance_squared(*b))
        })
    };
    let dig = nearest(&mut marks.0.iter().map(|mark| Vec2::new(mark.x, mark.y)))
        .filter(|spot| pos.distance(*spot) <= marvyr_domain_world::treasure::DIG_RADIUS);
    let others: Vec<marvyr_protocol::ShipState> = visuals.iter().map(|v| v.target).collect();
    // Só oferece a abordagem que o servidor aceitaria: alvo avariado ou parado.
    let board = crate::seafaring::board_target(state, &others)
        .and_then(|id| others.iter().find(|other| other.ship_id == id))
        .filter(|target| target.hp * 100 <= target.max_hp * 35 || target.speed < 1.0)
        .map(|target| Vec2::new(target.x, target.y));
    use crate::lighthouse::LighthouseAction;
    let lighthouse = world
        .as_ref()
        .and_then(|world| crate::lighthouse::action_at(&lighthouses, &world.0, pos));
    let (context, target) = match hud_context(pos, &zone, &wrecks, &nodes, &catalog) {
        HudContext::NearPort => (HudContext::NearPort, port.and_then(port_at)),
        HudContext::NearWreck => (
            HudContext::NearWreck,
            nearest(&mut wrecks.0.values().copied()),
        ),
        _ if dig.is_some() => (HudContext::AtDigSpot, dig),
        _ if board.is_some() => (HudContext::CanBoard, board),
        HudContext::NearNode(item) => (
            HudContext::NearNode(item),
            nearest(
                &mut nodes
                    .0
                    .values()
                    .filter(|node| node.stock > 0)
                    .map(|node| node.pos),
            ),
        ),
        HudContext::Idle
            if state.speed < 3.0 && crate::bottle::bottle_near(&bottles, pos).is_some() =>
        {
            (
                HudContext::NearBottle,
                crate::bottle::bottle_near(&bottles, pos),
            )
        }
        HudContext::Idle if lighthouse == Some(LighthouseAction::Tend) => {
            (HudContext::NearLighthouse, None)
        }
        HudContext::Idle if state.hp < state.max_hp && !state.repairing && state.speed < 2.0 => {
            (HudContext::CanRepair, None)
        }
        HudContext::Idle if lighthouse == Some(LighthouseAction::Raise) && state.speed < 2.0 => {
            (HudContext::CanRaiseLighthouse, None)
        }
        other => (other, None),
    };
    prompt_context.set_if_neq(PromptContext(context));
    let prompt = context_prompt(&context, &catalog, port);
    let key = context_key(&context);
    context_key_res.set_if_neq(ContextKey(key));
    target_res.set_if_neq(PromptTarget(target));
    if let (Some(key), Ok(mut slot)) = (key, slot.get_single_mut()) {
        if slot.0 != key {
            slot.0 = key;
        }
    }
    let display = if prompt.is_empty() {
        Display::None
    } else {
        Display::Flex
    };
    for mut node in &mut panel {
        if node.display != display {
            node.display = display;
        }
    }
    for (mut text, kind) in &mut texts {
        if *kind == HudText::Prompt && text.0 != prompt {
            text.0 = prompt.clone();
        }
    }
}

/// Linha-guia de tinta do navio até o alvo do bilhete de ação.
fn draw_prompt_leader(
    target: Res<PromptTarget>,
    docked: Res<crate::net::MyDocked>,
    my_ship: Res<MyShip>,
    visuals: Query<&crate::ship::ShipVisual>,
    mut gizmos: Gizmos,
) {
    let Some(target) = target.0.filter(|_| !docked.0) else {
        return;
    };
    let Some(state) = my_visual(&my_ship, &visuals) else {
        return;
    };
    let ink = ui::INK.with_alpha(0.8);
    ui::dashed_line(&mut gizmos, Vec2::new(state.x, state.y), target, 22.0, ink);
    gizmos.circle_2d(Isometry2d::from_translation(target), 18.0, ink);
}

pub fn toggle_sea_hud(
    docked: Res<crate::net::MyDocked>,
    status: Option<Res<crate::session::ConnectionStatus>>,
    mut hud: Query<&mut Visibility, With<SeaHud>>,
) {
    // Antes de entrar no mar (login, conectando) o HUD não existe para o
    // jogador: o cartaz de entrada fica sozinho sobre o mar.
    let at_sea = !status.is_some_and(|status| *status != crate::session::ConnectionStatus::InGame);
    let visibility = if docked.0 || !at_sea {
        Visibility::Hidden
    } else {
        Visibility::Visible
    };
    for mut entity in &mut hud {
        entity.set_if_neq(visibility);
    }
}

/// Largura de barra sem marcar o `Node` como mudado à toa.
pub(crate) fn set_width(node: &mut Mut<Node>, width: Val) {
    if node.width != width {
        node.width = width;
    }
}

pub(crate) fn spawn_faded_panel(
    commands: &mut Commands,
    parent: Entity,
    fade: (f32, f32, f32),
    border: Color,
    marker: impl Bundle,
    lines: &[(&str, f32, Color)],
) -> Entity {
    let bg = ui::PANEL_BG;
    commands
        .spawn((
            Node {
                flex_direction: FlexDirection::Column,
                align_items: AlignItems::Center,
                row_gap: Val::Px(4.0),
                padding: UiRect::axes(Val::Px(22.0), Val::Px(10.0)),
                border: UiRect::all(Val::Px(2.0)),
                max_width: Val::Px(560.0),
                ..default()
            },
            BackgroundColor(bg.with_alpha(0.0)),
            BorderColor(border.with_alpha(0.0)),
            BorderRadius::all(Val::Px(2.0)),
            UiFade::new(fade.0, fade.1, fade.2, bg, border),
            marker,
        ))
        .with_children(|panel| {
            for (value, size, color) in lines {
                // Linha grande vira manchete em tipo de madeira.
                let font = if *size >= 22.0 {
                    ui::FONT_DISPLAY
                } else {
                    Handle::default()
                };
                panel.spawn((
                    ui::face(*value, font, *size, color.with_alpha(0.0)),
                    TextLayout::new_with_justify(JustifyText::Center),
                ));
            }
        })
        .set_parent(parent)
        .id()
}

/// Banner momentâneo de zona (fade 0.4s in, até 2.4s, out até 3.2s).
pub fn spawn_zone_banner(commands: &mut Commands, anchor: Entity, name: &str) {
    let display = short_zone_name(name).to_uppercase();
    spawn_faded_panel(
        commands,
        anchor,
        (0.4, 2.4, 3.2),
        ui::PANEL_BORDER,
        ZoneBannerPanel,
        &[(&display, 34.0, ui::INK)],
    );
}

/// Faixa de nível novo de Renome (MV-067), no mesmo lugar da faixa de zona.
pub fn spawn_level_banner(commands: &mut Commands, anchor: Entity, level: u32) {
    let title = crate::i18n::trf("RENOME {0}", &[&level.to_string()]);
    let hint = crate::i18n::tr("+1 ponto na Rosa dos Ventos · tecla I");
    spawn_faded_panel(
        commands,
        anchor,
        (0.3, 2.8, 3.6),
        ui::BRASS,
        ZoneBannerPanel,
        &[(&title, 34.0, ui::BRASS_INK), (&hint, 16.0, ui::INK)],
    );
}

/// Placa de aviso de PvP (fade 0.5s in, até 4s, out até 5s).
pub fn spawn_pvp_warning(commands: &mut Commands, anchor: Entity) {
    spawn_faded_panel(
        commands,
        anchor,
        (0.5, 4.0, 5.0),
        ui::VERMILION,
        PvpWarningPanel,
        &[
            ("ÁGUAS DE RISCO", 30.0, ui::VERMILION_INK),
            (
                "Seu navio, equipamentos e carga podem ser perdidos.",
                16.0,
                ui::INK,
            ),
        ],
    );
}

/// Toast de contexto na pilha da direita (fade 0.3s in, até 2s, out até 2.5s).
pub fn spawn_context_toast(commands: &mut Commands, stack: Entity, message: &str) {
    spawn_faded_panel(
        commands,
        stack,
        (0.3, 2.0, 2.5),
        ui::PANEL_BORDER,
        ContextToast,
        &[(message, 15.0, ui::TEXT)],
    );
}

#[cfg(test)]
pub(crate) fn init_systems_for_tests(world: &mut World) {
    fn init<M>(world: &mut World, system: impl IntoSystem<(), (), M>) {
        let mut system = IntoSystem::into_system(system);
        system.initialize(world);
    }
    init(world, update_ship_panel);
    init(world, update_zone_panel);
    init(world, update_cooldown_panel);
    init(world, update_prompt_panel);
    init(world, update_sail_indicator);
    init(world, draw_prompt_leader);
    init(world, toggle_sea_hud);
}

#[cfg(test)]
mod tests {
    use std::collections::HashMap;
    use std::time::Instant;

    use bevy::ecs::schedule::Schedule;
    use marvyr_domain_world::RiskTier;
    use marvyr_protocol::ItemLine;

    use crate::ship::ShipVisual;
    use crate::zone::ServerZone;

    use super::*;

    #[test]
    fn sail_indicator_shows_level_bar_and_label() {
        use crate::net::SailLevel;
        assert_eq!(sail_indicator_text(SailLevel(0)), "Velas recolhidas");
        assert_eq!(sail_indicator_text(SailLevel(3)), "Pano cheio");
    }

    fn ship_state(port_cooldown: f32, starboard_cooldown: f32) -> marvyr_protocol::ShipState {
        marvyr_protocol::ShipState {
            ship_id: 1,
            kind: marvyr_domain_ships::ShipKind::SmallMerchant,
            x: 0.0,
            y: 0.0,
            heading: 0.0,
            speed: 0.0,
            cargo_weight: 8,
            hp: 120,
            max_hp: 150,
            max_speed: 30.0,
            weapon_damage: 20,
            weapon_range: 50.0,
            port_cooldown_secs: port_cooldown,
            starboard_cooldown_secs: starboard_cooldown,
            is_npc: false,
            cargo_capacity: 100,
            sail_hp: 100.0,
            ammo: Default::default(),
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
            fury: 0,
            title: 0,
            morale: 100,
        }
    }

    fn texts(world: &mut World) -> Vec<String> {
        let mut q = world.query::<&Text>();
        q.iter(world).map(|t| t.0.clone()).collect()
    }

    #[test]
    fn cooldown_label_is_human_readable() {
        assert_eq!(cooldown_label(0.0), "PRONTO");
        assert_eq!(cooldown_label(0.4), "1s");
        assert_eq!(cooldown_label(3.6), "4s");
    }

    #[test]
    fn reload_bar_fills_as_cooldown_drops() {
        assert_eq!(reload_fraction(BROADSIDE_RELOAD_SECS), 0.0);
        assert_eq!(reload_fraction(0.0), 1.0);
        assert!((reload_fraction(BROADSIDE_RELOAD_SECS * 0.25) - 0.75).abs() < 1e-5);
    }

    #[test]
    fn hp_color_thresholds_are_well_ordered() {
        let high = hp_color(100, 100);
        let mid = hp_color(40, 100);
        let low = hp_color(10, 100);
        assert_ne!(high, mid);
        assert_ne!(mid, low);
    }

    #[test]
    fn short_zone_name_strips_hulls() {
        assert_eq!(short_zone_name("Aguas do Porto da Serra"), "Porto da Serra");
        assert_eq!(short_zone_name("Aguas do Porto da Mina"), "Porto da Mina");
        assert_eq!(
            short_zone_name("Águas da Ilha do Coral Negro"),
            "Coral Negro"
        );
        assert_eq!(short_zone_name("Rota da Costa"), "Rota da Costa");
        assert_eq!(short_zone_name("Cerração"), "Cerração");
    }

    #[test]
    fn port_of_zone_accepts_accented_server_names() {
        assert_eq!(
            port_of_zone("Águas do Porto da Serra"),
            Some("Porto da Serra")
        );
        assert_eq!(
            port_of_zone("Aguas do Porto da Mina"),
            Some("Porto da Mina")
        );
        assert_eq!(port_of_zone("Rota da Costa"), None);
    }

    #[test]
    fn context_prompt_is_empty_when_idle() {
        let prompt = context_prompt(&HudContext::Idle, &KnownCatalog::default(), None);
        assert!(prompt.is_empty());
    }

    #[test]
    fn context_prompt_uses_port_label_when_near_port() {
        let prompt = context_prompt(
            &HudContext::NearPort,
            &KnownCatalog::default(),
            Some("Porto da Serra"),
        );
        assert_eq!(prompt, "Atracar em Porto da Serra");
        assert_eq!(context_key(&HudContext::NearPort), Some(KeyCode::KeyE));
    }

    #[test]
    fn context_prompt_names_resource_from_catalog() {
        let id = ItemDefinitionId::new();
        let catalog = KnownCatalog(HashMap::from([(
            String::from("Madeira"),
            ItemLine {
                id,
                name: String::from("Madeira"),
                weight: 2,
                equipment_slot: None,
            },
        )]));
        let prompt = context_prompt(&HudContext::NearNode(id), &catalog, None);
        assert_eq!(prompt, "Coletar Madeira");
        assert_eq!(context_key(&HudContext::NearNode(id)), Some(KeyCode::KeyG));
    }

    #[test]
    fn update_cooldown_panel_renders_reload_and_flag() {
        let mut world = World::new();
        world.insert_resource(MyShip(Some(1)));
        world.spawn(ShipVisual {
            target: marvyr_protocol::ShipState {
                black_flag: marvyr_protocol::FLAG_RAISED,
                ..ship_state(BROADSIDE_RELOAD_SECS * 0.75, BROADSIDE_RELOAD_SECS * 0.75)
            },
            last_seen: Instant::now(),
        });
        world.spawn((Text::default(), TextColor::default(), HudText::Reload));
        world.spawn((Text::default(), TextColor::default(), HudText::Flag));
        let port_fill = world
            .spawn((Node::default(), BackgroundColor::default(), HudFill::Reload))
            .id();

        let mut sched = Schedule::default();
        sched.add_systems(update_cooldown_panel);
        sched.run(&mut world);

        let texts = texts(&mut world);
        assert!(texts.iter().any(|t| t == "3s"), "texts={texts:?}");
        assert!(texts.iter().any(|t| t == "NEGRA"), "texts={texts:?}");
        assert_eq!(
            world.get::<Node>(port_fill).unwrap().width,
            Val::Percent(25.0)
        );
    }

    #[test]
    fn update_ship_panel_fills_hp_and_cargo() {
        let mut world = World::new();
        world.insert_resource(MyShip(Some(1)));
        world.spawn(ShipVisual {
            target: ship_state(0.0, 0.0),
            last_seen: Instant::now(),
        });
        for kind in [HudText::Hp, HudText::Cargo] {
            world.spawn((Text::default(), kind));
        }
        let hp_fill = world
            .spawn((Node::default(), BackgroundColor::default(), HudFill::Hp))
            .id();

        let mut sched = Schedule::default();
        sched.add_systems(update_ship_panel);
        sched.run(&mut world);

        let texts = texts(&mut world);
        for expected in ["120/150", "8/100"] {
            assert!(texts.iter().any(|t| t == expected), "texts={texts:?}");
        }
        assert_eq!(
            world.get::<Node>(hp_fill).unwrap().width,
            Val::Percent(80.0)
        );
    }

    #[test]
    fn update_zone_panel_reflects_current_zone() {
        let mut world = World::new();
        world.insert_resource(CurrentZone(Some(ServerZone {
            tier: RiskTier::Frontier,
            name: String::from("Rota da Costa"),
        })));
        for kind in [HudText::ZoneName, HudText::ZoneTag, HudText::ZoneRisk] {
            world.spawn((Text::default(), TextColor::default(), kind));
        }

        let mut sched = Schedule::default();
        sched.add_systems(update_zone_panel);
        sched.run(&mut world);

        let texts = texts(&mut world);
        assert!(
            texts.iter().any(|t| t == "Rota da Costa"),
            "texts={texts:?}"
        );
        assert!(texts.iter().any(|t| t == "FRONTEIRA"), "texts={texts:?}");
        assert!(
            texts.iter().any(|t| t == risk_tag(RiskTier::Frontier)),
            "texts={texts:?}"
        );
    }

    #[test]
    fn docked_hides_sea_hud() {
        let mut world = World::new();
        world.insert_resource(crate::net::MyDocked(true));
        let hud_ent = world.spawn((SeaHud, Visibility::Visible)).id();

        let mut sched = Schedule::default();
        sched.add_systems(toggle_sea_hud);
        sched.run(&mut world);
        assert_eq!(
            *world.get::<Visibility>(hud_ent).unwrap(),
            Visibility::Hidden
        );

        world.insert_resource(crate::net::MyDocked(false));
        sched.run(&mut world);
        assert_eq!(
            *world.get::<Visibility>(hud_ent).unwrap(),
            Visibility::Visible
        );
    }

    #[test]
    fn hud_does_not_attach_to_camera() {
        let mut world = World::new();
        let camera = world.spawn(Camera2d).id();
        let mut sched = Schedule::default();
        sched.add_systems(setup_hud);
        sched.run(&mut world);

        assert!(world.get::<Children>(camera).is_none());
        let mut q = world.query_filtered::<Entity, With<SailIndicator>>();
        assert_eq!(q.iter(&world).count(), 1);
    }
}
