//! v45: influência de porto. Os portos fora das águas da coroa são
//! disputados: todo Renome ganho perto de um deles vira influência ali, e
//! quem tem mais na semana é o Senhor do Porto. O Senhor, ao atracar no seu
//! porto, cobra uma vez por dia o tributo — o recurso do porto, pago pela
//! guilda (bruto de NPC, Pilar 1). Nada sai do bolso de outro capitão.
//!
//! v62: guerra de território. A influência soma por companhia (capitão sem
//! companhia disputa sozinho) e o Senhor passa a ser a companhia: todo
//! membro cobra o tributo do dia. Cada porto disputado abre uma janela de
//! guerra fixa (`WAR_SECS` a cada `WAR_EVERY_SECS`, escalonada por porto e
//! pelo relógio real — reiniciar o servidor não muda a hora): na janela,
//! quem está no porto ganha influência por presença (dobrada se não houver
//! rival por perto) e afundar capitão rival ali vale um bônus grande.

use std::collections::HashMap;

use bevy::prelude::*;
use lightyear::prelude::server::*;
use lightyear::prelude::*;
use marvyr_domain_ships::VesselPresence;
use marvyr_domain_world::{RiskTier, WorldMap};
use marvyr_protocol::{WarFront, WorldEventKind};
use marvyr_shared::ids::{CharacterId, ItemDefinitionId};
use tracing::{info, warn};
use uuid::Uuid;

use crate::companies::Companies;
use crate::market::ServerMarket;
use crate::net::{DevItems, ReliableChannel, ServerShip, ServerWorldMap};
use crate::persist::StoreHandle;
use crate::progress::CaptainLogbook;
use crate::sets::SimulationSet;

/// Renome ganho até esta distância de um porto disputado conta para ele (m).
pub const INFLUENCE_RADIUS: f32 = 600.0;
/// Tributo diário do Senhor do Porto (unidades do recurso do porto).
pub const TRIBUTE: u32 = 20;
const REFRESH_EVERY: f32 = 10.0;
/// O banco é relido a cada tanto (o Diário em memória é mais fresco).
const DB_REFRESH_EVERY: f32 = 60.0;

/// v62: janela de guerra — `WAR_SECS` a cada `WAR_EVERY_SECS`; o porto `i`
/// abre `i * WAR_STAGGER_SECS` depois do anterior.
pub const WAR_EVERY_SECS: u64 = 3 * 3600;
pub const WAR_SECS: u64 = 20 * 60;
const WAR_STAGGER_SECS: u64 = 30 * 60;
/// Raio do porto na guerra (presença e abate valem aqui).
pub const WAR_RADIUS: f32 = 500.0;
const WAR_TICK_SECS: f32 = 5.0;
/// Influência por membro presente a cada tick (dobra sem rival no raio).
pub const WAR_PRESENCE: u32 = 3;
/// Afundar capitão de outra bandeira no porto em guerra.
pub const WAR_KILL: u32 = 60;

/// Portos disputados: os que não estão em águas protegidas.
pub fn contested_ports(map: &WorldMap) -> Vec<(&'static str, Vec2)> {
    map.regions()
        .iter()
        .filter_map(|region| region.port.as_ref())
        .filter(|port| {
            map.zone_at(port.x, port.y)
                .is_ok_and(|zone| zone.tier != RiskTier::Protected)
        })
        .map(|port| (port.name, Vec2::new(port.x, port.y)))
        .collect()
}

/// Porto disputado mais perto de (x, y), se estiver no raio.
pub fn port_near(map: &WorldMap, at: Vec2) -> Option<&'static str> {
    contested_ports(map)
        .into_iter()
        .map(|(name, port)| (name, port.distance(at)))
        .filter(|(_, distance)| *distance <= INFLUENCE_RADIUS)
        .min_by(|a, b| a.1.total_cmp(&b.1))
        .map(|(name, _)| name)
}

/// v62: quem disputa — a companhia, ou o capitão sozinho.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Holder {
    Company(Uuid),
    Captain(CharacterId),
}

pub fn holder_of(companies: &Companies, character: CharacterId) -> Holder {
    companies
        .id_of(character)
        .map_or(Holder::Captain(character), Holder::Company)
}

/// Dev (teste ao vivo): MARVYR_WAR_OPEN=1 deixa toda janela aberta, nunca
/// em produção. Lido uma vez.
fn war_forced_open() -> bool {
    static FORCED: std::sync::OnceLock<bool> = std::sync::OnceLock::new();
    *FORCED.get_or_init(|| {
        std::env::var_os("MARVYR_WAR_OPEN").is_some()
            && !std::env::var("MARVYR_ENV").is_ok_and(|env| env == "production")
    })
}

/// Janela de guerra do porto de índice `index` em `unix`: (aberta, segundos
/// até fechar ou abrir).
pub fn war_window(index: usize, unix: u64) -> (bool, u64) {
    if war_forced_open() {
        return (true, WAR_SECS);
    }
    let phase = (unix + WAR_EVERY_SECS - (index as u64 * WAR_STAGGER_SECS) % WAR_EVERY_SECS)
        % WAR_EVERY_SECS;
    if phase < WAR_SECS {
        (true, WAR_SECS - phase)
    } else {
        (false, WAR_EVERY_SECS - phase)
    }
}

fn unix_now() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_or(0, |elapsed| elapsed.as_secs())
}

#[derive(Debug, Clone)]
pub struct Lord {
    pub holder: Holder,
    /// "[TAG] Nome" ou o capitão.
    pub label: String,
    pub points: u32,
}

/// Senhores da semana e o placar de cada porto.
#[derive(Resource, Default)]
pub struct PortLords {
    week: Option<u32>,
    pub lords: HashMap<&'static str, Lord>,
    /// Porto → influência somada por quem disputa.
    boards: HashMap<&'static str, HashMap<Holder, u32>>,
    /// Porto → influência de cada capitão lida do banco.
    stored: HashMap<&'static str, HashMap<CharacterId, u32>>,
    ports: Vec<(&'static str, Vec2)>,
    clock: f32,
    db_clock: f32,
    /// Janela aberta por porto (anuncia abrir e fechar).
    war_open: HashMap<&'static str, bool>,
    war_clock: f32,
    /// Abates que já pagaram nesta janela: (porto, quem afundou, afundado).
    /// Um par só paga uma vez por janela (alt afundado em série não toma
    /// porto).
    war_kills: std::collections::HashSet<(&'static str, CharacterId, CharacterId)>,
}

impl PortLords {
    pub fn lord_of(&self, port: &str) -> Option<Holder> {
        self.lords.get(port).map(|lord| lord.holder)
    }

    pub fn ports_of(&self, holder: &Holder) -> Vec<String> {
        let mut ports: Vec<String> = self
            .lords
            .iter()
            .filter(|(_, lord)| lord.holder == *holder)
            .map(|(port, _)| (*port).to_owned())
            .collect();
        ports.sort();
        ports
    }

    /// Frentes de guerra vistas por `me`.
    pub fn fronts_for(&self, me: &Holder) -> Vec<WarFront> {
        let unix = unix_now();
        self.ports
            .iter()
            .enumerate()
            .map(|(index, (port, at))| {
                let (open, secs) = war_window(index, unix);
                let board = self.boards.get(port);
                let mine = board.and_then(|b| b.get(me)).copied().unwrap_or(0);
                let rival = board
                    .map(|b| {
                        b.iter()
                            .filter(|(holder, _)| *holder != me)
                            .map(|(_, points)| *points)
                            .max()
                            .unwrap_or(0)
                    })
                    .unwrap_or(0);
                WarFront {
                    port: (*port).to_owned(),
                    x: at.x,
                    y: at.y,
                    holder: self
                        .lords
                        .get(port)
                        .map(|lord| lord.label.clone())
                        .unwrap_or_default(),
                    open,
                    secs: secs as f32,
                    mine,
                    rival,
                }
            })
            .collect()
    }
}

pub fn install(app: &mut App) {
    app.init_resource::<PortLords>().add_systems(
        FixedUpdate,
        (refresh_lords, collect_tribute, run_war)
            .chain()
            .in_set(SimulationSet::EconomyConsequences),
    );
}

fn label_of(companies: &Companies, holder: Holder) -> String {
    match holder {
        Holder::Company(id) => companies
            .by_id(id)
            .map(|company| format!("[{}] {}", company.tag, company.name))
            .unwrap_or_default(),
        Holder::Captain(character) => crate::season::captain_label(character),
    }
}

/// Soma a influência de cada capitão por quem disputa.
fn board_of(
    companies: &Companies,
    points: impl IntoIterator<Item = (CharacterId, u32)>,
) -> HashMap<Holder, u32> {
    let mut board: HashMap<Holder, u32> = HashMap::new();
    for (character, points) in points {
        let total = board.entry(holder_of(companies, character)).or_default();
        *total = total.saturating_add(points);
    }
    board
}

/// Quem manda: mais influência; empate fica com o Senhor atual.
fn pick_lord(board: &HashMap<Holder, u32>, current: Option<Holder>) -> Option<(Holder, u32)> {
    let best = board.values().copied().max().filter(|best| *best > 0)?;
    if let Some(current) = current.filter(|holder| board.get(holder) == Some(&best)) {
        return Some((current, best));
    }
    board
        .iter()
        .filter(|(_, points)| **points == best)
        .map(|(holder, points)| (*holder, *points))
        .min_by_key(|(holder, _)| format!("{holder:?}"))
}

fn refresh_lords(
    time: Res<Time>,
    map: Res<ServerWorldMap>,
    store: Res<StoreHandle>,
    logbook: Res<CaptainLogbook>,
    companies: Res<Companies>,
    mut lords: ResMut<PortLords>,
) {
    // MV-067: sem as companhias lidas, todo membro pareceria sozinho.
    if !companies.is_loaded() {
        return;
    }
    let dt = time.delta_secs();
    lords.clock += dt;
    lords.db_clock += dt;
    let (_, week) = crate::progress::today();
    let new_week = lords.week != Some(week);
    // Virada de semana recarrega na hora (o Senhor velho não cobra o
    // tributo do dia novo na janela até o próximo refresh).
    if lords.clock < REFRESH_EVERY && !new_week {
        return;
    }
    lords.clock = 0.0;
    if lords.ports.is_empty() {
        lords.ports = contested_ports(&map.0);
    }
    let ports = lords.ports.clone();
    if new_week || lords.db_clock >= DB_REFRESH_EVERY {
        lords.db_clock = 0.0;
        if new_week {
            lords.lords.clear();
            lords.boards.clear();
            lords.stored.clear();
        }
        if let Some(store) = &store.0 {
            for (port, _) in &ports {
                match store.load_port_influence(week, port) {
                    Ok(rows) => {
                        lords.stored.insert(port, rows.into_iter().collect());
                    }
                    Err(error) => {
                        warn!(%error, port, "influência do porto não carregou do banco");
                        // MV-067: leitura que falhou não vira "ninguém é
                        // Senhor" na semana nova (um capitão online com
                        // menos influência que o Senhor offline cobraria
                        // tributo a semana toda). Sem Senhores até ler.
                        if new_week {
                            lords.lords.clear();
                            return;
                        }
                    }
                }
            }
        }
        lords.week = Some(week);
    }
    for (port, _) in &ports {
        // O Diário em memória é mais novo que o banco.
        let mut points = lords.stored.get(port).cloned().unwrap_or_default();
        for (character, progress) in logbook.captains() {
            let fresh = progress.influence_at(week, port);
            let entry = points.entry(character).or_default();
            *entry = (*entry).max(fresh);
        }
        let board = board_of(&companies, points);
        let current = lords.lord_of(port);
        match pick_lord(&board, current) {
            Some((holder, points)) => {
                let label = label_of(&companies, holder);
                lords.lords.insert(
                    port,
                    Lord {
                        holder,
                        label,
                        points,
                    },
                );
            }
            None => {
                lords.lords.remove(port);
            }
        }
        lords.boards.insert(port, board);
    }
}

/// O Senhor (ou membro da companhia Senhora) atracou no porto: tributo do
/// dia no armazém.
// System Bevy: params são injeção de dependência, não assinatura.
#[allow(clippy::too_many_arguments)]
fn collect_tribute(
    ships: Query<&ServerShip>,
    map: Res<ServerWorldMap>,
    lords: Res<PortLords>,
    companies: Res<Companies>,
    dev: Res<DevItems>,
    mut market: ResMut<ServerMarket>,
    mut logbook: ResMut<CaptainLogbook>,
    (store, mut connection_manager): (Res<StoreHandle>, ResMut<ConnectionManager>),
) {
    let (day, week) = crate::progress::today();
    // Senhores de outra semana (ou ainda não lidos) não cobram.
    if lords.week != Some(week) || !companies.is_loaded() {
        return;
    }
    for ship in &ships {
        let VesselPresence::Docked(region) = ship.presence else {
            continue;
        };
        let Some(port) = map
            .0
            .regions()
            .iter()
            .find(|r| r.id == region)
            .and_then(|r| r.port.as_ref())
            .map(|port| port.name)
        else {
            continue;
        };
        if lords.lord_of(port) != Some(holder_of(&companies, ship.character)) {
            continue;
        }
        if !logbook.claim_tribute(&store, ship.character, day) {
            continue;
        }
        let item_name = marvyr_domain_economy::guild::payout(port);
        let id = ItemDefinitionId::stable(item_name);
        if dev.catalog.get(id).is_none() {
            continue;
        }
        market.grant_to_storage(ship.character, region, id, TRIBUTE, &dev.catalog);
        info!(character = ?ship.character, port, "tributo de Senhor do Porto");
        if let Some(client) = ship.client_id {
            crate::reputation::send_event(
                &mut connection_manager,
                &[client],
                format!("Senhor do Porto: tributo de {TRIBUTE} {item_name} no armazem"),
                WorldEventKind::Kill,
            );
        }
    }
}

fn announce(connection_manager: &mut ConnectionManager, text: String) {
    let _ = connection_manager.send_message_to_target::<ReliableChannel, _>(
        &marvyr_protocol::WorldEvent {
            text,
            kind: WorldEventKind::Alert,
        },
        NetworkTarget::All,
    );
}

/// Presença por bandeira no raio do porto: quem está lá e se há rival.
fn presence_points(present: &[(CharacterId, Holder)]) -> Vec<(CharacterId, u32)> {
    let mut flags: Vec<Holder> = present.iter().map(|(_, holder)| *holder).collect();
    flags.sort_by_key(|holder| format!("{holder:?}"));
    flags.dedup();
    let each = if flags.len() == 1 {
        WAR_PRESENCE * 2
    } else {
        WAR_PRESENCE
    };
    present
        .iter()
        .map(|(character, _)| (*character, each))
        .collect()
}

/// v62: janelas de guerra — anuncia, paga presença e abate no porto.
#[allow(clippy::too_many_arguments)]
fn run_war(
    time: Res<Time>,
    mut lords: ResMut<PortLords>,
    companies: Res<Companies>,
    mut logbook: ResMut<CaptainLogbook>,
    ships: Query<&ServerShip>,
    destructions: Res<crate::net::PendingShipDestructions>,
    mut connection_manager: ResMut<ConnectionManager>,
) {
    if lords.ports.is_empty() || !companies.is_loaded() {
        return;
    }
    let unix = unix_now();
    let ports = lords.ports.clone();
    let open: Vec<bool> = (0..ports.len())
        .map(|index| war_window(index, unix).0)
        .collect();
    for ((port, _), open) in ports.iter().zip(&open) {
        let was = lords.war_open.insert(port, *open);
        match (was, *open) {
            (Some(false), true) => announce(
                &mut connection_manager,
                format!("Guerra em {port}! 20 minutos para tomar o porto."),
            ),
            (Some(true), false) => {
                lords.war_kills.retain(|(kill_port, ..)| kill_port != port);
                let holder = lords
                    .lords
                    .get(port)
                    .map(|lord| lord.label.clone())
                    .unwrap_or_else(|| String::from("ninguém"));
                announce(
                    &mut connection_manager,
                    format!("A guerra em {port} acabou: manda {holder}."),
                );
            }
            _ => {}
        }
    }
    // Abate: capitão afundado no porto em guerra por outra bandeira.
    for destruction in &destructions.0 {
        let Some(killer) = destruction.exclusive_looter else {
            continue;
        };
        if killer == destruction.victim_character
            || holder_of(&companies, killer) == holder_of(&companies, destruction.victim_character)
        {
            continue;
        }
        let at = Vec2::new(destruction.victim_x, destruction.victim_y);
        for ((port, spot), open) in ports.iter().zip(&open) {
            if *open
                && spot.distance(at) <= WAR_RADIUS
                && lords
                    .war_kills
                    .insert((port, killer, destruction.victim_character))
                && logbook.add_war_influence(killer, port, WAR_KILL)
            {
                info!(?killer, port, "abate de guerra");
            }
        }
    }
    lords.war_clock += time.delta_secs();
    if lords.war_clock < WAR_TICK_SECS {
        return;
    }
    lords.war_clock = 0.0;
    for ((port, spot), open) in ports.iter().zip(&open) {
        if !*open {
            continue;
        }
        let present: Vec<(CharacterId, Holder)> = ships
            .iter()
            .filter(|ship| ship.client_id.is_some() && ship.presence == VesselPresence::AtSea)
            .filter(|ship| spot.distance(Vec2::new(ship.motion.x, ship.motion.y)) <= WAR_RADIUS)
            .map(|ship| (ship.character, holder_of(&companies, ship.character)))
            .collect();
        for (character, points) in presence_points(&present) {
            logbook.add_war_influence(character, port, points);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn only_ports_outside_crown_waters_are_contested() {
        for seed in [0, 1, 7, 42] {
            let map = WorldMap::from_seed(seed);
            let ports = contested_ports(&map);
            assert!(!ports.is_empty(), "seed {seed}: algum porto em disputa");
            for (name, at) in &ports {
                assert!(!["Porto da Serra", "Porto da Mina"].contains(name));
                assert_eq!(port_near(&map, *at), Some(*name));
            }
        }
        let map = WorldMap::from_seed(0);
        assert_eq!(port_near(&map, Vec2::from(map.features().spawn)), None);
    }

    #[test]
    fn war_windows_are_staggered_and_last_twenty_minutes() {
        // Porto 0 abre na virada do ciclo; o porto 1, meia hora depois.
        assert_eq!(war_window(0, 0), (true, WAR_SECS));
        assert_eq!(war_window(0, WAR_SECS), (false, WAR_EVERY_SECS - WAR_SECS));
        assert!(!war_window(1, 0).0);
        assert_eq!(war_window(1, WAR_STAGGER_SECS), (true, WAR_SECS));
        let open = (0..WAR_EVERY_SECS)
            .step_by(60)
            .filter(|t| war_window(3, *t).0)
            .count() as u64;
        assert_eq!(open * 60, WAR_SECS);
    }

    #[test]
    fn a_company_sums_its_members_and_ties_keep_the_lord() {
        let companies = Companies::default();
        let (a, b, c) = (CharacterId::new(), CharacterId::new(), CharacterId::new());
        let board = board_of(&companies, [(a, 30), (b, 50), (c, 50)]);
        // Sem companhia cada um é uma bandeira.
        assert_eq!(board.len(), 3);
        let lord = pick_lord(&board, Some(Holder::Captain(c))).unwrap();
        assert_eq!(lord, (Holder::Captain(c), 50), "empate fica com quem manda");
        let company = Holder::Company(Uuid::new_v4());
        let mut summed = HashMap::new();
        summed.insert(company, 80);
        summed.insert(Holder::Captain(c), 50);
        assert_eq!(
            pick_lord(&summed, Some(Holder::Captain(c))),
            Some((company, 80))
        );
        assert_eq!(pick_lord(&HashMap::new(), None), None);
    }

    #[test]
    fn holding_the_port_alone_pays_double() {
        let (a, b) = (CharacterId::new(), CharacterId::new());
        let flag = Holder::Company(Uuid::new_v4());
        let alone = presence_points(&[(a, flag), (b, flag)]);
        assert!(alone.iter().all(|(_, p)| *p == WAR_PRESENCE * 2));
        let contested = presence_points(&[(a, flag), (b, Holder::Captain(b))]);
        assert!(contested.iter().all(|(_, p)| *p == WAR_PRESENCE));
    }
}
