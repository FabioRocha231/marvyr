//! v35: Diário de Bordo no servidor. As regras vivem em
//! `domain-economy::logbook`; aqui: carrega e grava a progressão do capitão,
//! conta os feitos e paga as metas cumpridas no próximo porto.
//!
//! Os feitos chegam pelo `RenownEarned` que o jogo já dispara em todo lugar
//! que rende Renome — o motivo diz o que foi feito (`deed_of`).

use std::collections::HashMap;
use std::time::{SystemTime, UNIX_EPOCH};

use bevy::ecs::event::EventCursor;
use bevy::prelude::*;
use lightyear::prelude::server::*;
use lightyear::prelude::*;
use marvyr_domain_economy::logbook::{
    clock, daily_goals, mastery_level, season_of, weekly_goal, CaptainProgress, Deed, Goal,
    MASTERY_MAX,
};
use marvyr_domain_ships::VesselPresence;
use marvyr_protocol::{GoalLine, ProgressSnapshot, WorldEventKind};
use marvyr_shared::ids::{CharacterId, ItemDefinitionId};
use tracing::{info, warn};

use crate::market::ServerMarket;
use crate::net::{DevItems, ReliableChannel, ServerShip};
use crate::persist::StoreHandle;
use crate::renown::RenownEarned;
use crate::sets::SimulationSet;

/// v41: entrada nova (ou não) do Livro de Bordo. Quem vive o feito
/// dispara; aqui vira coleção e, página completa, título.
#[derive(Event, Debug, Clone, Copy)]
pub struct Discovered {
    pub character: CharacterId,
    pub entry: &'static str,
}

/// Motivo de Renome da meta cumprida (não conta como feito).
pub const GOAL_REASON: &str = "meta do Diário";

#[derive(Debug, Clone, Default)]
struct Captain {
    progress: CaptainProgress,
    client: Option<ClientId>,
    /// O banco não respondeu no connect: nada muda nem grava (MV-067).
    is_unread: bool,
    is_dirty: bool,
}

#[derive(Resource, Default)]
pub struct CaptainLogbook {
    captains: HashMap<CharacterId, Captain>,
    save_clock: f32,
}

impl CaptainLogbook {
    pub fn progress(&self, character: CharacterId) -> Option<&CaptainProgress> {
        self.captains.get(&character).map(|c| &c.progress)
    }

    /// v44: recompensa fora das metas (cabeça cobrada) — entra na mesma
    /// dívida que o porto paga. `false` se o Diário do capitão não foi
    /// lido (não se grava por cima do banco, MV-067).
    pub fn owe(
        &mut self,
        connection_manager: &mut ConnectionManager,
        character: CharacterId,
        items: &[(&str, u32)],
    ) -> bool {
        let Some(captain) = self.captains.get_mut(&character) else {
            return false;
        };
        if captain.is_unread {
            return false;
        }
        for (item, quantity) in items {
            captain.progress.owe(item, *quantity);
        }
        captain.is_dirty = true;
        send(connection_manager, captain.client, &captain.progress);
        true
    }

    /// Capitães da sessão com a progressão lida do banco.
    pub fn captains(&self) -> impl Iterator<Item = (CharacterId, &CaptainProgress)> {
        self.captains
            .iter()
            .filter(|(_, c)| !c.is_unread)
            .map(|(id, c)| (*id, &c.progress))
    }

    /// v41: título à mostra — o da página mais alta completa do Livro.
    /// v42: o melhor título conquistado — página do Livro ou maestria
    /// de casco no máximo (o de código mais alto).
    pub fn title(&self, character: CharacterId) -> u8 {
        let Some(progress) = self.progress(character) else {
            return 0;
        };
        let pages = progress.completed_pages().map(|page| page.title);
        let masters = marvyr_domain_ships::ShipKind::ALL
            .into_iter()
            .filter(|kind| {
                progress
                    .mastery
                    .get(kind.name())
                    .is_some_and(|xp| mastery_level(*xp) == MASTERY_MAX)
            })
            .map(|kind| kind.master_title());
        let crown = (progress.crowns > 0).then_some("Coroa da Maré");
        pages
            .chain(masters)
            .chain(crown)
            .filter_map(marvyr_domain_ships::title_code)
            .max()
            .unwrap_or(0)
    }
}

/// ponytail: grava a cada 5 s, como o Renome — crash perde no máximo isso.
const SAVE_EVERY: f32 = 5.0;

/// Relógio do Diário: agora, ou `MARVYR_DAY=<n>` em dev.
pub fn today() -> (u32, u32) {
    let now = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_or(0, |d| d.as_secs());
    match std::env::var("MARVYR_DAY")
        .ok()
        .and_then(|d| d.parse::<u64>().ok())
    {
        Some(day) => clock(day * 86_400),
        None => clock(now),
    }
}

/// v43: tema da temporada em curso (mexe em veio dourado, pesca e Abismo).
pub fn theme() -> marvyr_domain_economy::logbook::SeasonTheme {
    marvyr_domain_economy::logbook::season_of(today().1).1
}

/// ponytail: o feito sai do motivo do Renome — um lugar só a manter. Motivo
/// novo que deva contar entra aqui.
pub fn deed_of(earned: &RenownEarned) -> Option<Deed> {
    Some(match earned.reason {
        "navio afundado" => Deed::Sink { elite: false },
        "elite afundado" => Deed::Sink { elite: true },
        "Leviatã afundado" => Deed::BossSlain,
        "coleta" => Deed::Gather(earned.amount / marvyr_domain_economy::renown::PER_GATHERED_UNIT),
        "fabricação" | "navio construído" => Deed::Craft,
        "contrato entregue" => Deed::Contract,
        "destroço saqueado" => Deed::Loot,
        "Baú Maldito" => Deed::BloodChest,
        crate::cursed_cargo::DELIVERY_REASON => Deed::CursedCargo,
        crate::abyss::REASON => Deed::AbyssDepth(crate::abyss::depth_of(earned.amount)),
        crate::fishing::REASON => Deed::Fish(earned.amount / crate::fishing::RENOWN_PER_FISH),
        _ => return None,
    })
}

/// Entrada do Livro que o motivo do Renome já conta.
fn entry_of(earned: &RenownEarned) -> Option<&'static str> {
    match earned.reason {
        "Baú Maldito" => Some("Baú Maldito"),
        "contrato entregue" => Some("Contrato entregue"),
        crate::cursed_cargo::DELIVERY_REASON => Some("Carga Amaldiçoada entregue"),
        crate::abyss::REASON if crate::abyss::depth_of(earned.amount) >= 5 => {
            Some("Camada 5 do Abismo")
        }
        _ => None,
    }
}

pub fn install(app: &mut App) {
    app.init_resource::<CaptainLogbook>()
        .add_event::<Discovered>();
    app.add_systems(
        FixedUpdate,
        (
            load_on_connect,
            record_deeds,
            record_discoveries,
            pay_on_dock,
        )
            .chain()
            .in_set(SimulationSet::Telemetry),
    );
    app.add_systems(
        FixedUpdate,
        save_progress.in_set(SimulationSet::Persistence),
    );
}

fn line(goal: &Goal, progress: u32, weekly: bool) -> GoalLine {
    GoalLine {
        template: goal.kind.template().to_owned(),
        target: goal.target,
        progress,
        reward_item: goal.reward_item.to_owned(),
        reward_quantity: goal.reward_quantity,
        weekly,
    }
}

pub fn snapshot(progress: &CaptainProgress) -> ProgressSnapshot {
    let mut goals: Vec<GoalLine> = daily_goals(progress.day)
        .iter()
        .zip(progress.daily)
        .map(|(goal, count)| line(goal, count, false))
        .collect();
    goals.push(line(&weekly_goal(progress.week), progress.weekly, true));
    ProgressSnapshot {
        goals,
        unpaid: progress.unpaid.clone(),
        abyss_best: progress.abyss_best,
        found: progress.found.iter().cloned().collect(),
        mastery: progress
            .mastery
            .iter()
            .map(|(hull, xp)| (hull.clone(), *xp))
            .collect(),
        season_points: if progress.season == season_of(progress.week).0 {
            progress.season_points
        } else {
            0
        },
        crowns: progress.crowns,
    }
}

fn send(
    connection_manager: &mut ConnectionManager,
    client: Option<ClientId>,
    progress: &CaptainProgress,
) {
    if let Some(client_id) = client {
        let _ =
            connection_manager.send_message::<ReliableChannel, _>(client_id, &snapshot(progress));
    }
}

fn load_on_connect(
    ships: Query<&ServerShip>,
    store: Res<StoreHandle>,
    mut logbook: ResMut<CaptainLogbook>,
    mut connection_manager: ResMut<ConnectionManager>,
) {
    let (day, week) = today();
    for ship in &ships {
        let Some(client_id) = ship.client_id else {
            continue;
        };
        let known = logbook.captains.get(&ship.character);
        if known.is_some_and(|c| c.client == Some(client_id)) {
            continue;
        }
        // Reconexão sem gravar ainda: a memória é mais nova que o banco.
        let (progress, is_unread) = match (known, store.0.as_ref()) {
            (Some(captain), _) if captain.is_dirty => (captain.progress.clone(), false),
            (_, Some(store)) => match store.load_progress(ship.character) {
                Ok(progress) => (progress, false),
                Err(error) => {
                    warn!(%error, "Diário não carregou: sessão sem metas");
                    (CaptainProgress::default(), true)
                }
            },
            (known, None) => (known.map(|c| c.progress.clone()).unwrap_or_default(), false),
        };
        let captain = logbook.captains.entry(ship.character).or_default();
        let is_dirty = captain.is_dirty;
        *captain = Captain {
            progress,
            client: Some(client_id),
            is_unread,
            is_dirty,
        };
        captain.progress.roll(day, week);
        send(&mut connection_manager, captain.client, &captain.progress);
    }
}

/// Conta os feitos do tick. Lê e escreve `RenownEarned` (a meta cumprida
/// rende Renome), por isso o cursor é manual.
fn record_deeds(
    mut cursor: Local<EventCursor<RenownEarned>>,
    mut events: ResMut<Events<RenownEarned>>,
    mut logbook: ResMut<CaptainLogbook>,
    mut connection_manager: ResMut<ConnectionManager>,
    ships: Query<&ServerShip>,
) {
    let earned: Vec<RenownEarned> = cursor.read(&events).copied().collect();
    // v42: todo Renome ganho vira maestria do casco que o capitão usa.
    for gain in &earned {
        let Some(ship) = ships.iter().find(|ship| ship.character == gain.character) else {
            continue;
        };
        let Some(captain) = logbook.captains.get_mut(&gain.character) else {
            continue;
        };
        if captain.is_unread || gain.amount == 0 {
            continue;
        }
        if let Some(level) = captain.progress.add_mastery(ship.kind.name(), gain.amount) {
            info!(character = ?gain.character, hull = ship.kind.name(), level, "maestria de casco subiu");
        }
        // v43: o mesmo Renome vira ponto de temporada.
        if captain.progress.add_season_points(today().1, gain.amount) {
            info!(character = ?gain.character, "coroa da temporada");
        }
        captain.is_dirty = true;
        send(&mut connection_manager, captain.client, &captain.progress);
    }
    for entry in earned
        .iter()
        .filter_map(|e| Some((e.character, entry_of(e)?)))
    {
        discover(&mut logbook, &mut connection_manager, entry.0, entry.1);
    }
    let deeds: Vec<(CharacterId, Deed)> = earned
        .iter()
        .filter_map(|earned| Some((earned.character, deed_of(earned)?)))
        .collect();
    if deeds.is_empty() {
        return;
    }
    let (day, week) = today();
    for (character, deed) in deeds {
        let Some(captain) = logbook.captains.get_mut(&character) else {
            continue;
        };
        if captain.is_unread {
            continue;
        }
        let before = captain.progress.clone();
        for goal in captain.progress.record(&deed, day, week) {
            info!(?character, kind = ?goal.kind, "meta do Diário cumprida");
            events.send(RenownEarned {
                character,
                amount: goal.renown,
                reason: GOAL_REASON,
            });
        }
        if captain.progress != before {
            captain.is_dirty = true;
            send(&mut connection_manager, captain.client, &captain.progress);
        }
    }
}

/// Registra a entrada do Livro; nova, avisa o capitão (e a página
/// completa ganha o título no próximo snapshot do navio).
fn discover(
    logbook: &mut CaptainLogbook,
    connection_manager: &mut ConnectionManager,
    character: CharacterId,
    entry: &'static str,
) {
    let Some(captain) = logbook.captains.get_mut(&character) else {
        return;
    };
    if captain.is_unread || !captain.progress.discover(entry) {
        return;
    }
    captain.is_dirty = true;
    info!(?character, entry, "entrada nova no Livro de Bordo");
    send(connection_manager, captain.client, &captain.progress);
}

fn record_discoveries(
    mut events: EventReader<Discovered>,
    mut logbook: ResMut<CaptainLogbook>,
    mut connection_manager: ResMut<ConnectionManager>,
) {
    for event in events.read() {
        discover(
            &mut logbook,
            &mut connection_manager,
            event.character,
            event.entry,
        );
    }
}

/// Atracou com meta cumprida: grava o Diário sem a dívida e só então põe o
/// recurso no armazém (falhou a gravação, ninguém recebe duas vezes).
fn pay_on_dock(
    ships: Query<&ServerShip>,
    dev: Res<DevItems>,
    store: Res<StoreHandle>,
    mut market: ResMut<ServerMarket>,
    mut logbook: ResMut<CaptainLogbook>,
    mut connection_manager: ResMut<ConnectionManager>,
) {
    for ship in &ships {
        let VesselPresence::Docked(region) = ship.presence else {
            continue;
        };
        let Some(captain) = logbook.captains.get_mut(&ship.character) else {
            continue;
        };
        if captain.is_unread || captain.progress.unpaid.is_empty() {
            continue;
        }
        let mut paid = captain.progress.clone();
        let owed = std::mem::take(&mut paid.unpaid);
        if let Some(store) = &store.0 {
            if let Err(error) = store.save_progress(ship.character, &paid) {
                warn!(%error, "Diário não gravou: pagamento fica para depois");
                continue;
            }
        }
        for (item, quantity) in &owed {
            let id = ItemDefinitionId::stable(item);
            if dev.catalog.get(id).is_some() {
                market.grant_to_storage(ship.character, region, id, *quantity, &dev.catalog);
            }
        }
        captain.progress = paid;
        info!(character = ?ship.character, ?owed, "Diário pago no porto");
        send(&mut connection_manager, captain.client, &captain.progress);
        if let Some(client) = captain.client {
            let text = owed
                .iter()
                .map(|(item, quantity)| format!("{quantity} {item}"))
                .collect::<Vec<_>>()
                .join(", ");
            crate::reputation::send_event(
                &mut connection_manager,
                &[client],
                format!("Diario de Bordo: {text} no armazem"),
                WorldEventKind::Kill,
            );
        }
    }
}

fn save_progress(time: Res<Time>, store: Res<StoreHandle>, mut logbook: ResMut<CaptainLogbook>) {
    logbook.save_clock += time.delta_secs();
    if logbook.save_clock < SAVE_EVERY {
        return;
    }
    logbook.save_clock = 0.0;
    let Some(store) = &store.0 else {
        for captain in logbook.captains.values_mut() {
            captain.is_dirty = false;
        }
        return;
    };
    for (character, captain) in &mut logbook.captains {
        if !captain.is_dirty || captain.is_unread {
            continue;
        }
        match store.save_progress(*character, &captain.progress) {
            Ok(()) => captain.is_dirty = false,
            Err(error) => warn!(%error, "Diário não gravou; tenta no próximo ciclo"),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn renown_reasons_map_to_deeds() {
        let earned = |reason, amount| RenownEarned {
            character: CharacterId::new(),
            amount,
            reason,
        };
        assert_eq!(deed_of(&earned("coleta", 7)), Some(Deed::Gather(7)));
        assert_eq!(
            deed_of(&earned("elite afundado", 1)),
            Some(Deed::Sink { elite: true })
        );
        assert_eq!(
            deed_of(&earned(GOAL_REASON, 90)),
            None,
            "meta não conta meta"
        );
    }

    #[test]
    fn best_title_counts_pages_and_maxed_hulls() {
        let character = CharacterId::new();
        let mut logbook = CaptainLogbook::default();
        logbook.captains.insert(character, Captain::default());
        assert_eq!(logbook.title(character), 0);
        let captain = logbook.captains.get_mut(&character).unwrap();
        for entry in marvyr_domain_economy::logbook::PAGES[0].entries {
            captain.progress.discover(entry);
        }
        assert_eq!(
            logbook.title(character),
            marvyr_domain_ships::title_code("o Andarilho da Névoa").unwrap()
        );
        let captain = logbook.captains.get_mut(&character).unwrap();
        captain.progress.add_mastery("Corsário", 1_000_000);
        assert_eq!(
            logbook.title(character),
            marvyr_domain_ships::title_code("Mestre do Corsário").unwrap()
        );
    }

    #[test]
    fn a_crown_is_the_best_title() {
        let character = CharacterId::new();
        let mut logbook = CaptainLogbook::default();
        let mut captain = Captain::default();
        captain.progress.crowns = 1;
        captain.progress.add_mastery("Mercante", 1_000_000);
        logbook.captains.insert(character, captain);
        assert_eq!(
            logbook.title(character),
            marvyr_domain_ships::title_code("Coroa da Maré").unwrap()
        );
    }

    #[test]
    fn snapshot_has_three_dailies_and_the_weekly() {
        let progress = CaptainProgress {
            day: 20_000,
            week: 2857,
            ..Default::default()
        };
        let snap = snapshot(&progress);
        assert_eq!(snap.goals.len(), 4);
        assert!(snap.goals[3].weekly && !snap.goals[0].weekly);
    }
}
