//! Texto do mundo nítido em qualquer zoom. O Bevy 0.15 rasteriza o `Text2d`
//! só pela escala da janela e ignora o zoom da câmera (bevy#5621): com a
//! câmera ampliando 2× (padrão), 2,5× (combate) ou até 5× (roda do mouse),
//! placas, nomes de porto e letreiros saíam esticados e embaçados. Aqui
//! cada texto do mundo é gerado já no tamanho em que aparece na tela e a
//! escala compensa — o tamanho no mundo não muda.
//!
//! Cada texto do mundo ganha também uma sombra escura deslocada (filha,
//! mesma letra): "Ilha do Coral Negro" sobre areia e o nome do capitão
//! sobre água clara sumiam sem contorno.
//!
//! Quem cria ou anima o texto continua escrevendo `font_size` e
//! `Transform::scale` como sempre: o `Crisp` lembra o que este sistema
//! escreveu por último e trata qualquer valor diferente como escrita nova
//! de quem é dono do texto.

use bevy::prelude::*;
use bevy::sprite::Anchor;

/// Passo do fator (limita quantos tamanhos de glifo vão para o atlas).
const STEP: f32 = 0.25;
const MIN_FACTOR: f32 = 0.5;
const MAX_FACTOR: f32 = 4.0;

pub struct CrispTextPlugin;

impl Plugin for CrispTextPlugin {
    fn build(&self, app: &mut App) {
        app.add_systems(
            PostUpdate,
            (add_shadows, sharpen_world_text, sync_shadows)
                .chain()
                .before(bevy::text::Update2dText)
                .before(bevy::transform::TransformSystem::TransformPropagate),
        );
    }
}

#[derive(Component, Debug, Clone, Copy, PartialEq)]
struct Crisp {
    /// Tamanho e escala que o dono do texto quer.
    base_size: f32,
    base_scale: Vec3,
    /// O que este sistema escreveu por último.
    written_size: f32,
    written_scale: Vec3,
}

/// Fator de rasterização para a escala da projeção (câmera ampliando:
/// escala < 1, fator > 1).
fn factor_for(projection_scale: f32) -> f32 {
    if projection_scale <= 0.0 {
        return 1.0;
    }
    ((1.0 / projection_scale / STEP).round() * STEP).clamp(MIN_FACTOR, MAX_FACTOR)
}

/// Novo estado a partir do que está no componente agora. Escrita do dono
/// igual ao último valor escrito aqui passa despercebida (nenhum dono faz
/// isso hoje).
fn crisp_update(current: Option<Crisp>, size: f32, scale: Vec3, factor: f32) -> Crisp {
    let (base_size, base_scale) = match current {
        Some(crisp) => (
            if size == crisp.written_size {
                crisp.base_size
            } else {
                size
            },
            if scale == crisp.written_scale {
                crisp.base_scale
            } else {
                scale
            },
        ),
        None => (size, scale),
    };
    Crisp {
        base_size,
        base_scale,
        written_size: base_size * factor,
        written_scale: base_scale / factor,
    }
}

/// Deslocamento da sombra em unidades do texto (vira ~2 px na tela).
const SHADOW_OFFSET: Vec2 = Vec2::new(1.5, -1.5);
const SHADOW: Color = Color::srgba(0.02, 0.05, 0.09, 0.8);

/// Sombra de um texto do mundo (filha dele).
#[derive(Component)]
struct TextShadow;

/// Texto que já ganhou sombra.
#[derive(Component)]
struct Shadowed;

#[allow(clippy::type_complexity)]
fn add_shadows(
    mut commands: Commands,
    texts: Query<
        (
            Entity,
            &Text2d,
            &TextFont,
            Option<&Anchor>,
            Option<&TextLayout>,
        ),
        (Without<Shadowed>, Without<TextShadow>),
    >,
) {
    for (entity, text, font, anchor, layout) in &texts {
        let mut shadow = commands.spawn((
            TextShadow,
            Text2d::new(text.0.clone()),
            font.clone(),
            TextColor(SHADOW),
            Transform::from_translation(SHADOW_OFFSET.extend(-0.01)),
            anchor.copied().unwrap_or_default(),
        ));
        if let Some(layout) = layout {
            shadow.insert(*layout);
        }
        let shadow = shadow.id();
        commands.entity(entity).insert(Shadowed).add_child(shadow);
    }
}

/// A sombra copia letra, tamanho e transparência do texto; texto que
/// sumiu (despawn sem os filhos) leva a sombra junto.
#[allow(clippy::type_complexity)]
fn sync_shadows(
    mut commands: Commands,
    parents: Query<(&Text2d, &TextFont, &TextColor), Without<TextShadow>>,
    mut shadows: Query<
        (Entity, &Parent, &mut Text2d, &mut TextFont, &mut TextColor),
        With<TextShadow>,
    >,
) {
    for (entity, parent, mut text, mut font, mut color) in &mut shadows {
        let Ok((source, source_font, source_color)) = parents.get(parent.get()) else {
            commands.entity(entity).despawn();
            continue;
        };
        if text.0 != source.0 {
            text.0.clone_from(&source.0);
        }
        if font.font_size != source_font.font_size {
            font.font_size = source_font.font_size;
        }
        let alpha = SHADOW.alpha() * source_color.0.alpha();
        if color.0.alpha() != alpha {
            color.0 = SHADOW.with_alpha(alpha);
        }
    }
}

#[allow(clippy::type_complexity)]
fn sharpen_world_text(
    mut commands: Commands,
    camera: Query<&OrthographicProjection, With<Camera2d>>,
    mut texts: Query<
        (Entity, &mut TextFont, &mut Transform, Option<&mut Crisp>),
        (
            With<Text2d>,
            Without<TextShadow>,
            // O número de dano já compensa o zoom na escala (juice.rs).
            Without<crate::juice::DamageNumber>,
        ),
    >,
) {
    let Ok(projection) = camera.get_single() else {
        return;
    };
    let factor = factor_for(projection.scale);
    for (entity, mut font, mut transform, crisp) in &mut texts {
        let current = crisp.as_deref().copied();
        let next = crisp_update(current, font.font_size, transform.scale, factor);
        if Some(next) == current {
            continue;
        }
        // Só escreve o que mudou: re-layout de texto custa.
        if font.font_size != next.written_size {
            font.font_size = next.written_size;
        }
        if transform.scale != next.written_scale {
            transform.scale = next.written_scale;
        }
        match crisp {
            Some(mut crisp) => *crisp = next,
            None => {
                commands.entity(entity).insert(next);
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn zoomed_in_text_is_rasterized_bigger_and_scaled_back() {
        assert_eq!(factor_for(0.5), 2.0);
        assert_eq!(factor_for(0.4), 2.5);
        assert_eq!(factor_for(0.2), 4.0);
        assert_eq!(factor_for(1.4), 0.75);
        let first = crisp_update(None, 14.0, Vec3::ONE, 2.0);
        assert_eq!(first.written_size, 28.0);
        assert_eq!(first.written_scale, Vec3::splat(0.5));
        // Mesmo tamanho na tela: fonte × escala / projeção.
        assert_eq!(first.written_size * first.written_scale.x, 14.0);
    }

    #[test]
    fn owner_writes_are_taken_as_the_new_base() {
        let crisp = crisp_update(None, 14.0, Vec3::ONE, 2.0);
        // Nada mudou: fica igual.
        assert_eq!(
            crisp_update(Some(crisp), 28.0, Vec3::splat(0.5), 2.0),
            crisp
        );
        // A animação escreveu escala nova: vira a base.
        let popped = crisp_update(Some(crisp), 28.0, Vec3::splat(1.5), 2.0);
        assert_eq!(popped.base_scale, Vec3::splat(1.5));
        assert_eq!(popped.written_scale, Vec3::splat(0.75));
        // O zoom mudou: a mesma base, fator novo.
        let zoomed = crisp_update(Some(crisp), 28.0, Vec3::splat(0.5), 4.0);
        assert_eq!(zoomed.base_size, 14.0);
        assert_eq!(zoomed.written_size, 56.0);
    }
}
