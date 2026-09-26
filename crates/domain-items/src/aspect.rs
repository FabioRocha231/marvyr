//! Aspectos lendários (D4): um efeito único que muda como o navio luta,
//! não só quanto. Entra numa peça Rara pelo Selo do aspecto (fabricado na
//! oficina de jogador com recurso da Maré Sangrenta e da Cerração) e a
//! peça vira Lendária. Cada aspecto mora num slot só; o servidor aplica o
//! efeito, o client só descreve.

use serde::{Deserialize, Serialize};

use crate::definition::EquipmentSlot;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum AspectKind {
    /// Velas (Vento do Desespero): +25% de pano com o casco abaixo da metade.
    Tailwind,
    /// Casco: −25% de dano recebido quase parado (< 2 m/s).
    IronAnchor,
    /// Canhão: afundar um navio recarrega os dois bordos na hora.
    LightningSalvo,
    /// Canhão: cada acerto remenda 2 de casco.
    ThirstyPowder,
}

/// Vento do Desespero: multiplicador de pano e o limiar de casco.
pub const TAILWIND_SPEED: f32 = 1.25;
/// Âncora de Ferro: fração do dano que passa e a velocidade limite.
pub const IRON_ANCHOR_TAKEN: f32 = 0.75;
pub const IRON_ANCHOR_MAX_SPEED: f32 = 2.0;
/// Pólvora Sedenta: casco remendado por acerto.
pub const THIRSTY_HEAL: u32 = 2;

impl AspectKind {
    pub const ALL: [AspectKind; 4] = [
        AspectKind::Tailwind,
        AspectKind::IronAnchor,
        AspectKind::LightningSalvo,
        AspectKind::ThirstyPowder,
    ];

    pub fn name(self) -> &'static str {
        match self {
            AspectKind::Tailwind => "Vento do Desespero",
            AspectKind::IronAnchor => "Âncora de Ferro",
            AspectKind::LightningSalvo => "Salva Relâmpago",
            AspectKind::ThirstyPowder => "Pólvora Sedenta",
        }
    }

    pub fn description(self) -> &'static str {
        match self {
            AspectKind::Tailwind => "+25% de pano com o casco abaixo da metade",
            AspectKind::IronAnchor => "-25% de dano recebido quase parado",
            AspectKind::LightningSalvo => "afundar um navio recarrega os dois bordos",
            AspectKind::ThirstyPowder => "cada acerto remenda 2 de casco",
        }
    }

    /// Onde o aspecto pode morar.
    pub fn slot(self) -> EquipmentSlot {
        match self {
            AspectKind::Tailwind => EquipmentSlot::Sail,
            AspectKind::IronAnchor => EquipmentSlot::Hull,
            AspectKind::LightningSalvo | AspectKind::ThirstyPowder => EquipmentSlot::Weapon,
        }
    }

    /// Item do Selo que imprime este aspecto.
    pub fn seal_name(self) -> &'static str {
        match self {
            AspectKind::Tailwind => "Selo: Vento do Desespero",
            AspectKind::IronAnchor => "Selo: Âncora de Ferro",
            AspectKind::LightningSalvo => "Selo: Salva Relâmpago",
            AspectKind::ThirstyPowder => "Selo: Pólvora Sedenta",
        }
    }

    pub fn index(self) -> usize {
        AspectKind::ALL.iter().position(|a| *a == self).unwrap_or(0)
    }
}

/// Pano com Vento do Desespero (casco abaixo da metade).
pub fn tailwind_speed(speed: f32, hp: u32, max_hp: u32) -> f32 {
    if hp * 2 < max_hp {
        speed * TAILWIND_SPEED
    } else {
        speed
    }
}

/// Dano recebido com Âncora de Ferro (quase parado); nunca menos de 1.
pub fn iron_anchor_damage(damage: u32, speed: f32) -> u32 {
    if speed.abs() < IRON_ANCHOR_MAX_SPEED && damage > 0 {
        ((damage as f32 * IRON_ANCHOR_TAKEN).ceil() as u32).max(1)
    } else {
        damage
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_aspect_has_a_seal_a_slot_and_words() {
        for aspect in AspectKind::ALL {
            assert!(aspect.seal_name().ends_with(aspect.name()));
            assert!(!aspect.description().is_empty());
            assert_eq!(AspectKind::ALL[aspect.index()], aspect);
        }
        assert_eq!(AspectKind::Tailwind.slot(), EquipmentSlot::Sail);
    }

    #[test]
    fn conditions_gate_the_effects() {
        assert_eq!(tailwind_speed(10.0, 60, 100), 10.0);
        assert_eq!(tailwind_speed(10.0, 40, 100), 12.5);
        assert_eq!(iron_anchor_damage(8, 5.0), 8);
        assert_eq!(iron_anchor_damage(8, 1.0), 6);
        assert_eq!(iron_anchor_damage(1, 0.0), 1, "nunca zera o golpe");
    }
}
