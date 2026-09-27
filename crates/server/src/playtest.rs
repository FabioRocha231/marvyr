//! Session recorder (MF-055, MV-061).
//!
//! Observabilidade, não economia autoritativa: toda execução do servidor
//! identifica a sessão por um UUID e grava um resumo JSON em
//! `MARVYR_REPORT_DIR` (default `playtest-results/`) quando o processo
//! encerra de forma ordenada (SIGTERM/SIGINT): `session-<id>.json` e uma
//! cópia `session-summary.json` com a última sessão. Nada aqui toca no
//! estado persistido do jogo.

use std::collections::{HashMap, HashSet};
use std::time::Instant;

use bevy::app::{App, AppExit, TerminalCtrlCHandlerPlugin};
use bevy::ecs::event::EventReader;
use bevy::prelude::{IntoSystemConfigs, Query, Res, ResMut, Resource, Time, Update};
use lightyear::prelude::ServerReceiveMessage;
use marvyr_protocol::OnboardingProgress;
use marvyr_shared::ids::CharacterId;
use serde::Serialize;
use uuid::Uuid;

use crate::net::{Metrics, ServerShip, TripTelemetry};

/// Último passo do guia de onboarding (1..=6); 0 = boas-vindas.
const ONBOARDING_LAST_STEP: usize = 6;

/// Funil do onboarding (MV-062): instante (segundos de servidor) em que
/// cada personagem atingiu cada passo pela PRIMEIRA vez. Telemetria pura —
/// nada aqui concede ou libera coisa alguma.
#[derive(Debug, Default, Clone)]
pub struct OnboardingTelemetry {
    reached_at: HashMap<CharacterId, [Option<f64>; ONBOARDING_LAST_STEP + 1]>,
    skipped: HashSet<CharacterId>,
}

impl OnboardingTelemetry {
    /// Registra um relato do client. Repetição e passo fora de 0..=6 são
    /// ignorados; `skipped` marca o pulo sem contar o passo como concluído.
    pub fn record(&mut self, character: CharacterId, step: u8, skipped: bool, now_secs: f64) {
        let step = usize::from(step);
        if step > ONBOARDING_LAST_STEP {
            return;
        }
        if skipped {
            self.skipped.insert(character);
            return;
        }
        let slot = &mut self.reached_at.entry(character).or_default()[step];
        slot.get_or_insert(now_secs);
    }

    /// Personagens que viram as boas-vindas (passo 0).
    pub fn welcomed(&self) -> usize {
        self.reached_at
            .values()
            .filter(|steps| steps[0].is_some())
            .count()
    }

    fn summary(&self) -> OnboardingSummary {
        let count = |step: usize| {
            self.reached_at
                .values()
                .filter(|steps| steps[step].is_some())
                .count()
        };
        let median_secs_to = |step: usize| {
            let mut deltas: Vec<f64> = self
                .reached_at
                .values()
                .filter_map(|steps| Some((steps[step]? - steps[0]?).max(0.0)))
                .collect();
            median(&mut deltas)
        };
        OnboardingSummary {
            welcomed: self.welcomed(),
            skipped: self.skipped.len(),
            reached: std::array::from_fn(|index| count(index + 1)),
            median_secs_to_step: std::array::from_fn(|index| median_secs_to(index + 1)),
        }
    }
}

fn median(values: &mut [f64]) -> Option<f64> {
    if values.is_empty() {
        return None;
    }
    values.sort_by(f64::total_cmp);
    let mid = values.len() / 2;
    Some(if values.len() % 2 == 0 {
        (values[mid - 1] + values[mid]) / 2.0
    } else {
        values[mid]
    })
}

/// Bloco `onboarding` do relatório (MV-062).
#[derive(Serialize, Debug, PartialEq)]
struct OnboardingSummary {
    welcomed: usize,
    skipped: usize,
    /// Personagens que concluíram cada passo 1..=6.
    reached: [usize; ONBOARDING_LAST_STEP],
    /// Mediana de segundos das boas-vindas até cada passo; `null` sem dados.
    median_secs_to_step: [Option<f64>; ONBOARDING_LAST_STEP],
}

/// Intent `OnboardingProgress` (MV-062): só registra, por personagem.
pub(crate) fn handle_onboarding(
    mut events: EventReader<ServerReceiveMessage<OnboardingProgress>>,
    time: Res<Time>,
    mut metrics: ResMut<Metrics>,
    ships: Query<&ServerShip>,
) {
    for event in events.read() {
        let client_id = event.from();
        let Some(ship) = ships.iter().find(|ship| ship.client_id == Some(client_id)) else {
            continue;
        };
        let progress = event.message();
        metrics.onboarding.record(
            ship.character,
            progress.step,
            progress.skipped,
            time.elapsed_secs_f64(),
        );
    }
}

/// Identidade de sessão gerada no boot quando `--playtest` está ativo.
#[derive(Resource, Debug, Clone, Copy)]
struct PlaytestSessionId(Uuid);

/// Instante de boot (relógio de parede) usado para `session_duration`.
#[derive(Resource)]
struct PlaytestBoot(Instant);

/// Resumo gravado em disco no encerramento. Os campos casam com o spec
/// MF-055; `ship_losses_by_kind` segue a ordem de `ShipKind as usize`
/// (SmallMerchant=0, Patrol=1, Corsair=2).
#[derive(Serialize)]
struct PlaytestReport {
    version: &'static str,
    build: &'static str,
    protocol: u16,
    session_duration: f64,
    players_seen: usize,
    trips: u64,
    completed_routes: u64,
    average_trip_duration: f64,
    cargo_value_at_risk: u64,
    cargo_value_coverage: f32,
    pvp_engagements: u64,
    ship_losses_by_kind: [u64; marvyr_domain_ships::ShipKind::ALL.len()],
    wrecks_looted: u64,
    items_gathered: u64,
    items_crafted: u64,
    items_destroyed: u64,
    /// Recurso bruto (unidades) que NPC afundado deixou boiando.
    npc_spoils_dropped: u64,
    cargo_value_departed: u64,
    cargo_value_arrived: u64,
    cargo_value_sunk: u64,
    ships_destroyed: u64,
    boardings_won: u64,
    treasures_dug: u64,
    sea_events_started: u64,
    onboarding: OnboardingSummary,
}

/// Monta o resumo a partir do `Metrics` já existente. Não
/// adiciona nenhuma medição nova: valores que não existem em nenhuma fonte
/// autoritativa ficariam em zero.
fn build_report<'a>(
    session_duration: f64,
    metrics: &Metrics,
    active_trips: impl Iterator<Item = &'a TripTelemetry>,
) -> PlaytestReport {
    let completed_routes = metrics.completed_routes.values().sum::<u64>();
    let average_trip_duration = if metrics.trip_count == 0 {
        0.0
    } else {
        metrics.trip_total_secs / metrics.trip_count as f64
    };
    let active = active_trips.fold((0u64, 0u64, 0u64), |totals, trip| {
        let (value_at_risk, priced, unpriced) = totals;
        (
            value_at_risk.saturating_add(if trip.unpriced_quantity == 0 {
                trip.marked_cargo_value
            } else {
                0
            }),
            priced.saturating_add(u64::from(trip.priced_quantity)),
            unpriced.saturating_add(u64::from(trip.unpriced_quantity)),
        )
    });
    let priced_items = metrics.cargo_value_priced_items.saturating_add(active.1);
    let unpriced_items = metrics.cargo_value_unpriced_items.saturating_add(active.2);
    let cargo_value_coverage = if priced_items + unpriced_items == 0 {
        100.0
    } else {
        priced_items as f32 / (priced_items + unpriced_items) as f32 * 100.0
    };

    PlaytestReport {
        version: marvyr_protocol::VERSION_LABEL,
        build: marvyr_protocol::BUILD_SHA,
        protocol: marvyr_protocol::PROTOCOL_VERSION,
        session_duration,
        players_seen: metrics.unique_players.len(),
        trips: metrics.trip_count,
        completed_routes,
        average_trip_duration,
        cargo_value_at_risk: metrics.cargo_value_at_risk_total.saturating_add(active.0),
        cargo_value_coverage,
        pvp_engagements: metrics.pvp_engagements,
        ship_losses_by_kind: metrics.ship_losses_by_kind,
        wrecks_looted: metrics.wrecks_looted,
        items_gathered: metrics.items_gathered,
        items_crafted: metrics.items_crafted,
        items_destroyed: metrics.items_destroyed,
        npc_spoils_dropped: metrics.npc_spoils_dropped,
        cargo_value_departed: metrics.cargo_value_departed,
        cargo_value_arrived: metrics.cargo_value_arrived,
        cargo_value_sunk: metrics.cargo_value_sunk,
        ships_destroyed: metrics.ships_destroyed,
        boardings_won: metrics.boardings_won,
        treasures_dug: metrics.treasures_dug,
        sea_events_started: metrics.sea_events_started,
        onboarding: metrics.onboarding.summary(),
    }
}

/// Grava o resumo em `<dir>/session-{id}.json` e `<dir>/session-summary.json`.
fn write_report(report: &PlaytestReport, session_id: Uuid) -> std::io::Result<()> {
    let dir = std::env::var_os("MARVYR_REPORT_DIR")
        .map(std::path::PathBuf::from)
        .unwrap_or_else(|| std::path::PathBuf::from("playtest-results"));
    std::fs::create_dir_all(&dir)?;
    let json = serde_json::to_string_pretty(report)
        .map_err(|error| std::io::Error::other(error.to_string()))?;
    std::fs::write(dir.join(format!("session-{session_id}.json")), &json)?;
    std::fs::write(dir.join("session-summary.json"), json)
}

/// Instala os recursos, o handler de SIGTERM e o dump no encerramento.
pub(crate) fn install(app: &mut App) {
    install_sigterm_handler();
    app.insert_resource(PlaytestSessionId(Uuid::new_v4()));
    app.insert_resource(PlaytestBoot(Instant::now()));
    app.add_systems(
        Update,
        dump_on_exit.after(TerminalCtrlCHandlerPlugin::exit_on_flag),
    );
}

/// Escreve o resumo quando o app recebe `AppExit` (SIGINT/Ctrl+C via
/// `TerminalCtrlCHandlerPlugin`, SIGTERM via handler próprio, ou `TickLimit`).
fn dump_on_exit(
    mut exits: EventReader<AppExit>,
    session: Res<PlaytestSessionId>,
    boot: Res<PlaytestBoot>,
    metrics: Res<Metrics>,
    ships: Query<&ServerShip>,
) {
    if exits.read().next().is_none() {
        return;
    }
    let report = build_report(
        boot.0.elapsed().as_secs_f64(),
        &metrics,
        ships.iter().filter_map(|ship| ship.trip.as_ref()),
    );
    match write_report(&report, session.0) {
        Ok(()) => tracing::info!(session = %session.0, "playtest report written"),
        Err(error) => tracing::error!(error = %error, "failed to write playtest report"),
    }
}

/// SIGTERM roteado para o mesmo flag do Ctrl+C, para que o encerramento
/// ordenado (docker stop, Dokploy redeploy) persista e grave o resumo antes
/// do processo sair.
#[cfg(unix)]
fn install_sigterm_handler() {
    let handler: extern "C" fn(libc::c_int) = handle_sigterm;
    // SAFETY: assinatura e semântica de `libc::signal` para um handler global
    // que apenas faz um store atômico (o mesmo padrão usado pelo
    // TerminalCtrlCHandlerPlugin no SIGINT).
    unsafe {
        libc::signal(libc::SIGTERM, handler as libc::sighandler_t);
    }
}

/// Windows não tem SIGTERM: Ctrl+C (TerminalCtrlCHandlerPlugin) cobre.
#[cfg(not(unix))]
fn install_sigterm_handler() {}

#[cfg(unix)]
extern "C" fn handle_sigterm(_sig: libc::c_int) {
    TerminalCtrlCHandlerPlugin::gracefully_exit();
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::net::TradeRouteKey;
    use marvyr_shared::ids::RegionId;

    #[test]
    fn report_builder_emits_zero_fields_for_empty_metrics() {
        let report = build_report(0.0, &Metrics::default(), [].iter());

        assert_eq!(report.session_duration, 0.0);
        assert_eq!(report.players_seen, 0);
        assert_eq!(report.trips, 0);
        assert_eq!(report.completed_routes, 0);
        assert_eq!(report.average_trip_duration, 0.0);
        assert_eq!(report.cargo_value_at_risk, 0);
        assert_eq!(report.pvp_engagements, 0);
        assert_eq!(
            report.ship_losses_by_kind,
            [0; marvyr_domain_ships::ShipKind::ALL.len()]
        );
        assert_eq!(report.wrecks_looted, 0);
        assert_eq!(report.items_gathered, 0);
        assert_eq!(report.items_crafted, 0);
        assert_eq!(report.items_destroyed, 0);
        assert_eq!(report.npc_spoils_dropped, 0);
    }

    #[test]
    fn onboarding_summary_counts_first_reach_and_medians() {
        let (a, b, c) = (CharacterId::new(), CharacterId::new(), CharacterId::new());
        let mut log = OnboardingTelemetry::default();
        log.record(a, 0, false, 10.0);
        log.record(a, 1, false, 20.0);
        log.record(a, 1, false, 99.0); // repetição: ignorada
        log.record(b, 0, false, 0.0);
        log.record(b, 1, false, 30.0);
        log.record(c, 0, false, 5.0);
        log.record(c, 1, false, 45.0);
        log.record(c, 7, false, 50.0); // fora do intervalo: ignorado
        log.record(c, 2, true, 60.0); // pulou o guia
        log.record(c, 2, true, 61.0);

        let summary = log.summary();
        assert_eq!(summary.welcomed, 3);
        assert_eq!(summary.skipped, 1);
        assert_eq!(summary.reached, [3, 0, 0, 0, 0, 0]);
        // Deltas 10, 30, 40 → mediana 30 (a repetição em 99s não conta).
        assert_eq!(summary.median_secs_to_step[0], Some(30.0));
        assert_eq!(summary.median_secs_to_step[1], None);

        log.record(a, 2, false, 14.0);
        log.record(b, 2, false, 6.0);
        assert_eq!(log.summary().median_secs_to_step[1], Some(5.0));

        let json = serde_json::to_value(log.summary()).unwrap();
        assert!(json["median_secs_to_step"][2].is_null());
    }

    #[test]
    fn report_builder_guards_div_by_zero() {
        let metrics = Metrics {
            trip_total_secs: 123.0,
            cargo_value_at_risk_total: 500,
            ..Metrics::default()
        };

        let report = build_report(10.0, &metrics, [].iter());

        assert_eq!(report.average_trip_duration, 0.0);
        assert_eq!(report.cargo_value_at_risk, 500);
    }

    #[test]
    fn report_builder_sums_completed_routes() {
        let mut metrics = Metrics {
            trip_total_secs: 100.0,
            trip_count: 10,
            ..Metrics::default()
        };
        metrics.completed_routes.insert(
            TradeRouteKey {
                origin: RegionId::new(),
                destination: RegionId::new(),
            },
            3,
        );
        metrics.completed_routes.insert(
            TradeRouteKey {
                origin: RegionId::new(),
                destination: RegionId::new(),
            },
            5,
        );

        let report = build_report(0.0, &metrics, [].iter());

        assert_eq!(report.completed_routes, 8);
        assert_eq!(report.average_trip_duration, 10.0);
    }

    #[test]
    fn report_includes_priced_active_trip_without_counting_it_as_completed() {
        let active = [TripTelemetry {
            started_at: 0.0,
            origin: RegionId::new(),
            marked_cargo_value: 250,
            priced_quantity: 5,
            unpriced_quantity: 0,
        }];
        let report = build_report(0.0, &Metrics::default(), active.iter());

        assert_eq!(report.trips, 0);
        assert_eq!(report.cargo_value_at_risk, 250);
        assert_eq!(report.cargo_value_coverage, 100.0);
    }

    #[test]
    fn report_includes_unpriced_active_trip_in_coverage_without_zero_valuation() {
        let active = [TripTelemetry {
            started_at: 0.0,
            origin: RegionId::new(),
            marked_cargo_value: 0,
            priced_quantity: 0,
            unpriced_quantity: 4,
        }];
        let report = build_report(0.0, &Metrics::default(), active.iter());

        assert_eq!(report.cargo_value_at_risk, 0);
        assert_eq!(report.cargo_value_coverage, 0.0);
    }
}
