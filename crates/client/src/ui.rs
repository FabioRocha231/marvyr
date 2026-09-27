//! Tema e peças comuns da UI em espaço de tela (bevy_ui). Nada aqui é filho
//! da câmera: zoom/projeção não afetam o HUD nem a tela de porto.
//!
//! Mundo visual "Tipografia do Porto" (MV-062): o HUD é papel impresso do
//! porto — bilhetes de papel de trapo presos sobre o mar, tinta preta e
//! vermelhão (prensa de duas cores), tipos de madeira nas manchetes.

use bevy::prelude::*;
use bevy::sprite::{BorderRect, SliceScaleMode, TextureSlicer};
use bevy::ui::widget::NodeImageMode;

// ── Papel e tinta ──────────────────────────────────────────────────────
/// Papel de trapo: fundo de todo bilhete.
pub const PAPER: Color = Color::srgb(0.914, 0.863, 0.753);
/// Papel envelhecido: linhas alternadas, trilhos, cabeçalhos.
pub const PAPER_SHADE: Color = Color::srgb(0.847, 0.780, 0.635);
/// Tinta de impressão.
pub const INK: Color = Color::srgb(0.086, 0.125, 0.169);
/// Tinta rala (texto secundário no papel; ≥ 4.5:1 sobre PAPER).
pub const INK_SOFT: Color = Color::srgb(0.29, 0.31, 0.33);
/// Vermelhão da segunda cor (perigo, carimbos, manchetes).
pub const VERMILION: Color = Color::srgb(0.761, 0.212, 0.169);
/// Vermelhão para texto pequeno (contraste ≥ 4.5:1 sobre PAPER).
pub const VERMILION_INK: Color = Color::srgb(0.659, 0.157, 0.122);
/// Azul-mar: águas protegidas, estado "pronto".
pub const TEAL: Color = Color::srgb(0.122, 0.373, 0.451);
/// Latão: ouro e destaque (preenchimento, nunca texto pequeno no papel).
pub const BRASS: Color = Color::srgb(0.851, 0.643, 0.255);
/// Latão escuro: ouro como texto no papel.
pub const BRASS_INK: Color = Color::srgb(0.478, 0.306, 0.055);
/// Ocre: aviso intermediário (fronteira, casco a meio).
pub const OCHRE_INK: Color = Color::srgb(0.604, 0.353, 0.047);

// Nomes antigos do tema escuro, agora mapeados para o papel impresso.
pub const PANEL_BG: Color = PAPER;
pub const PANEL_BORDER: Color = INK;
pub const TEXT: Color = INK;
pub const TEXT_DIM: Color = INK_SOFT;
pub const GOLD: Color = BRASS_INK;
pub const DANGER: Color = VERMILION_INK;
pub const OK_GREEN: Color = TEAL;
pub const AMBER: Color = OCHRE_INK;
pub const BAR_TRACK: Color = Color::srgba(0.086, 0.125, 0.169, 0.16);
pub const BUTTON_BG: Color = PAPER_SHADE;
/// Linha escolhida: lavagem de latão sob tinta preta.
pub const BUTTON_SELECTED: Color = Color::srgb(0.906, 0.765, 0.455);
/// Sombra de papel sobre o mar.
pub const SHADOW: Color = Color::srgba(0.02, 0.05, 0.09, 0.45);
pub const MARGIN: f32 = 16.0;

// ── Tipos ──────────────────────────────────────────────────────────────
/// Tipo de madeira (Alfa Slab One): manchetes, título, carimbos.
pub const FONT_DISPLAY: Handle<Font> =
    Handle::weak_from_u128(0x4d56_0062_0000_0000_0000_0000_0000_0001);
/// Fira Sans SemiBold: rótulos curtos em caixa-alta.
pub const FONT_BOLD: Handle<Font> =
    Handle::weak_from_u128(0x4d56_0062_0000_0000_0000_0000_0000_0002);
/// Fira Sans Regular: prosa longa (ajuda, boas-vindas).
pub const FONT_REGULAR: Handle<Font> =
    Handle::weak_from_u128(0x4d56_0062_0000_0000_0000_0000_0000_0003);

/// Fira Sans Medium substitui a fonte padrão do Bevy (só ASCII): todo
/// texto do jogo — HUD, rótulos do mundo, placas — ganha acentos de uma vez.
/// Trocou a Zilla Slab (serifa de laje) porque borrava e cansava em tamanho
/// pequeno; a Alfa Slab One segue nos títulos.
const TEXT_FACE: &[u8] = include_bytes!("../../../assets/external/fonts/FiraSans-Medium.ttf");
const DISPLAY_FACE: &[u8] =
    include_bytes!("../../../assets/external/fonts/AlfaSlabOne-Regular.ttf");
const BOLD_FACE: &[u8] = include_bytes!("../../../assets/external/fonts/FiraSans-SemiBold.ttf");
const REGULAR_FACE: &[u8] = include_bytes!("../../../assets/external/fonts/FiraSans-Regular.ttf");

/// Botão com realce ao passar o mouse; `base` é a cor em repouso.
#[derive(Component, Clone, Copy)]
pub struct UiButton {
    pub base: Color,
}

pub struct UiThemePlugin;

impl Plugin for UiThemePlugin {
    fn build(&self, app: &mut App) {
        app.add_systems(PreStartup, install_fonts)
            .add_systems(Update, (button_hover, scale_to_window, dress_tickets));
    }
}

/// Registra as faces embutidas no binário (o zip da release não depende de
/// arquivo de fonte solto) e troca a fonte padrão pela Zilla Slab.
fn install_fonts(fonts: Option<ResMut<Assets<Font>>>) {
    let Some(mut fonts) = fonts else {
        return; // app headless de teste: sem texto para desenhar
    };
    // Dev (comparar fontes sem recompilar): MARVYR_FONT_DIR=<pasta> com
    // text.ttf, bold.ttf e regular.ttf troca as de texto (o título fica).
    let dev_dir = std::env::var("MARVYR_FONT_DIR").ok();
    let dev_face = |file: &str, embedded: &'static [u8]| -> Vec<u8> {
        dev_dir
            .as_ref()
            .and_then(|dir| std::fs::read(std::path::Path::new(dir).join(file)).ok())
            .unwrap_or_else(|| embedded.to_vec())
    };
    for (handle, bytes) in [
        (Handle::<Font>::default(), dev_face("text.ttf", TEXT_FACE)),
        (FONT_DISPLAY, DISPLAY_FACE.to_vec()),
        (FONT_BOLD, dev_face("bold.ttf", BOLD_FACE)),
        (FONT_REGULAR, dev_face("regular.ttf", REGULAR_FACE)),
    ] {
        match Font::try_from_bytes(bytes) {
            Ok(font) => {
                fonts.insert(handle.id(), font);
            }
            Err(error) => warn!(%error, "fonte embutida ilegível"),
        }
    }
}

/// Escala da UI pela janela lógica: 1.0 em 1280×720 (o desenho de
/// referência), maior em 1080p para ler do sofá com controle, e nunca
/// abaixo de 0.85 — em 1366×768 o papel ainda cabe e o texto não some.
pub fn ui_scale_for(width: f32, height: f32) -> f32 {
    (width / 1280.0).min(height / 720.0).clamp(0.85, 1.6)
}

fn scale_to_window(
    windows: Query<&Window, (With<bevy::window::PrimaryWindow>, Changed<Window>)>,
    mut scale: ResMut<UiScale>,
) {
    let Ok(window) = windows.get_single() else {
        return;
    };
    let next = ui_scale_for(window.width(), window.height());
    if (scale.0 - next).abs() > 0.01 {
        scale.0 = next;
    }
}

fn button_hover(
    mut buttons: Query<(&Interaction, &UiButton, &mut BackgroundColor), Changed<Interaction>>,
) {
    for (interaction, button, mut bg) in &mut buttons {
        bg.0 = match interaction {
            Interaction::Pressed => BRASS,
            Interaction::Hovered => button.base.mix(&BUTTON_SELECTED, 0.55),
            Interaction::None => button.base,
        };
    }
}

/// Bilhete padrão: papel de trapo, fio de tinta, sombra macia deslocada
/// sobre o mar. Cantos quase retos — é papel cortado, não cartão. A cor
/// chapada vale até a moldura pixel art ([`Ticket`]) carregar por cima.
pub fn panel(node: Node) -> impl Bundle {
    (
        Node {
            border: UiRect::all(Val::Px(2.0)),
            padding: UiRect::axes(Val::Px(12.0), Val::Px(8.0)),
            ..node
        },
        BackgroundColor(PANEL_BG),
        BorderColor(PANEL_BORDER),
        BorderRadius::all(Val::Px(2.0)),
        paper_shadow(),
        Ticket,
    )
}

/// Moldura 9-slice dos bilhetes (`marvyr/ui/ticket.png`, gerada por
/// `tools/art/marvyr_art.py`): bisel de papel, fio de tinta e rebites.
const TICKET: &str = "marvyr/ui/ticket.png";
/// Borda da moldura em px de tela (4 px de arte × 2).
const TICKET_BORDER: f32 = 8.0;

/// Marca um painel para receber a moldura pixel art.
#[derive(Component)]
pub struct Ticket;

/// Troca o fundo chapado dos bilhetes novos pela moldura 9-slice.
fn dress_tickets(
    mut commands: Commands,
    assets: Option<Res<AssetServer>>,
    mut tickets: Query<(Entity, &mut BackgroundColor, &mut BorderColor), Added<Ticket>>,
) {
    let Some(assets) = assets else {
        return; // app headless de teste
    };
    for (entity, mut bg, mut border) in &mut tickets {
        bg.0 = Color::NONE;
        border.0 = Color::NONE;
        commands
            .entity(entity)
            .insert(
                ImageNode::new(assets.load(TICKET)).with_mode(NodeImageMode::Sliced(
                    TextureSlicer {
                        border: BorderRect::square(TICKET_BORDER),
                        center_scale_mode: SliceScaleMode::Stretch,
                        sides_scale_mode: SliceScaleMode::Stretch,
                        max_corner_scale: 1.0,
                    },
                )),
            );
    }
}

/// Sombra do papel: deslocada para baixo, borrada — profundidade, não halo.
pub fn paper_shadow() -> BoxShadow {
    BoxShadow {
        color: SHADOW,
        x_offset: Val::Px(0.0),
        y_offset: Val::Px(3.0),
        spread_radius: Val::Px(0.0),
        blur_radius: Val::Px(6.0),
    }
}

/// Menor texto da UI (px lógicos em 1280×720): abaixo disso a letra pede
/// esforço para ler (queixa de playtest, v63).
pub const MIN_UI_TEXT: f32 = 13.0;

/// Texto de corpo (Fira Sans Medium). Passa pela tradução: a chave é o
/// texto em PT-BR (ver `i18n`).
pub fn text(value: impl Into<String>, size: f32, color: Color) -> impl Bundle {
    let value = value.into();
    (
        Text::new(crate::i18n::tr(&value)),
        TextFont {
            font_size: size.max(MIN_UI_TEXT),
            ..default()
        },
        TextColor(color),
    )
}

/// Texto em tipo de madeira (manchetes, título, carimbos).
pub fn display(value: impl Into<String>, size: f32, color: Color) -> impl Bundle {
    face(value, FONT_DISPLAY, size, color)
}

/// Texto numa face específica (`FONT_BOLD`, `FONT_REGULAR`).
pub fn face(value: impl Into<String>, font: Handle<Font>, size: f32, color: Color) -> impl Bundle {
    let value = value.into();
    (
        Text::new(crate::i18n::tr(&value)),
        TextFont {
            font,
            font_size: size.max(MIN_UI_TEXT),
            ..default()
        },
        TextColor(color),
    )
}

/// Manchete em tipo de madeira com a segunda cor fora de registro: a cópia
/// em vermelhão escorrega 2 px por baixo da tinta preta, como numa prensa
/// de duas passadas. É o momento tipográfico da tela — um por vez.
pub fn masthead(parent: &mut ChildBuilder, value: &str, size: f32) {
    masthead_colored(parent, value, size, INK, VERMILION);
}

pub fn masthead_colored(
    parent: &mut ChildBuilder,
    value: &str,
    size: f32,
    ink: Color,
    second: Color,
) {
    let offset = (size / 16.0).clamp(1.5, 4.0);
    parent.spawn(Node::default()).with_children(|stack| {
        stack.spawn((
            display(value, size, second),
            Node {
                position_type: PositionType::Absolute,
                left: Val::Px(offset),
                top: Val::Px(offset),
                ..default()
            },
            MastheadLayer,
        ));
        stack.spawn((display(value, size, ink), MastheadLayer));
    });
}

/// Camada de uma manchete (as duas cópias trocam de texto juntas).
#[derive(Component)]
pub struct MastheadLayer;

/// Fio de tinta de 1 px na largura toda.
pub fn rule(parent: &mut ChildBuilder) {
    parent.spawn((
        Node {
            width: Val::Percent(100.0),
            height: Val::Px(1.0),
            ..default()
        },
        BackgroundColor(INK),
    ));
}

/// Fio duplo de xilogravura (grosso + fino) sob mastros e cabeçalhos.
pub fn double_rule(parent: &mut ChildBuilder) {
    parent
        .spawn(Node {
            width: Val::Percent(100.0),
            flex_direction: FlexDirection::Column,
            row_gap: Val::Px(2.0),
            ..default()
        })
        .with_children(|rules| {
            for height in [3.0, 1.0] {
                rules.spawn((
                    Node {
                        width: Val::Percent(100.0),
                        height: Val::Px(height),
                        ..default()
                    },
                    BackgroundColor(INK),
                ));
            }
        });
}

/// Tecla impressa num quadrinho: "G", "LB", "Enter".
pub fn keycap(parent: &mut ChildBuilder, label: &str) {
    parent
        .spawn((
            Node {
                min_width: Val::Px(30.0),
                padding: UiRect::axes(Val::Px(7.0), Val::Px(2.0)),
                border: UiRect::all(Val::Px(1.5)),
                justify_content: JustifyContent::Center,
                align_items: AlignItems::Center,
                flex_shrink: 0.0,
                ..default()
            },
            BackgroundColor(PAPER_SHADE),
            BorderColor(INK),
            BorderRadius::all(Val::Px(3.0)),
            KeycapNode,
        ))
        .with_children(|cap| {
            // Letra de tecla passa intacta; glifos com palavra ("D-pad cima")
            // se traduzem.
            cap.spawn((
                Text::new(crate::i18n::tr(label)),
                TextFont {
                    font: FONT_BOLD,
                    font_size: 15.0,
                    ..default()
                },
                TextColor(INK),
                KeycapLabel,
            ));
        });
}

#[derive(Component)]
pub struct KeycapNode;
#[derive(Component)]
pub struct KeycapLabel;

/// Carimbo: moldura grossa na cor do estado, texto em caixa-alta.
pub fn stamp(value: &str, color: Color) -> impl Bundle {
    (
        Node {
            border: UiRect::all(Val::Px(2.0)),
            padding: UiRect::axes(Val::Px(6.0), Val::Px(1.0)),
            ..default()
        },
        BorderColor(color),
        BorderRadius::all(Val::Px(2.0)),
        Text::new(crate::i18n::tr(value)),
        TextFont {
            font: FONT_BOLD,
            font_size: 13.0,
            ..default()
        },
        TextColor(color),
    )
}

pub fn button(node: Node, base: Color) -> impl Bundle {
    (
        Button,
        Node {
            border: UiRect::all(Val::Px(1.0)),
            padding: UiRect::axes(Val::Px(10.0), Val::Px(5.0)),
            align_items: AlignItems::Center,
            justify_content: JustifyContent::Center,
            ..node
        },
        BackgroundColor(base),
        BorderColor(PANEL_BORDER.with_alpha(0.45)),
        BorderRadius::all(Val::Px(4.0)),
        UiButton { base },
    )
}

/// Barra horizontal: trilho escuro com um filho de preenchimento (`fill`)
/// cuja largura é percentual. Retorna nada; o filho carrega `fill`.
pub fn spawn_bar(parent: &mut ChildBuilder, width: f32, color: Color, fill: impl Bundle) {
    parent
        .spawn((
            Node {
                width: Val::Px(width),
                height: Val::Px(8.0),
                ..default()
            },
            BackgroundColor(BAR_TRACK),
            BorderRadius::all(Val::Px(3.0)),
        ))
        .with_children(|track| {
            track.spawn((
                Node {
                    width: Val::Percent(100.0),
                    height: Val::Percent(100.0),
                    ..default()
                },
                BackgroundColor(color),
                BorderRadius::all(Val::Px(3.0)),
                fill,
            ));
        });
}

/// Linha tracejada de tinta no mar (gizmos), de `from` a `to`, deixando
/// `inset` metros livres em cada ponta (casco e alvo).
pub fn dashed_line(gizmos: &mut Gizmos, from: Vec2, to: Vec2, inset: f32, color: Color) {
    const DASH: f32 = 14.0;
    const GAP: f32 = 10.0;
    let length = from.distance(to);
    if length <= inset * 2.0 {
        return;
    }
    let dir = (to - from) / length;
    let mut at = inset;
    while at < length - inset {
        let end = (at + DASH).min(length - inset);
        gizmos.line_2d(from + dir * at, from + dir * end, color);
        at = end + GAP;
    }
}

/// Fração 0..=1 como largura de barra.
pub fn bar_width(fraction: f32) -> Val {
    Val::Percent(fraction.clamp(0.0, 1.0) * 100.0)
}

/// Fade in → hold → fade out, usado por banner, aviso PvP e toasts.
/// Guarda a cor base do fundo/borda para aplicar alpha proporcional.
#[derive(Component, Clone, Copy)]
pub struct UiFade {
    pub elapsed: f32,
    pub fade_in: f32,
    pub hold_until: f32,
    pub end: f32,
    pub bg: Color,
    pub border: Color,
}

impl UiFade {
    pub fn new(fade_in: f32, hold_until: f32, end: f32, bg: Color, border: Color) -> Self {
        Self {
            elapsed: 0.0,
            fade_in,
            hold_until,
            end,
            bg,
            border,
        }
    }

    pub fn alpha(&self) -> f32 {
        let t = self.elapsed;
        if t < self.fade_in {
            t / self.fade_in
        } else if t < self.hold_until {
            1.0
        } else if t < self.end {
            1.0 - (t - self.hold_until) / (self.end - self.hold_until)
        } else {
            0.0
        }
    }
}

/// O que o fade mexe: fundo e borda chapados, ou a moldura do bilhete.
type FadeParts<'a> = (
    Entity,
    &'a mut UiFade,
    &'a mut BackgroundColor,
    &'a mut BorderColor,
    &'a Children,
    Option<&'a mut ImageNode>,
);

pub fn tick_ui_fades(
    time: Res<Time>,
    mut commands: Commands,
    mut fades: Query<FadeParts>,
    mut texts: Query<&mut TextColor>,
) {
    for (entity, mut fade, mut bg, mut border, children, image) in &mut fades {
        fade.elapsed += time.delta_secs();
        let alpha = fade.alpha();
        if let Some(mut image) = image {
            // Bilhete com moldura: quem esmaece é a imagem.
            image.color.set_alpha(alpha);
        } else {
            bg.0 = fade.bg.with_alpha(fade.bg.alpha() * alpha);
            border.0 = fade.border.with_alpha(fade.border.alpha() * alpha);
        }
        for child in children.iter() {
            if let Ok(mut color) = texts.get_mut(*child) {
                color.0.set_alpha(alpha);
            }
        }
        if fade.elapsed >= fade.end {
            commands.entity(entity).despawn_recursive();
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn fade_ramps_in_holds_and_ramps_out() {
        let mut fade = UiFade::new(0.4, 2.4, 3.2, PANEL_BG, PANEL_BORDER);
        assert_eq!(fade.alpha(), 0.0);
        fade.elapsed = 0.2;
        assert!((fade.alpha() - 0.5).abs() < 1e-5);
        fade.elapsed = 1.0;
        assert_eq!(fade.alpha(), 1.0);
        fade.elapsed = 2.8;
        assert!((fade.alpha() - 0.5).abs() < 1e-5);
        fade.elapsed = 3.5;
        assert_eq!(fade.alpha(), 0.0);
    }

    #[test]
    fn ui_scale_tracks_the_window_within_bounds() {
        assert_eq!(ui_scale_for(1280.0, 720.0), 1.0);
        assert!((ui_scale_for(1366.0, 768.0) - 1.0667).abs() < 0.01);
        assert_eq!(ui_scale_for(1920.0, 1080.0), 1.5);
        assert_eq!(ui_scale_for(800.0, 600.0), 0.85);
        assert_eq!(ui_scale_for(3840.0, 2160.0), 1.6);
    }

    #[test]
    fn bar_width_is_clamped_percentage() {
        assert_eq!(bar_width(0.25), Val::Percent(25.0));
        assert_eq!(bar_width(1.7), Val::Percent(100.0));
        assert_eq!(bar_width(-1.0), Val::Percent(0.0));
    }
}
