//! Diário de Bordo: três metas por dia e uma grande por semana. As metas
//! saem do relógio (dia/semana UTC), iguais para todo o servidor — "hoje é
//! dia de caçar elite" vira assunto no porto. Cada feito do capitão é um
//! [`Deed`]; a meta cumprida paga recurso bruto (no próximo porto) e Renome.
//! Não há sequência de login: perder um dia não tira nada.

use serde::{Deserialize, Serialize};

/// Um feito no mar, contado pelo Diário.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Deed {
    /// Navio NPC afundado; `elite` = pirata de elite, Saqueador ou chefe.
    Sink {
        elite: bool,
    },
    Gather(u32),
    Craft,
    Contract,
    Loot,
    BloodChest,
    BossSlain,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum GoalKind {
    SinkShips,
    SinkElites,
    Gather,
    Craft,
    Contract,
    LootWrecks,
    BloodChest,
    BossSlain,
}

impl GoalKind {
    /// Modelo PT-BR (`{0}` = alvo). O client traduz com `trf`.
    pub fn template(self) -> &'static str {
        match self {
            GoalKind::SinkShips => "Afunde {0} navios",
            GoalKind::SinkElites => "Afunde {0} elites",
            GoalKind::Gather => "Colete {0} recursos",
            GoalKind::Craft => "Fabrique {0} peças",
            GoalKind::Contract => "Entregue {0} contratos",
            GoalKind::LootWrecks => "Saqueie {0} destroços",
            GoalKind::BloodChest => "Abra {0} Baús Malditos",
            GoalKind::BossSlain => "Afunde o Leviatã {0} vez",
        }
    }

    /// Quanto o feito avança esta meta.
    fn step(self, deed: &Deed) -> u32 {
        match (self, deed) {
            (GoalKind::SinkShips, Deed::Sink { .. }) => 1,
            (GoalKind::SinkElites, Deed::Sink { elite: true }) => 1,
            (GoalKind::Gather, Deed::Gather(n)) => *n,
            (GoalKind::Craft, Deed::Craft)
            | (GoalKind::Contract, Deed::Contract)
            | (GoalKind::LootWrecks, Deed::Loot)
            | (GoalKind::BloodChest, Deed::BloodChest)
            | (GoalKind::BossSlain, Deed::BossSlain) => 1,
            _ => 0,
        }
    }
}

/// Uma meta: o que fazer e quanto paga (recurso bruto pelo nome do
/// catálogo — Pilar 1 — e Renome).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Goal {
    pub kind: GoalKind,
    pub target: u32,
    pub reward_item: &'static str,
    pub reward_quantity: u32,
    pub renown: u32,
}

const fn goal(kind: GoalKind, target: u32, item: &'static str, quantity: u32, renown: u32) -> Goal {
    Goal {
        kind,
        target,
        reward_item: item,
        reward_quantity: quantity,
        renown,
    }
}

const DAILY_POOL: [Goal; 7] = [
    goal(GoalKind::SinkShips, 8, "Minério", 30, 60),
    goal(GoalKind::SinkElites, 3, "Coral Negro", 10, 90),
    goal(GoalKind::Gather, 60, "Madeira", 40, 50),
    goal(GoalKind::Craft, 2, "Minério", 25, 50),
    goal(GoalKind::Contract, 1, "Coral Negro", 8, 60),
    goal(GoalKind::LootWrecks, 3, "Madeira", 30, 50),
    goal(GoalKind::BloodChest, 1, "Pérola Abissal", 2, 90),
];

const WEEKLY_POOL: [Goal; 3] = [
    goal(GoalKind::SinkElites, 25, "Pérola Abissal", 6, 400),
    goal(GoalKind::BossSlain, 1, "Âmbar Abissal", 6, 400),
    goal(GoalKind::Contract, 8, "Pérola Abissal", 5, 350),
];

pub const DAILY_GOALS: usize = 3;

fn splitmix(mut x: u64) -> u64 {
    x = x.wrapping_add(0x9E37_79B9_7F4A_7C15);
    x = (x ^ (x >> 30)).wrapping_mul(0xBF58_476D_1CE4_E5B9);
    x = (x ^ (x >> 27)).wrapping_mul(0x94D0_49BB_1331_11EB);
    x ^ (x >> 31)
}

/// As três metas do dia (distintas, iguais para todo mundo).
pub fn daily_goals(day: u32) -> [Goal; DAILY_GOALS] {
    let mut pool: Vec<Goal> = DAILY_POOL.to_vec();
    let mut seed = u64::from(day);
    std::array::from_fn(|_| {
        seed = splitmix(seed);
        pool.remove((seed % pool.len() as u64) as usize)
    })
}

pub fn weekly_goal(week: u32) -> Goal {
    WEEKLY_POOL[(splitmix(u64::from(week) ^ 0x5745_454B) % WEEKLY_POOL.len() as u64) as usize]
}

/// Dia e semana (UTC, semana começa na segunda) de um instante Unix.
pub fn clock(unix_secs: u64) -> (u32, u32) {
    let day = (unix_secs / 86_400) as u32;
    // 1970-01-01 foi quinta: +3 alinha a virada na segunda.
    (day, (day + 3) / 7)
}

/// O que o capitão já fez no Diário. Persistido (JSON) junto do personagem;
/// campos novos entram com `default`.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct CaptainProgress {
    pub day: u32,
    pub daily: [u32; DAILY_GOALS],
    pub week: u32,
    pub weekly: u32,
    /// Recompensas cumpridas esperando o próximo porto: (item, quantidade).
    pub unpaid: Vec<(String, u32)>,
}

impl CaptainProgress {
    /// Virou o dia ou a semana: as metas velhas somem (o que já foi pago
    /// fica pago).
    pub fn roll(&mut self, day: u32, week: u32) {
        if self.day != day {
            self.day = day;
            self.daily = [0; DAILY_GOALS];
        }
        if self.week != week {
            self.week = week;
            self.weekly = 0;
        }
    }

    /// Conta o feito; devolve as metas que se cumpriram AGORA (a recompensa
    /// já entrou em `unpaid`).
    pub fn record(&mut self, deed: &Deed, day: u32, week: u32) -> Vec<Goal> {
        self.roll(day, week);
        let mut done = Vec::new();
        for (goal, count) in daily_goals(day).iter().zip(self.daily.iter_mut()) {
            if advance(goal, count, deed) {
                done.push(*goal);
            }
        }
        let weekly = weekly_goal(week);
        if advance(&weekly, &mut self.weekly, deed) {
            done.push(weekly);
        }
        for goal in &done {
            self.owe(goal.reward_item, goal.reward_quantity);
        }
        done
    }

    fn owe(&mut self, item: &str, quantity: u32) {
        match self.unpaid.iter_mut().find(|(name, _)| name == item) {
            Some((_, owed)) => *owed += quantity,
            None => self.unpaid.push((item.to_owned(), quantity)),
        }
    }
}

/// Avança `count`; true só na passagem pelo alvo (paga uma vez).
fn advance(goal: &Goal, count: &mut u32, deed: &Deed) -> bool {
    let step = goal.kind.step(deed);
    if step == 0 || *count >= goal.target {
        return false;
    }
    *count = (*count + step).min(goal.target);
    *count == goal.target
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn daily_goals_are_distinct_and_stable_per_day() {
        for day in 0..200 {
            let goals = daily_goals(day);
            assert_eq!(goals, daily_goals(day));
            assert_ne!(goals[0].kind, goals[1].kind);
            assert_ne!(goals[1].kind, goals[2].kind);
            assert_ne!(goals[0].kind, goals[2].kind);
        }
        assert_ne!(
            (0..30).map(daily_goals).collect::<Vec<_>>()[0],
            daily_goals(1),
            "o dia muda as metas"
        );
    }

    #[test]
    fn a_goal_pays_once_and_resets_with_the_day() {
        let day = 20_000;
        let week = clock(u64::from(day) * 86_400).1;
        let goal = daily_goals(day)[0];
        let deed = match goal.kind {
            GoalKind::SinkShips | GoalKind::SinkElites => Deed::Sink { elite: true },
            GoalKind::Gather => Deed::Gather(1),
            GoalKind::Craft => Deed::Craft,
            GoalKind::Contract => Deed::Contract,
            GoalKind::LootWrecks => Deed::Loot,
            GoalKind::BloodChest => Deed::BloodChest,
            GoalKind::BossSlain => Deed::BossSlain,
        };
        let mut progress = CaptainProgress::default();
        let mut paid = 0;
        for _ in 0..goal.target * 2 {
            paid += progress
                .record(&deed, day, week)
                .iter()
                .filter(|g| **g == goal)
                .count();
        }
        assert_eq!(paid, 1, "meta cumprida paga uma vez");
        assert!(progress
            .unpaid
            .iter()
            .any(|(item, q)| item == goal.reward_item && *q >= goal.reward_quantity));
        progress.record(&Deed::Craft, day + 1, week);
        assert!(progress.daily.iter().all(|c| *c <= 1), "dia novo zera");
    }

    #[test]
    fn gather_counts_units_and_week_starts_on_monday() {
        let mut progress = CaptainProgress::default();
        let day = (0..)
            .find(|d| daily_goals(*d).iter().any(|g| g.kind == GoalKind::Gather))
            .unwrap();
        let slot = daily_goals(day)
            .iter()
            .position(|g| g.kind == GoalKind::Gather)
            .unwrap();
        progress.record(&Deed::Gather(25), day, 0);
        assert_eq!(progress.daily[slot], 25);
        // 1970-01-05 foi segunda.
        assert_eq!(clock(4 * 86_400).1, 1);
        assert_eq!(clock(3 * 86_400).1, 0);
    }
}
