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
    /// v38: Carga Amaldiçoada entregue num porto.
    CursedCargo,
    /// v39: peixes fisgados.
    Fish(u32),
    /// v40: camada do Abismo vencida (a profundidade).
    AbyssDepth(u32),
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
    CursedCargo,
    Fish,
    AbyssDepth,
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
            GoalKind::CursedCargo => "Entregue {0} Cargas Amaldiçoadas",
            GoalKind::Fish => "Pesque {0} peixes",
            GoalKind::AbyssDepth => "Vença a camada {0} do Abismo",
        }
    }

    /// Quanto o feito avança esta meta.
    fn step(self, deed: &Deed) -> u32 {
        match (self, deed) {
            (GoalKind::SinkShips, Deed::Sink { .. }) => 1,
            (GoalKind::SinkElites, Deed::Sink { elite: true }) => 1,
            (GoalKind::Gather, Deed::Gather(n)) | (GoalKind::Fish, Deed::Fish(n)) => *n,
            (GoalKind::Craft, Deed::Craft)
            | (GoalKind::Contract, Deed::Contract)
            | (GoalKind::LootWrecks, Deed::Loot)
            | (GoalKind::BloodChest, Deed::BloodChest)
            | (GoalKind::BossSlain, Deed::BossSlain)
            | (GoalKind::CursedCargo, Deed::CursedCargo) => 1,
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

const DAILY_POOL: [Goal; 9] = [
    goal(GoalKind::SinkShips, 8, "Minério", 30, 60),
    goal(GoalKind::SinkElites, 3, "Coral Negro", 10, 90),
    goal(GoalKind::Gather, 60, "Madeira", 40, 50),
    goal(GoalKind::Craft, 2, "Minério", 25, 50),
    goal(GoalKind::Contract, 1, "Coral Negro", 8, 60),
    goal(GoalKind::LootWrecks, 3, "Madeira", 30, 50),
    goal(GoalKind::BloodChest, 1, "Pérola Abissal", 2, 90),
    goal(GoalKind::Fish, 15, "Coral Negro", 8, 50),
    goal(GoalKind::AbyssDepth, 3, "Pérola Abissal", 3, 80),
];

const WEEKLY_POOL: [Goal; 5] = [
    goal(GoalKind::SinkElites, 25, "Pérola Abissal", 6, 400),
    goal(GoalKind::BossSlain, 1, "Âmbar Abissal", 6, 400),
    goal(GoalKind::Contract, 8, "Pérola Abissal", 5, 350),
    goal(GoalKind::CursedCargo, 2, "Âmbar Abissal", 8, 450),
    goal(GoalKind::AbyssDepth, 8, "Cristal da Cerração", 2, 500),
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

/// v42: maestria de casco — experiência (o Renome ganho com aquele casco)
/// para chegar a cada nível; o 5 é o máximo e rende o título de mestre.
pub const MASTERY_XP: [u32; 5] = [300, 1_000, 2_500, 5_000, 9_000];
pub const MASTERY_MAX: u32 = MASTERY_XP.len() as u32;

pub fn mastery_level(xp: u32) -> u32 {
    MASTERY_XP.iter().filter(|need| xp >= **need).count() as u32
}

/// Experiência do próximo nível (`None` no máximo).
pub fn mastery_next(xp: u32) -> Option<u32> {
    MASTERY_XP.iter().copied().find(|need| xp < *need)
}

/// Uma página do Livro de Bordo: completa, rende o título (catálogo
/// `TITLES` de `domain-ships`).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Page {
    pub name: &'static str,
    pub entries: &'static [&'static str],
    pub title: &'static str,
}

/// v41: Livro de Bordo — coleção do capitão. Página completa rende título.
pub const PAGES: [Page; 3] = [
    Page {
        name: "Travessias",
        entries: &[
            "Cerração",
            "Sorvedouro",
            "Ilha oculta",
            "Tesouro desenterrado",
            "Baú Maldito",
            "Contrato entregue",
        ],
        title: "o Andarilho da Névoa",
    },
    Page {
        name: "Façanhas",
        entries: &[
            "Veio dourado",
            "Peixe-Lanterna",
            "Carga Amaldiçoada entregue",
            "Camada 5 do Abismo",
            "Fúria do Mar no máximo",
            "Aspecto lendário",
        ],
        title: "a Lenda do Porto",
    },
    Page {
        name: "Bestiário",
        entries: &[
            "Corsário",
            "Navio da Marinha",
            "Mercador",
            "Galeão do Tesouro",
            "Guardião do Tesouro",
            "Saqueador da Maré",
            "Kraken",
            "Leviatã",
        ],
        title: "o Terror dos Mares",
    },
];

/// Entrada conhecida do Livro (o nome estável, com `'static`).
pub fn entry(name: &str) -> Option<&'static str> {
    PAGES
        .iter()
        .flat_map(|page| page.entries.iter())
        .find(|entry| **entry == name)
        .copied()
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
    /// v40: recorde de profundidade no Abismo.
    pub abyss_best: u32,
    /// v41: entradas do Livro de Bordo já registradas.
    pub found: std::collections::BTreeSet<String>,
    /// v42: experiência de maestria por casco (nome do casco).
    pub mastery: std::collections::BTreeMap<String, u32>,
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
        if let Deed::AbyssDepth(depth) = deed {
            self.abyss_best = self.abyss_best.max(*depth);
        }
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

    /// Soma maestria no casco; devolve o nível novo se subiu.
    pub fn add_mastery(&mut self, hull: &str, xp: u32) -> Option<u32> {
        let total = self.mastery.entry(hull.to_owned()).or_default();
        let before = mastery_level(*total);
        *total = total.saturating_add(xp);
        let after = mastery_level(*total);
        (after > before).then_some(after)
    }

    /// Registra a entrada do Livro; true se é nova.
    pub fn discover(&mut self, entry: &'static str) -> bool {
        self.found.insert(entry.to_owned())
    }

    /// Páginas completas, na ordem do Livro.
    pub fn completed_pages(&self) -> impl Iterator<Item = &'static Page> + '_ {
        PAGES
            .iter()
            .filter(|page| page.entries.iter().all(|e| self.found.contains(*e)))
    }

    fn owe(&mut self, item: &str, quantity: u32) {
        match self.unpaid.iter_mut().find(|(name, _)| name == item) {
            Some((_, owed)) => *owed += quantity,
            None => self.unpaid.push((item.to_owned(), quantity)),
        }
    }
}

/// Avança `count`; true só na passagem pelo alvo (paga uma vez). A meta de
/// profundidade guarda o recorde em vez de somar.
fn advance(goal: &Goal, count: &mut u32, deed: &Deed) -> bool {
    if *count >= goal.target {
        return false;
    }
    let next = match (goal.kind, deed) {
        (GoalKind::AbyssDepth, Deed::AbyssDepth(depth)) => (*count).max(*depth),
        _ => *count + goal.kind.step(deed),
    };
    if next == *count {
        return false;
    }
    *count = next.min(goal.target);
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
            GoalKind::CursedCargo => Deed::CursedCargo,
            GoalKind::Fish => Deed::Fish(1),
            GoalKind::AbyssDepth => Deed::AbyssDepth(goal.target),
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
    fn mastery_climbs_five_levels_and_reports_each() {
        let mut progress = CaptainProgress::default();
        assert_eq!(progress.add_mastery("Mercante", 299), None);
        assert_eq!(progress.add_mastery("Mercante", 1), Some(1));
        assert_eq!(progress.add_mastery("Mercante", 50_000), Some(MASTERY_MAX));
        assert_eq!(mastery_next(50_300), None);
        assert_eq!(mastery_next(0), Some(300));
        assert_eq!(progress.mastery.get("Patrulha"), None, "cada casco é um");
    }

    #[test]
    fn a_full_page_unlocks_its_title() {
        let mut progress = CaptainProgress::default();
        let page = &PAGES[0];
        for entry in &page.entries[1..] {
            assert!(progress.discover(entry));
        }
        assert!(!progress.discover(page.entries[1]), "repetida não é nova");
        assert_eq!(progress.completed_pages().count(), 0);
        progress.discover(page.entries[0]);
        assert_eq!(progress.completed_pages().next(), Some(page));
        assert_eq!(entry("Kraken"), Some("Kraken"));
        assert_eq!(entry("Sereia"), None);
    }

    #[test]
    fn abyss_goal_keeps_the_record_depth() {
        let goal = goal(GoalKind::AbyssDepth, 3, "Pérola Abissal", 3, 80);
        let mut count = 0;
        assert!(!advance(&goal, &mut count, &Deed::AbyssDepth(2)));
        assert!(
            !advance(&goal, &mut count, &Deed::AbyssDepth(1)),
            "recorde não desce"
        );
        assert_eq!(count, 2);
        assert!(advance(&goal, &mut count, &Deed::AbyssDepth(4)));
        assert_eq!(count, 3);
        let mut progress = CaptainProgress::default();
        progress.record(&Deed::AbyssDepth(5), 0, 0);
        progress.record(&Deed::AbyssDepth(2), 0, 0);
        assert_eq!(progress.abyss_best, 5);
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
