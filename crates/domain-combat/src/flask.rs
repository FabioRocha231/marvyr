//! Frascos (estilo PoE): quatro frascos de bordo, cada um com cargas. Usar
//! gasta uma dose e liga o efeito por alguns segundos; acertar canhão e
//! afundar navio enchem as cargas, atracar enche tudo. O frasco é item
//! fabricado que viaja no porão — sem ele a bordo, não há o que beber, e no
//! naufrágio ele vai para o saque como qualquer carga.

use marvyr_shared::ids::ItemDefinitionId;
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum FlaskKind {
    /// Estopa e piche: remenda o casco aos poucos.
    Repair,
    /// Vento engarrafado: mais pano.
    Wind,
    /// Pólvora fina: tiro mais forte e mais rápido.
    Fury,
    /// Breu no costado: o golpe entra menos.
    Tar,
}

/// Cargas cheias; cada dose gasta [`DOSE`] (três doses por frasco).
pub const MAX_CHARGES: u8 = 60;
pub const DOSE: u8 = 20;
/// Acerto de canhão e naufrágio causado enchem todos os frascos a bordo.
pub const CHARGES_PER_HIT: u8 = 3;
pub const CHARGES_PER_SINK: u8 = 15;

impl FlaskKind {
    /// Ordem das teclas 1-4 e da linha de frascos do atlas.
    pub const ALL: [FlaskKind; 4] = [
        FlaskKind::Repair,
        FlaskKind::Wind,
        FlaskKind::Fury,
        FlaskKind::Tar,
    ];

    pub fn index(self) -> usize {
        self as usize
    }

    pub fn item_name(self) -> &'static str {
        match self {
            FlaskKind::Repair => "Frasco de Estopa",
            FlaskKind::Wind => "Frasco de Vento",
            FlaskKind::Fury => "Frasco de Fúria",
            FlaskKind::Tar => "Frasco de Breu",
        }
    }

    pub fn item_id(self) -> ItemDefinitionId {
        ItemDefinitionId::stable(self.item_name())
    }

    pub fn duration_secs(self) -> f32 {
        match self {
            FlaskKind::Repair => 4.0,
            FlaskKind::Wind => 5.0,
            FlaskKind::Fury => 6.0,
            FlaskKind::Tar => 5.0,
        }
    }
}

/// Casco remendado pela Estopa ao longo do efeito, fração do máximo.
pub const REPAIR_FRACTION: f32 = 0.30;
pub const WIND_SPEED: f32 = 1.30;
pub const FURY_DAMAGE: f32 = 1.25;
pub const FURY_RELOAD: f32 = 0.80;
pub const TAR_INCOMING: f32 = 0.70;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FlaskRefusal {
    /// Menos de uma dose.
    Empty,
    /// Já está fazendo efeito (beber de novo não soma).
    Active,
}

/// Cargas e efeitos ativos dos quatro frascos de um navio. Não persiste:
/// o navio nasce e atraca de frascos cheios.
#[derive(Debug, Clone, PartialEq)]
pub struct FlaskBelt {
    charges: [u8; 4],
    remaining: [f32; 4],
    /// Fração de casco remendada que ainda não fechou um ponto.
    heal_carry: f32,
}

impl Default for FlaskBelt {
    fn default() -> Self {
        Self {
            charges: [MAX_CHARGES; 4],
            remaining: [0.0; 4],
            heal_carry: 0.0,
        }
    }
}

impl FlaskBelt {
    pub fn charges(&self) -> [u8; 4] {
        self.charges
    }

    pub fn is_active(&self, kind: FlaskKind) -> bool {
        self.remaining[kind.index()] > 0.0
    }

    /// Bit `i` ligado = frasco `FlaskKind::ALL[i]` fazendo efeito.
    pub fn active_mask(&self) -> u8 {
        FlaskKind::ALL
            .iter()
            .filter(|kind| self.is_active(**kind))
            .fold(0, |mask, kind| mask | 1 << kind.index())
    }

    pub fn drink(&mut self, kind: FlaskKind) -> Result<(), FlaskRefusal> {
        let i = kind.index();
        if self.remaining[i] > 0.0 {
            return Err(FlaskRefusal::Active);
        }
        if self.charges[i] < DOSE {
            return Err(FlaskRefusal::Empty);
        }
        self.charges[i] -= DOSE;
        self.remaining[i] = kind.duration_secs();
        Ok(())
    }

    /// Avança os efeitos; devolve os pontos de casco que a Estopa remenda
    /// agora (o resto fracionado fica para o próximo tick).
    pub fn tick(&mut self, dt: f32, max_hp: u32) -> u32 {
        // Só o tempo de efeito que ainda restava conta: o total fecha certo.
        let repair_secs = dt.min(self.remaining[FlaskKind::Repair.index()]);
        for remaining in &mut self.remaining {
            *remaining = (*remaining - dt).max(0.0);
        }
        self.heal_carry +=
            max_hp as f32 * REPAIR_FRACTION * repair_secs / FlaskKind::Repair.duration_secs();
        // Folga de arredondamento do f32: 59,9999 conta como 60.
        let whole = (self.heal_carry + 1e-3).floor();
        self.heal_carry = (self.heal_carry - whole).max(0.0);
        whole as u32
    }

    pub fn fill(&mut self, amount: u8) {
        for charges in &mut self.charges {
            *charges = charges.saturating_add(amount).min(MAX_CHARGES);
        }
    }

    /// Porto: frascos cheios.
    pub fn refill(&mut self) {
        self.charges = [MAX_CHARGES; 4];
    }

    pub fn speed_multiplier(&self) -> f32 {
        if self.is_active(FlaskKind::Wind) {
            WIND_SPEED
        } else {
            1.0
        }
    }

    pub fn damage_multiplier(&self) -> f32 {
        if self.is_active(FlaskKind::Fury) {
            FURY_DAMAGE
        } else {
            1.0
        }
    }

    pub fn reload_multiplier(&self) -> f32 {
        if self.is_active(FlaskKind::Fury) {
            FURY_RELOAD
        } else {
            1.0
        }
    }

    /// Dano que entra no casco com o Breu ligado (arredonda para cima: o
    /// golpe nunca some).
    pub fn incoming(&self, damage: u32) -> u32 {
        if self.is_active(FlaskKind::Tar) && damage > 0 {
            ((damage as f32 * TAR_INCOMING).ceil() as u32).max(1)
        } else {
            damage
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn three_doses_then_empty_and_no_double_drink() {
        let mut belt = FlaskBelt::default();
        belt.drink(FlaskKind::Fury).unwrap();
        assert_eq!(belt.drink(FlaskKind::Fury), Err(FlaskRefusal::Active));
        assert_eq!(belt.damage_multiplier(), FURY_DAMAGE);
        assert_eq!(belt.active_mask(), 0b0100);
        belt.tick(FlaskKind::Fury.duration_secs(), 100);
        assert_eq!(belt.damage_multiplier(), 1.0);
        belt.drink(FlaskKind::Fury).unwrap();
        belt.tick(10.0, 100);
        belt.drink(FlaskKind::Fury).unwrap();
        belt.tick(10.0, 100);
        assert_eq!(belt.drink(FlaskKind::Fury), Err(FlaskRefusal::Empty));
        assert_eq!(belt.charges()[FlaskKind::Wind.index()], MAX_CHARGES);
    }

    #[test]
    fn hits_fill_the_belt_and_port_tops_it_up() {
        let mut belt = FlaskBelt::default();
        for _ in 0..3 {
            belt.drink(FlaskKind::Tar).unwrap();
            belt.tick(10.0, 100);
        }
        belt.fill(CHARGES_PER_HIT);
        assert_eq!(belt.charges()[FlaskKind::Tar.index()], CHARGES_PER_HIT);
        belt.fill(200);
        assert_eq!(belt.charges(), [MAX_CHARGES; 4], "nunca passa do cheio");
        belt.drink(FlaskKind::Repair).unwrap();
        belt.refill();
        assert_eq!(belt.charges(), [MAX_CHARGES; 4]);
    }

    #[test]
    fn repair_heals_its_fraction_over_the_effect() {
        let mut belt = FlaskBelt::default();
        belt.drink(FlaskKind::Repair).unwrap();
        let healed: u32 = (0..40).map(|_| belt.tick(0.1, 200)).sum();
        assert_eq!(healed, 60);
        assert_eq!(belt.tick(0.1, 200), 0);
    }

    #[test]
    fn tar_softens_but_never_erases_a_hit() {
        let mut belt = FlaskBelt::default();
        assert_eq!(belt.incoming(10), 10);
        belt.drink(FlaskKind::Tar).unwrap();
        assert_eq!(belt.incoming(10), 7);
        assert_eq!(belt.incoming(1), 1);
        assert_eq!(belt.incoming(0), 0);
    }
}
