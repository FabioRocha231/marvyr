//! Bandeira Negra: o capitão declara que caça qualquer um. Içada, o tiro
//! automático mira todo navio no alcance — e todo mundo mira nele. Içar leva
//! tempo (a intenção fica à vista) e arriar exige calma (não dá para afundar
//! alguém e posar de inocente no instante seguinte).

/// Segundos para içar.
pub const HOIST_SECS: f32 = 5.0;
/// Segundos sem disparar para poder arriar.
pub const CALM_SECS: f32 = 60.0;

#[derive(Debug, Clone, Copy, Default, PartialEq)]
pub enum BlackFlag {
    #[default]
    Lowered,
    /// Subindo; `secs` restantes.
    Hoisting { secs: f32 },
    /// Içada; `calm` = segundos desde o último disparo (ou desde que subiu).
    Raised { calm: f32 },
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub enum FlagRefusal {
    /// Disparou há pouco: faltam `secs` de calma para arriar.
    NotCalm { secs: f32 },
}

impl BlackFlag {
    pub fn is_raised(self) -> bool {
        matches!(self, Self::Raised { .. })
    }

    /// Começa a içar; já subindo ou içada, nada muda.
    pub fn hoist(&mut self) {
        if *self == Self::Lowered {
            *self = Self::Hoisting { secs: HOIST_SECS };
        }
    }

    /// Arria: subindo cancela na hora; içada só depois de `CALM_SECS` sem
    /// disparar.
    pub fn lower(&mut self) -> Result<(), FlagRefusal> {
        match *self {
            Self::Raised { calm } if calm < CALM_SECS => Err(FlagRefusal::NotCalm {
                secs: CALM_SECS - calm,
            }),
            _ => {
                *self = Self::Lowered;
                Ok(())
            }
        }
    }

    /// Águas protegidas: a coroa manda arriar, sem espera.
    pub fn force_lower(&mut self) {
        *self = Self::Lowered;
    }

    /// Disparou: a calma recomeça.
    pub fn fired(&mut self) {
        if let Self::Raised { calm } = self {
            *calm = 0.0;
        }
    }

    pub fn tick(&mut self, dt: f32) {
        match self {
            Self::Hoisting { secs } => {
                *secs -= dt;
                if *secs <= 0.0 {
                    *self = Self::Raised { calm: 0.0 };
                }
            }
            Self::Raised { calm } => *calm += dt,
            Self::Lowered => {}
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn hoisting_takes_time_and_lowering_needs_calm() {
        let mut flag = BlackFlag::default();
        flag.hoist();
        flag.tick(HOIST_SECS - 0.5);
        assert!(!flag.is_raised(), "ainda subindo");
        flag.tick(1.0);
        assert!(flag.is_raised());

        flag.tick(CALM_SECS - 10.0);
        flag.fired();
        flag.tick(30.0);
        assert_eq!(
            flag.lower(),
            Err(FlagRefusal::NotCalm {
                secs: CALM_SECS - 30.0
            })
        );
        flag.tick(CALM_SECS);
        assert_eq!(flag.lower(), Ok(()));
        assert_eq!(flag, BlackFlag::Lowered);
    }

    #[test]
    fn hoisting_can_be_cancelled_and_crown_waters_force_it_down() {
        let mut flag = BlackFlag::default();
        flag.hoist();
        assert_eq!(flag.lower(), Ok(()));

        flag.hoist();
        flag.tick(HOIST_SECS);
        flag.force_lower();
        assert_eq!(flag, BlackFlag::Lowered);
    }
}
