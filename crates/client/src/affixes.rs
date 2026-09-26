//! Afixos de equipamento na tela (v22): cor e rótulo por raridade, texto
//! dos afixos e a comemoração quando a oficina entrega uma peça Mágica ou
//! Rara. O servidor sorteia; aqui só se mostra o que veio.

use bevy::prelude::*;
use lightyear::prelude::ClientReceiveMessage;
use marvyr_domain_items::{Affix, AffixKind, Quality, Rarity};
use marvyr_protocol::CraftResult;

use crate::assets::{icons, ItemIcons};
use crate::crafting::KnownRecipes;
use crate::i18n::{tr, trf};
use crate::ui;

/// Tinta por raridade, legível sobre o papel da UI.
pub fn rarity_color(rarity: Rarity) -> Color {
    match rarity {
        Rarity::Normal => ui::TEXT,
        Rarity::Magic => Color::srgb(0.14, 0.30, 0.74),
        Rarity::Rare => Color::srgb(0.74, 0.42, 0.02),
    }
}

/// Brilho das faíscas (mais vivo que a tinta).
fn sparkle_color(rarity: Rarity) -> Color {
    match rarity {
        Rarity::Rare => Color::srgb(1.0, 0.8, 0.25),
        _ => Color::srgb(0.45, 0.65, 1.0),
    }
}

pub fn rarity_label(rarity: Rarity) -> &'static str {
    match rarity {
        Rarity::Normal => "Normal",
        Rarity::Magic => "Mágico",
        Rarity::Rare => "Raro",
    }
}

pub fn quality_rarity(quality: Option<&Quality>) -> Rarity {
    quality.map_or(Rarity::Normal, |quality| quality.rarity)
}

pub fn affix_label(affix: &Affix) -> String {
    let template = match affix.kind {
        AffixKind::Damage => "+{0} dano",
        AffixKind::Range => "+{0}% alcance",
        AffixKind::Hull => "+{0} casco",
        AffixKind::Speed => "+{0}% velocidade",
        AffixKind::Cargo => "+{0} porão",
        AffixKind::Turn => "+{0}% leme",
        AffixKind::Reload => "-{0}% recarga",
    };
    trf(template, &[&affix.value.to_string()])
}

/// "+5 dano · +8% alcance" (vazio na peça Normal).
pub fn affix_summary(quality: Option<&Quality>) -> String {
    quality
        .map(|quality| {
            quality
                .affixes
                .iter()
                .map(affix_label)
                .collect::<Vec<_>>()
                .join(" · ")
        })
        .unwrap_or_default()
}

/// Nome com a raridade: "Canhões Longos [Raro]" (Normal fica só o nome).
pub fn piece_name(name: &str, quality: Option<&Quality>) -> String {
    match quality_rarity(quality) {
        Rarity::Normal => tr(name),
        rarity => format!("{} [{}]", tr(name), tr(rarity_label(rarity))),
    }
}

/// Âncora no centro da tela onde a comemoração aparece.
#[derive(Component)]
struct CelebrationAnchor;

#[derive(Component)]
struct CelebrationPanel;

/// Faísca: voa do centro, cai um pouco e some.
#[derive(Component)]
struct Sparkle {
    velocity: Vec2,
    age: f32,
}

const SPARKLE_LIFE: f32 = 1.1;

/// Selo grande da comemoração: surge, pulsa e some junto do painel.
#[derive(Component)]
struct CelebrationIcon {
    age: f32,
    rarity: Rarity,
}

/// Mesmo ritmo do painel (`spawn_faded_panel` com (0.15, 3.0, 3.8)).
const CELEBRATION_FADE: (f32, f32, f32) = (0.15, 3.0, 3.8);

/// Moldura de raridade (fundo do selo), `scale` pixels de tela por pixel de
/// arte: a moldura tem 32 de arte, o ícone 24.
pub fn badge_frame(icons_atlas: &ItemIcons, rarity: Rarity, scale: f32) -> impl Bundle {
    (
        Node {
            width: Val::Px(32.0 * scale),
            height: Val::Px(32.0 * scale),
            justify_content: JustifyContent::Center,
            align_items: AlignItems::Center,
            flex_shrink: 0.0,
            ..default()
        },
        icons_atlas.node(icons::frame(rarity)),
    )
}

pub fn badge_icon(icons_atlas: &ItemIcons, icon: usize, scale: f32) -> impl Bundle {
    (
        Node {
            width: Val::Px(24.0 * scale),
            height: Val::Px(24.0 * scale),
            ..default()
        },
        icons_atlas.node(icon),
    )
}

/// Selo inline (listas do porto): moldura da raridade com o ícone dentro.
pub fn spawn_badge(
    parent: &mut ChildBuilder,
    icons_atlas: &ItemIcons,
    icon: usize,
    rarity: Rarity,
    scale: f32,
) {
    parent
        .spawn(badge_frame(icons_atlas, rarity, scale))
        .with_children(|frame| {
            frame.spawn(badge_icon(icons_atlas, icon, scale));
        });
}

pub struct AffixPlugin;

impl Plugin for AffixPlugin {
    fn build(&self, app: &mut App) {
        app.add_systems(Startup, spawn_anchor).add_systems(
            Update,
            (celebrate_craft, animate_sparkles, animate_celebration_icon),
        );
    }
}

fn spawn_anchor(mut commands: Commands) {
    commands.spawn((
        Node {
            position_type: PositionType::Absolute,
            width: Val::Percent(100.0),
            height: Val::Percent(100.0),
            justify_content: JustifyContent::Center,
            align_items: AlignItems::Center,
            ..default()
        },
        GlobalZIndex(35),
        PickingBehavior::IGNORE,
        CelebrationAnchor,
    ));
}

/// Linhas do painel: manchete da raridade, afixos um por linha.
pub fn celebration_lines(quality: &Quality) -> Vec<(String, f32, Color)> {
    let color = rarity_color(quality.rarity);
    let title = match quality.rarity {
        Rarity::Rare => tr("PEÇA RARA!"),
        _ => tr("PEÇA MÁGICA!"),
    };
    let mut lines = vec![(title, 30.0, color)];
    lines.extend(
        quality
            .affixes
            .iter()
            .map(|affix| (affix_label(affix), 17.0, ui::TEXT)),
    );
    lines
}

fn celebrate_craft(
    mut commands: Commands,
    mut results: EventReader<ClientReceiveMessage<CraftResult>>,
    recipes: Option<Res<KnownRecipes>>,
    icons_atlas: Option<Res<ItemIcons>>,
    anchor: Query<Entity, With<CelebrationAnchor>>,
    old: Query<Entity, With<CelebrationPanel>>,
) {
    let Some((recipe_id, quality)) = results
        .read()
        .filter_map(|event| {
            let result = event.message();
            Some((result.recipe_id, result.quality.clone()?))
        })
        .last()
    else {
        return;
    };
    // Ícone da peça pela saída da receita (nome do catálogo).
    let icon = recipes.as_ref().and_then(|recipes| {
        recipes
            .0
            .iter()
            .find(|entry| entry.recipe_id == recipe_id)
            .and_then(|entry| icons::item(&entry.output_name))
    });
    let Ok(anchor) = anchor.get_single() else {
        return;
    };
    for entity in &old {
        commands.entity(entity).despawn_recursive();
    }
    let lines = celebration_lines(&quality);
    let borrowed: Vec<(&str, f32, Color)> = lines
        .iter()
        .map(|(text, size, color)| (text.as_str(), *size, *color))
        .collect();
    let panel = crate::hud::spawn_faded_panel(
        &mut commands,
        anchor,
        CELEBRATION_FADE,
        rarity_color(quality.rarity),
        CelebrationPanel,
        &borrowed,
    );
    // A peça em si, grande, entre a manchete e os afixos.
    if let (Some(icon), Some(icons_atlas)) = (icon, icons_atlas.as_ref()) {
        let mark = || CelebrationIcon {
            age: 0.0,
            rarity: quality.rarity,
        };
        let badge = commands
            .spawn((badge_frame(icons_atlas, quality.rarity, 4.0), mark()))
            .with_children(|frame| {
                frame.spawn((badge_icon(icons_atlas, icon, 4.0), mark()));
            })
            .id();
        commands.entity(panel).insert_children(1, &[badge]);
    }
    // Raro solta mais faíscas que Mágico.
    let count = if quality.rarity == Rarity::Rare {
        28
    } else {
        14
    };
    let glow = sparkle_color(quality.rarity);
    for i in 0..count {
        let angle = i as f32 / count as f32 * std::f32::consts::TAU + (i % 3) as f32 * 0.21;
        let speed = 160.0 + (i % 5) as f32 * 45.0;
        commands
            .spawn((
                Node {
                    position_type: PositionType::Absolute,
                    width: Val::Px(8.0),
                    height: Val::Px(8.0),
                    ..default()
                },
                BackgroundColor(glow),
                Sparkle {
                    velocity: Vec2::from_angle(angle) * speed,
                    age: 0.0,
                },
            ))
            .set_parent(anchor);
    }
}

fn animate_celebration_icon(
    time: Res<Time>,
    mut icons_q: Query<(&mut CelebrationIcon, &mut ImageNode)>,
) {
    let (fade_in, hold, end) = CELEBRATION_FADE;
    for (mut mark, mut image) in &mut icons_q {
        mark.age += time.delta_secs();
        let t = mark.age;
        let alpha = if t < fade_in {
            t / fade_in
        } else if t < hold {
            1.0
        } else {
            (1.0 - (t - hold) / (end - hold)).max(0.0)
        };
        // A moldura rara pulsa mais forte (o brilho do "super").
        let pulse = match mark.rarity {
            Rarity::Rare => 0.22,
            _ => 0.12,
        };
        let glow = 1.0 - pulse + pulse * (t * 7.0).sin().abs();
        image.color = Color::srgba(glow, glow, glow, alpha);
    }
}

fn animate_sparkles(
    mut commands: Commands,
    time: Res<Time>,
    mut sparkles: Query<(Entity, &mut Sparkle, &mut Node, &mut BackgroundColor)>,
) {
    let dt = time.delta_secs();
    for (entity, mut sparkle, mut node, mut color) in &mut sparkles {
        sparkle.age += dt;
        if sparkle.age >= SPARKLE_LIFE {
            commands.entity(entity).despawn_recursive();
            continue;
        }
        // Gravidade leve: sobe em leque e cai.
        sparkle.velocity.y += 260.0 * dt;
        let at = sparkle.velocity * sparkle.age;
        // O centro da âncora é a origem (a faísca nasce no meio da tela).
        node.margin = UiRect {
            left: Val::Px(at.x),
            top: Val::Px(at.y),
            ..default()
        };
        let alpha = 1.0 - sparkle.age / SPARKLE_LIFE;
        color.0 = color.0.with_alpha(alpha);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn labels_read_as_the_player_expects() {
        let quality = Quality {
            rarity: Rarity::Rare,
            affixes: vec![
                Affix {
                    kind: AffixKind::Damage,
                    value: 5,
                },
                Affix {
                    kind: AffixKind::Reload,
                    value: 7,
                },
            ],
        };
        assert_eq!(affix_summary(Some(&quality)), "+5 dano · -7% recarga");
        assert_eq!(
            piece_name("Canhões Longos", Some(&quality)),
            "Canhões Longos [Raro]"
        );
        assert_eq!(piece_name("Canhões Longos", None), "Canhões Longos");
        let lines = celebration_lines(&quality);
        assert_eq!(lines[0].0, "PEÇA RARA!");
        assert_eq!(lines.len(), 3);
        assert_eq!(
            crate::i18n::trf_in("+{0}% alcance", &["8"], crate::i18n::Lang::En),
            "+8% range"
        );
    }
}
