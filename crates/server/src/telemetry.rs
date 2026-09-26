//! Telemetria de retenção: o que cada capitão faz, gravado em lote na
//! tabela `captain_events` para análise (D1/D7, metas, mecânicas usadas).
//! Nunca volta para o jogo — perder um lote não muda nada do mundo.
//!
//! Tipos: `sessao` (entrou; detalhe = casco), `renome` (usou a mecânica
//! que rende Renome, uma vez por dia por motivo; detalhe = motivo),
//! `meta` (meta do Diário cumprida; detalhe = modelo) e `livro` (entrada
//! nova do Livro de Bordo). Consultas prontas em `docs/DEPLOY.md`.

use std::collections::{HashMap, HashSet};

use bevy::prelude::*;
use lightyear::prelude::ClientId;
use marvyr_shared::ids::CharacterId;
use tracing::warn;

use crate::net::ServerShip;
use crate::persist::StoreHandle;
use crate::renown::RenownEarned;
use crate::sets::SimulationSet;

/// De quanto em quanto o lote vai para o banco (s).
const FLUSH_EVERY: f32 = 30.0;
/// Teto do que espera o banco: banco fora do ar não enche a memória.
const MAX_PENDING: usize = 10_000;

#[derive(Resource, Default)]
pub struct Telemetry {
    pending: Vec<(CharacterId, &'static str, String)>,
    /// (capitão, motivo) já registrados hoje, e o dia deles.
    seen_today: HashSet<(CharacterId, &'static str)>,
    day: u32,
    online: HashMap<CharacterId, ClientId>,
    clock: f32,
}

impl Telemetry {
    pub fn note(&mut self, character: CharacterId, kind: &'static str, detail: impl Into<String>) {
        if self.pending.len() >= MAX_PENDING {
            self.pending.remove(0);
        }
        self.pending.push((character, kind, detail.into()));
    }

    /// Uso da mecânica: conta uma vez por dia por capitão e motivo.
    fn note_daily(&mut self, day: u32, character: CharacterId, reason: &'static str) {
        if self.day != day {
            self.day = day;
            self.seen_today.clear();
        }
        if self.seen_today.insert((character, reason)) {
            self.note(character, "renome", reason);
        }
    }
}

pub fn install(app: &mut App) {
    app.init_resource::<Telemetry>()
        .add_systems(
            FixedUpdate,
            (note_sessions, note_renown).in_set(SimulationSet::Telemetry),
        )
        .add_systems(FixedUpdate, flush.in_set(SimulationSet::Persistence));
}

fn note_sessions(ships: Query<&ServerShip>, mut telemetry: ResMut<Telemetry>) {
    for ship in &ships {
        let Some(client) = ship.client_id else {
            continue;
        };
        if telemetry.online.insert(ship.character, client) != Some(client) {
            telemetry.note(ship.character, "sessao", ship.kind.name());
        }
    }
}

fn note_renown(mut events: EventReader<RenownEarned>, mut telemetry: ResMut<Telemetry>) {
    let (day, _) = crate::progress::today();
    for earned in events.read() {
        telemetry.note_daily(day, earned.character, earned.reason);
    }
}

fn flush(time: Res<Time>, store: Res<StoreHandle>, mut telemetry: ResMut<Telemetry>) {
    telemetry.clock += time.delta_secs();
    if telemetry.clock < FLUSH_EVERY || telemetry.pending.is_empty() {
        return;
    }
    telemetry.clock = 0.0;
    let Some(store) = &store.0 else {
        telemetry.pending.clear();
        return;
    };
    match store.append_events(&telemetry.pending) {
        Ok(()) => telemetry.pending.clear(),
        Err(error) => warn!(%error, pending = telemetry.pending.len(), "telemetria não gravou"),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn renown_counts_once_a_day_per_reason() {
        let mut telemetry = Telemetry::default();
        let captain = CharacterId::new();
        for _ in 0..5 {
            telemetry.note_daily(10, captain, "coleta");
        }
        telemetry.note_daily(10, captain, "pesca");
        telemetry.note_daily(11, captain, "coleta");
        let kinds: Vec<&str> = telemetry.pending.iter().map(|e| e.2.as_str()).collect();
        assert_eq!(kinds, ["coleta", "pesca", "coleta"]);
    }

    #[test]
    fn pending_is_capped() {
        let mut telemetry = Telemetry::default();
        let captain = CharacterId::new();
        for i in 0..MAX_PENDING + 5 {
            telemetry.note(captain, "livro", i.to_string());
        }
        assert_eq!(telemetry.pending.len(), MAX_PENDING);
        assert_eq!(telemetry.pending[0].2, "5");
    }
}
