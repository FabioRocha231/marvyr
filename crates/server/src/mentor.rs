//! v49: mentoria. Um veterano (Renome nível [`VETERAN`]+) que navega junto
//! de um novato (até [`NOVICE`]) em águas protegidas vira mentor dele: o
//! novato coleta 50% a mais enquanto estão juntos e o mentor ganha Renome a
//! cada minuto de companhia (com teto por dia). Sem mensagem nova: o aviso
//! vai pelo feed de eventos.

use std::collections::HashMap;

use bevy::prelude::*;
use lightyear::prelude::server::*;
use marvyr_domain_ships::VesselPresence;
use marvyr_domain_world::RiskTier;
use marvyr_protocol::WorldEventKind;
use marvyr_shared::ids::CharacterId;
use tracing::info;

use crate::net::{ServerShip, ServerWorldMap};
use crate::sets::SimulationSet;

pub const VETERAN: u32 = 10;
pub const NOVICE: u32 = 4;
/// Distância máxima entre os dois (m).
pub const RANGE: f32 = 300.0;
/// Minuto de companhia que rende Renome ao mentor.
const PAY_EVERY: f32 = 60.0;
pub const RENOWN_PER_MINUTE: u32 = 5;
/// Pagamentos por mentor por dia (contas-sombra não viram fazenda).
const PAYS_PER_DAY: u32 = 12;
pub const REASON: &str = "mentoria";
/// Coleta a mais que um novato ganha por dia navegando com mentor: veterano
/// e conta-sombra novata coletando juntos não viram bônus eterno.
pub const NOVICE_BONUS_PER_DAY: u32 = 150;
const CHECK_EVERY: f32 = 1.0;

#[derive(Resource, Default)]
pub struct Mentoring {
    /// novato → mentor.
    pairs: HashMap<CharacterId, CharacterId>,
    together: HashMap<(CharacterId, CharacterId), f32>,
    /// (mentor, dia) → pagamentos feitos. ponytail: em memória; restart
    /// reabre o dia (o teto segura o estrago).
    paid_today: HashMap<(CharacterId, u32), u32>,
    /// (novato, dia) → coleta a mais já dada.
    bonus_today: HashMap<(CharacterId, u32), u32>,
    clock: f32,
}

impl Mentoring {
    /// O novato está navegando com um mentor agora.
    pub fn is_mentored(&self, novice: CharacterId) -> bool {
        self.pairs.contains_key(&novice)
    }

    /// v49: metade a mais do que o novato tirou, se há mentor por perto e
    /// o teto do dia ainda deixa. ponytail: teto em memória, restart reabre.
    pub fn novice_bonus(&mut self, novice: CharacterId, taken: u32, day: u32) -> u32 {
        if !self.is_mentored(novice) {
            return 0;
        }
        self.bonus_today.retain(|(_, d), _| *d == day);
        let given = self.bonus_today.entry((novice, day)).or_default();
        let bonus = (taken / 2).min(NOVICE_BONUS_PER_DAY.saturating_sub(*given));
        *given += bonus;
        bonus
    }
}

/// Um navegante para o pareamento: quem, onde, nível, se está em água
/// protegida no mar.
#[derive(Debug, Clone, Copy)]
pub struct Sailor {
    pub character: CharacterId,
    pub at: Vec2,
    pub level: u32,
    pub eligible: bool,
}

/// Cada novato com o veterano mais perto no alcance.
pub fn pair(sailors: &[Sailor]) -> HashMap<CharacterId, CharacterId> {
    let veterans: Vec<&Sailor> = sailors
        .iter()
        .filter(|s| s.eligible && s.level >= VETERAN)
        .collect();
    sailors
        .iter()
        .filter(|s| s.eligible && s.level <= NOVICE)
        .filter_map(|novice| {
            veterans
                .iter()
                .map(|v| (v, v.at.distance(novice.at)))
                .filter(|(_, d)| *d <= RANGE)
                .min_by(|a, b| a.1.total_cmp(&b.1))
                .map(|(v, _)| (novice.character, v.character))
        })
        .collect()
}

pub fn install(app: &mut App) {
    app.init_resource::<Mentoring>().add_systems(
        FixedUpdate,
        tick_mentoring.in_set(SimulationSet::EconomyConsequences),
    );
}

// System Bevy: params são injeção de dependência, não assinatura.
#[allow(clippy::too_many_arguments)]
fn tick_mentoring(
    time: Res<Time>,
    map: Res<ServerWorldMap>,
    renown_book: Res<crate::renown::CaptainRenown>,
    ships: Query<&ServerShip>,
    mut mentoring: ResMut<Mentoring>,
    mut renown: EventWriter<crate::renown::RenownEarned>,
    mut discoveries: EventWriter<crate::progress::Discovered>,
    mut connection_manager: ResMut<ConnectionManager>,
) {
    let dt = time.delta_secs();
    mentoring.clock += dt;
    if mentoring.clock < CHECK_EVERY {
        return;
    }
    let elapsed = std::mem::take(&mut mentoring.clock);
    let sailors: Vec<Sailor> = ships
        .iter()
        .filter(|ship| ship.client_id.is_some())
        .map(|ship| Sailor {
            character: ship.character,
            at: Vec2::new(ship.motion.x, ship.motion.y),
            level: renown_book.level(ship.character),
            eligible: ship.presence == VesselPresence::AtSea
                && map
                    .0
                    .zone_at(ship.motion.x, ship.motion.y)
                    .is_ok_and(|zone| zone.tier == RiskTier::Protected),
        })
        .collect();
    let pairs = pair(&sailors);
    let client_of = |character: CharacterId| {
        ships
            .iter()
            .find(|s| s.character == character)
            .and_then(|s| s.client_id)
    };
    for (novice, mentor) in &pairs {
        if mentoring.pairs.get(novice) == Some(mentor) {
            continue;
        }
        info!(?novice, ?mentor, "mentoria começou");
        let label = crate::season::captain_label;
        if let Some(client) = client_of(*novice) {
            crate::reputation::send_event(
                &mut connection_manager,
                &[client],
                format!("Navegando com um mentor ({}): coleta +50%", label(*mentor)),
                WorldEventKind::Kill,
            );
        }
        if let Some(client) = client_of(*mentor) {
            crate::reputation::send_event(
                &mut connection_manager,
                &[client],
                format!("Você guia {}: Renome a cada minuto juntos", label(*novice)),
                WorldEventKind::Kill,
            );
        }
        discoveries.send(crate::progress::Discovered {
            character: *mentor,
            entry: "Mentoria",
        });
    }
    let (day, _) = crate::progress::today();
    mentoring.paid_today.retain(|(_, d), _| *d == day);
    mentoring
        .together
        .retain(|(mentor, novice), _| pairs.get(novice) == Some(mentor));
    for (novice, mentor) in &pairs {
        let clock = mentoring.together.entry((*mentor, *novice)).or_default();
        *clock += elapsed;
        if *clock < PAY_EVERY {
            continue;
        }
        *clock -= PAY_EVERY;
        let paid = mentoring.paid_today.entry((*mentor, day)).or_default();
        if *paid >= PAYS_PER_DAY {
            continue;
        }
        *paid += 1;
        renown.send(crate::renown::RenownEarned {
            character: *mentor,
            amount: RENOWN_PER_MINUTE,
            reason: REASON,
        });
    }
    mentoring.pairs = pairs;
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sailor(level: u32, x: f32) -> Sailor {
        Sailor {
            character: CharacterId::new(),
            at: Vec2::new(x, 0.0),
            level,
            eligible: true,
        }
    }

    #[test]
    fn the_novice_bonus_stops_at_the_daily_cap() {
        let novice = CharacterId::new();
        let mut mentoring = Mentoring::default();
        assert_eq!(
            mentoring.novice_bonus(novice, 100, 1),
            0,
            "sem mentor, nada"
        );
        mentoring.pairs.insert(novice, CharacterId::new());
        let mut total = 0;
        for _ in 0..10 {
            total += mentoring.novice_bonus(novice, 100, 1);
        }
        assert_eq!(total, NOVICE_BONUS_PER_DAY);
        assert_eq!(mentoring.novice_bonus(novice, 100, 2), 50, "dia novo");
    }

    #[test]
    fn a_novice_pairs_with_the_nearest_veteran_in_range() {
        let novice = sailor(1, 0.0);
        let near = sailor(12, 100.0);
        let far = sailor(20, 250.0);
        let pairs = pair(&[novice, near, far]);
        assert_eq!(pairs.get(&novice.character), Some(&near.character));
        assert_eq!(pairs.len(), 1);
    }

    #[test]
    fn out_of_range_middle_levels_and_lawless_waters_do_not_pair() {
        let novice = sailor(1, 0.0);
        assert!(pair(&[novice, sailor(12, RANGE + 1.0)]).is_empty());
        assert!(pair(&[novice, sailor(7, 10.0)]).is_empty());
        let mut lawless = sailor(12, 10.0);
        lawless.eligible = false;
        assert!(pair(&[novice, lawless]).is_empty());
    }
}
