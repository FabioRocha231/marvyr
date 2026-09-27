//! Tripulação, leme e reparo em mar (MV-061). Marujo é capacidade comprada
//! com ouro no porto (sink) e perdida no mar — afundou, a tripulação foi
//! junto (Pilar 2). Nada aqui é XP: é propriedade embarcada.

use crate::definition::ShipKind;

/// Tripulação máxima por casco.
pub fn crew_capacity(kind: ShipKind) -> u16 {
    match kind {
        ShipKind::SmallMerchant => 8,
        ShipKind::Patrol => 16,
        ShipKind::Corsair => 24,
        ShipKind::Brig => 16,
        ShipKind::Galleon => 20,
        ShipKind::Bombard => 20,
    }
}

/// Marujos de graça que todo casco novo traz (sem tripulação o jogador não
/// sairia do porto — o esqueleto mínimo não é recompensa, é piso).
pub const SKELETON_CREW: u16 = 4;

/// Mantimento por marujo contratado: madeira do armazém do porto (o soldo
/// em ouro saiu com a moeda).
pub const CREW_WAGE_ITEM: &str = "Madeira";
pub const CREW_WAGE: u32 = 3;

/// Multiplicador da recarga de bordo: tripulação cheia 1,0x; sem ninguém
/// para carregar os canhões, 1,8x.
pub fn reload_multiplier(crew: u16, capacity: u16) -> f32 {
    1.8 - 0.8 * crew_ratio(crew, capacity)
}

fn crew_ratio(crew: u16, capacity: u16) -> f32 {
    if capacity == 0 {
        return 0.0;
    }
    (f32::from(crew) / f32::from(capacity)).clamp(0.0, 1.0)
}

/// Marujos mortos por um impacto no casco: cada 12% do casco perdido custa
/// um marujo (arredondado para baixo, mínimo 0).
pub fn casualties(hull_damage: u32, max_hp: u32, crew: u16) -> u16 {
    if max_hp == 0 {
        return 0;
    }
    let lost = (hull_damage as f32 / max_hp as f32 / 0.12).floor() as u16;
    lost.min(crew)
}

pub const RUDDER_HP_MAX: f32 = 100.0;

/// Leme avariado governa menos: 30% do giro com o leme destruído.
pub fn rudder_turn_multiplier(rudder_hp: f32) -> f32 {
    0.3 + 0.7 * (rudder_hp / RUDDER_HP_MAX).clamp(0.0, 1.0)
}

/// Segundos sem levar tiro antes de o reparo poder começar.
pub const REPAIR_COMBAT_LOCK_SECS: f32 = 6.0;

/// Um ciclo de reparo (1 s de trabalho da tripulação).
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct RepairStep {
    pub hull_gain: u32,
    pub rudder_gain: f32,
    /// Unidades de Madeira consumidas do porão.
    pub timber_used: u32,
}

/// Quanto um ciclo de reparo recupera. Consome 1 Madeira por ciclo; sem
/// madeira, sem tripulação ou sem avaria, nada acontece (`None`).
/// Cada marujo recupera 0,5% do casco e 2 pontos de leme por ciclo.
pub fn repair_step(
    hp: u32,
    max_hp: u32,
    rudder_hp: f32,
    crew: u16,
    timber_available: u32,
) -> Option<RepairStep> {
    let hull_missing = max_hp.saturating_sub(hp);
    let rudder_missing = (RUDDER_HP_MAX - rudder_hp).max(0.0);
    if crew == 0 || timber_available == 0 || (hull_missing == 0 && rudder_missing <= 0.0) {
        return None;
    }
    let hull_gain = ((max_hp as f32 * 0.005 * f32::from(crew)).ceil() as u32).min(hull_missing);
    let rudder_gain = (2.0 * f32::from(crew)).min(rudder_missing);
    Some(RepairStep {
        hull_gain,
        rudder_gain,
        timber_used: 1,
    })
}

/// v59: oficial de bordo — um de cada por navio, mora junto da tripulação
/// (afundou, foi junto; abordagem vencida sobre jogador captura um).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Officer {
    /// Recarga do bordo mais rápida.
    Gunner,
    /// Leme mais firme: giro maior.
    Boatswain,
    /// Metade das baixas por impacto.
    Surgeon,
}

impl Officer {
    pub const ALL: [Officer; 3] = [Officer::Gunner, Officer::Boatswain, Officer::Surgeon];

    /// Bit na máscara de rede/banco.
    pub fn bit(self) -> u8 {
        match self {
            Officer::Gunner => 1,
            Officer::Boatswain => 2,
            Officer::Surgeon => 4,
        }
    }

    pub fn from_code(code: u8) -> Option<Officer> {
        Officer::ALL
            .into_iter()
            .find(|officer| officer.bit() == code)
    }

    pub fn name(self) -> &'static str {
        match self {
            Officer::Gunner => "Artilheiro",
            Officer::Boatswain => "Contramestre",
            Officer::Surgeon => "Cirurgião",
        }
    }

    /// Contratação: unidades do recurso do armazém (o servidor escolhe o
    /// recurso: minério, madeira e coral, na ordem).
    pub fn hire_cost(self) -> u32 {
        match self {
            Officer::Gunner | Officer::Boatswain => 20,
            Officer::Surgeon => 10,
        }
    }
}

/// Máscara de oficiais a bordo (`Officer::bit`).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct Officers(pub u8);

impl Officers {
    pub fn has(self, officer: Officer) -> bool {
        self.0 & officer.bit() != 0
    }

    pub fn add(&mut self, officer: Officer) {
        self.0 |= officer.bit();
    }

    pub fn remove(&mut self, officer: Officer) {
        self.0 &= !officer.bit();
    }

    /// Só bits conhecidos (banco velho ou corrompido não inventa oficial).
    pub fn sanitized(mask: u8) -> Self {
        Self(mask & 0b111)
    }

    /// Primeiro oficial que `victim` tem e `self` não: o que a abordagem leva.
    pub fn capturable_from(self, victim: Officers) -> Option<Officer> {
        Officer::ALL
            .into_iter()
            .find(|officer| victim.has(*officer) && !self.has(*officer))
    }

    pub fn reload_multiplier(self) -> f32 {
        if self.has(Officer::Gunner) {
            0.85
        } else {
            1.0
        }
    }

    pub fn turn_multiplier(self) -> f32 {
        if self.has(Officer::Boatswain) {
            1.15
        } else {
            1.0
        }
    }

    /// Baixas depois do cirurgião (metade, para baixo).
    pub fn treat(self, casualties: u16) -> u16 {
        if self.has(Officer::Surgeon) {
            casualties / 2
        } else {
            casualties
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn officers_mask_effects_and_capture() {
        let mut mine = Officers::default();
        assert_eq!(mine.reload_multiplier(), 1.0);
        mine.add(Officer::Gunner);
        assert!(mine.reload_multiplier() < 1.0);
        let victim = Officers(Officer::Gunner.bit() | Officer::Surgeon.bit());
        assert_eq!(mine.capturable_from(victim), Some(Officer::Surgeon));
        assert_eq!(victim.treat(5), 2);
        assert_eq!(Officers::sanitized(0xFF).0, 0b111);
        for officer in Officer::ALL {
            assert_eq!(Officer::from_code(officer.bit()), Some(officer));
        }
        mine.remove(Officer::Gunner);
        assert_eq!(mine, Officers::default());
    }

    #[test]
    fn bigger_hulls_carry_more_crew() {
        assert!(crew_capacity(ShipKind::Corsair) > crew_capacity(ShipKind::SmallMerchant));
        assert!(SKELETON_CREW <= crew_capacity(ShipKind::SmallMerchant));
    }

    #[test]
    fn full_crew_reloads_fastest() {
        assert!((reload_multiplier(8, 8) - 1.0).abs() < 1e-6);
        assert!((reload_multiplier(0, 8) - 1.8).abs() < 1e-6);
        assert!(reload_multiplier(4, 8) > 1.0);
    }

    #[test]
    fn heavy_hits_kill_sailors_but_never_more_than_aboard() {
        assert_eq!(casualties(10, 200, 8), 0);
        assert_eq!(casualties(50, 200, 8), 2);
        assert_eq!(casualties(500, 200, 3), 3);
    }

    #[test]
    fn rudder_damage_slows_the_turn() {
        assert_eq!(rudder_turn_multiplier(RUDDER_HP_MAX), 1.0);
        assert!((rudder_turn_multiplier(0.0) - 0.3).abs() < 1e-6);
    }

    #[test]
    fn repair_needs_timber_crew_and_damage() {
        let step = repair_step(100, 200, 50.0, 8, 5).unwrap();
        assert_eq!(step.hull_gain, 8);
        assert_eq!(step.rudder_gain, 16.0);
        assert_eq!(step.timber_used, 1);
        assert_eq!(repair_step(100, 200, 50.0, 8, 0), None);
        assert_eq!(repair_step(100, 200, 50.0, 0, 5), None);
        assert_eq!(repair_step(200, 200, 100.0, 8, 5), None);
        assert_eq!(repair_step(199, 200, 100.0, 8, 5).unwrap().hull_gain, 1);
    }
}
