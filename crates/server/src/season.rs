//! v43: placar da temporada. Os pontos moram na progressão de cada capitão
//! (`CaptainProgress`); aqui só o recorte dos dez melhores para todo mundo:
//! o banco no boot (e na virada) mais quem está na sessão agora.

use std::collections::HashMap;

use bevy::prelude::*;
use lightyear::prelude::server::*;
use lightyear::prelude::*;
use marvyr_domain_economy::logbook::{season_days_left, season_of, SEASON_CROWN};
use marvyr_protocol::SeasonBoard;
use marvyr_shared::ids::CharacterId;
use tracing::warn;

use crate::net::ReliableChannel;
use crate::persist::StoreHandle;
use crate::progress::CaptainLogbook;
use crate::sets::SimulationSet;

/// De quanto em quanto o placar vai para todo mundo (s).
const BROADCAST_EVERY: f32 = 10.0;
pub const TOP: usize = 10;

#[derive(Resource, Default)]
pub struct SeasonScores {
    season: Option<u32>,
    points: HashMap<CharacterId, u32>,
    clock: f32,
}

/// Rótulo público do capitão (ainda não há nome de capitão): um código
/// curto e estável do personagem. Oito hex (4 bi de códigos): com 4, dois
/// capitães já colidiam a partir de umas centenas no servidor e o quadro
/// de caçadas apontava o alvo errado.
pub fn captain_label(character: CharacterId) -> String {
    let hex = character.0.simple().to_string().to_uppercase();
    format!("Capitão {}", &hex[..8])
}

/// Os dez melhores, maior primeiro (empate: código do capitão).
pub fn top(points: &HashMap<CharacterId, u32>) -> Vec<(CharacterId, u32)> {
    let mut all: Vec<(CharacterId, u32)> = points
        .iter()
        .filter(|(_, p)| **p > 0)
        .map(|(c, p)| (*c, *p))
        .collect();
    all.sort_by(|a, b| b.1.cmp(&a.1).then(a.0 .0.cmp(&b.0 .0)));
    all.truncate(TOP);
    all
}

pub fn install(app: &mut App) {
    app.init_resource::<SeasonScores>()
        .add_systems(FixedUpdate, broadcast_board.in_set(SimulationSet::Snapshot));
}

fn broadcast_board(
    time: Res<Time>,
    store: Res<StoreHandle>,
    logbook: Res<CaptainLogbook>,
    lords: Res<crate::territory::PortLords>,
    mut scores: ResMut<SeasonScores>,
    mut connection_manager: ResMut<ConnectionManager>,
) {
    scores.clock += time.delta_secs();
    if scores.clock < BROADCAST_EVERY {
        return;
    }
    scores.clock = 0.0;
    let (day, week) = crate::progress::today();
    let (season, theme) = season_of(week);
    if scores.season != Some(season) {
        scores.points.clear();
        // Falhou a leitura: tenta de novo no próximo pulso (senão os
        // líderes offline sumiam do placar a temporada inteira).
        let loaded = match &store.0 {
            Some(store) => match store.load_season_top(season, TOP as u32) {
                Ok(rows) => {
                    scores.points.extend(rows);
                    true
                }
                Err(error) => {
                    warn!(%error, "placar da temporada não carregou do banco");
                    false
                }
            },
            None => true,
        };
        if loaded {
            scores.season = Some(season);
        }
    }
    for (character, progress) in logbook.captains() {
        if progress.season == season && progress.season_points > 0 {
            scores.points.insert(character, progress.season_points);
        }
    }
    let board = SeasonBoard {
        season,
        theme: theme.name().to_owned(),
        theme_description: theme.description().to_owned(),
        days_left: season_days_left(day),
        crown_at: SEASON_CROWN,
        top: top(&scores.points)
            .into_iter()
            .map(|(character, points)| (captain_label(character), points))
            .collect(),
        lords: {
            let mut lords: Vec<(String, String, u32)> = lords
                .lords
                .iter()
                .map(|(port, (character, points))| {
                    ((*port).to_owned(), captain_label(*character), *points)
                })
                .collect();
            lords.sort();
            lords
        },
    };
    let _ =
        connection_manager.send_message_to_target::<ReliableChannel, _>(&board, NetworkTarget::All);
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn top_ten_is_sorted_and_skips_zero() {
        let mut points = HashMap::new();
        for i in 0..15u32 {
            points.insert(CharacterId::new(), i * 10);
        }
        let board = top(&points);
        assert_eq!(board.len(), TOP);
        assert_eq!(board[0].1, 140);
        assert!(board.windows(2).all(|w| w[0].1 >= w[1].1));
        assert!(board.iter().all(|(_, p)| *p > 0));
        assert!(captain_label(CharacterId::new()).starts_with("Capitão "));
    }
}
