//! v46: faróis de jogador. Perto da costa, o capitão gasta madeira e
//! minério do porão e ergue um farol: clareia a noite em volta, aparece na
//! carta de todo mundo e, a cada capitão que passa na luz (uma vez por dia
//! cada), rende Renome a quem ergueu. Apaga sozinho; qualquer um reforça
//! com madeira. É ralo de recurso com cooperação — nada sai de NPC.

use std::collections::{HashMap, HashSet};
use std::time::{SystemTime, UNIX_EPOCH};

use bevy::prelude::*;
use lightyear::prelude::server::*;
use lightyear::prelude::*;
use marvyr_domain_ships::VesselPresence;
use marvyr_protocol::lighthouse::{COAST, LIGHT, SPACING, TEND};
use marvyr_protocol::{ActionKind, LighthouseLine, LighthousesUpdate, RaiseLighthouse};
use marvyr_shared::ids::CharacterId;
use tracing::{info, warn};

use crate::net::{DevItems, ReliableChannel, ServerShip, ServerWorldMap};
use crate::persist::StoreHandle;
use crate::seafaring::send_action;
use crate::sets::SimulationSet;

/// Erguer: madeira e minério do porão.
pub const BUILD_TIMBER: u32 = 12;
pub const BUILD_ORE: u32 = 4;
/// Reforçar: madeira.
pub const TEND_TIMBER: u32 = 5;
/// Aceso por (h) ao erguer, somado por reforço, e o teto.
const LIFE_HOURS: u64 = 72;
const TEND_HOURS: u64 = 24;
const MAX_HOURS: u64 = 7 * 24;
/// Faróis acesos por capitão.
pub const MAX_PER_BUILDER: usize = 3;
/// Renome de quem ergueu por capitão que passou na luz (1x por dia).
pub const RENOWN_PER_VISIT: u32 = 10;
/// Visitas que rendem Renome a quem ergueu, por dia (somando os faróis):
/// contas-sombra paradas na luz não viram fazenda.
pub const VISITS_PAID_PER_DAY: u32 = 10;
pub const BUILD_RENOWN: u32 = 30;
pub const BUILD_REASON: &str = "farol erguido";
pub const VISIT_REASON: &str = "farol visitado";
/// Mais rápido que isto não dá para trabalhar na costa (m/s).
const MAX_SPEED: f32 = 2.0;
const SWEEP_EVERY: f32 = 5.0;

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Lighthouse {
    pub id: u32,
    pub builder: CharacterId,
    pub x: f32,
    pub y: f32,
    /// Unix (s) em que apaga.
    pub expires_at: u64,
}

impl Lighthouse {
    fn at(&self) -> Vec2 {
        Vec2::new(self.x, self.y)
    }
}

#[derive(Resource, Default)]
pub struct Lighthouses {
    pub list: Vec<Lighthouse>,
    /// (farol, capitão, dia) que já renderam hoje.
    visited: HashSet<(u32, CharacterId, u32)>,
    /// (quem ergueu, dia) → visitas pagas hoje.
    paid_today: HashMap<(CharacterId, u32), u32>,
    /// Clients que já receberam a lista.
    told: HashSet<ClientId>,
    changed: bool,
    clock: f32,
    /// O banco não respondeu no boot: nada se ergue (ids e acesos
    /// desconhecidos, MV-067).
    is_unread: bool,
}

impl Lighthouses {
    /// Farol aceso cuja luz cobre (x, y).
    pub fn lit(&self, at: Vec2) -> bool {
        self.list.iter().any(|l| l.at().distance(at) <= LIGHT)
    }
}

fn now() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_or(0, |d| d.as_secs())
}

/// O que o capitão faz com a tecla aqui: reforçar o farol colado, erguer
/// um novo, ou o motivo da recusa.
#[derive(Debug, PartialEq)]
pub enum Plan {
    Tend(usize),
    Build,
    Refuse(&'static str),
}

pub fn plan(
    list: &[Lighthouse],
    map: &marvyr_domain_world::WorldMap,
    builder: CharacterId,
    at: Vec2,
) -> Plan {
    if let Some(index) = list.iter().position(|l| l.at().distance(at) <= TEND) {
        return Plan::Tend(index);
    }
    if list.iter().any(|l| l.at().distance(at) < SPACING) {
        return Plan::Refuse("Já há um farol perto daqui.");
    }
    let coastal = map
        .land()
        .iter()
        .any(|mass| mass.contains(at.x, at.y, COAST));
    if !coastal {
        return Plan::Refuse("Farol só perto da costa.");
    }
    if list.iter().filter(|l| l.builder == builder).count() >= MAX_PER_BUILDER {
        return Plan::Refuse("Você já tem 3 faróis acesos.");
    }
    Plan::Build
}

pub fn install(app: &mut App) {
    app.init_resource::<Lighthouses>()
        .add_systems(Startup, load)
        .add_systems(FixedUpdate, handle_raise.in_set(SimulationSet::Input))
        .add_systems(
            FixedUpdate,
            (sweep, broadcast).chain().in_set(SimulationSet::Snapshot),
        );
}

fn load(store: Res<StoreHandle>, mut lighthouses: ResMut<Lighthouses>) {
    // Dev: `MARVYR_DEV_LIGHTHOUSE=x,y` (fora de produção) acende um farol
    // de teste ali no boot (não grava).
    let dev = std::env::var("MARVYR_DEV_LIGHTHOUSE")
        .ok()
        .filter(|_| !std::env::var("MARVYR_ENV").is_ok_and(|env| env == "production"))
        .and_then(|raw| {
            let (x, y) = raw.split_once(',')?;
            Some((x.trim().parse::<f32>().ok()?, y.trim().parse::<f32>().ok()?))
        });
    if let Some((x, y)) = dev {
        lighthouses.list.push(Lighthouse {
            id: 0,
            builder: CharacterId::new(),
            x,
            y,
            expires_at: now() + LIFE_HOURS * 3_600,
        });
        lighthouses.changed = true;
    }
    let Some(store) = &store.0 else {
        return;
    };
    match store.load_lighthouses() {
        Ok(list) => {
            info!(count = list.len(), "faróis carregados");
            lighthouses.list.extend(list);
        }
        Err(error) => {
            warn!(%error, "faróis não carregaram do banco: ninguém ergue até reiniciar");
            lighthouses.is_unread = true;
        }
    }
}

/// B: reforça o farol colado ou ergue um novo. Grava antes de cobrar:
/// falhou o banco, ninguém perde material.
// System Bevy: params são injeção de dependência, não assinatura.
#[allow(clippy::too_many_arguments)]
fn handle_raise(
    mut events: EventReader<ServerReceiveMessage<RaiseLighthouse>>,
    mut ships: Query<&mut ServerShip>,
    dev: Res<DevItems>,
    map: Res<ServerWorldMap>,
    store: Res<StoreHandle>,
    mut lighthouses: ResMut<Lighthouses>,
    mut renown: EventWriter<crate::renown::RenownEarned>,
    mut discoveries: EventWriter<crate::progress::Discovered>,
    mut connection_manager: ResMut<ConnectionManager>,
) {
    for event in events.read() {
        let client_id = event.from();
        let Some(mut ship) = ships.iter_mut().find(|s| s.client_id == Some(client_id)) else {
            continue;
        };
        let refuse = |cm: &mut ConnectionManager, reason: &str| {
            send_action(cm, client_id, ActionKind::Lighthouse, false, reason);
        };
        if ship.presence != VesselPresence::AtSea {
            refuse(&mut connection_manager, "Farol só no mar.");
            continue;
        }
        if ship.motion.speed.abs() > MAX_SPEED {
            refuse(&mut connection_manager, "Recolha as velas para trabalhar.");
            continue;
        }
        if lighthouses.is_unread {
            refuse(
                &mut connection_manager,
                "Os faróis estão fora do ar. Tente mais tarde.",
            );
            continue;
        }
        let at = Vec2::new(ship.motion.x, ship.motion.y);
        let now = now();
        match plan(&lighthouses.list, &map.0, ship.character, at) {
            Plan::Refuse(reason) => refuse(&mut connection_manager, reason),
            Plan::Tend(index) => {
                let remaining = in_hold(&ship.hold, dev.timber);
                if remaining < TEND_TIMBER {
                    refuse(
                        &mut connection_manager,
                        &format!("Reforçar pede {TEND_TIMBER} Madeira no porão."),
                    );
                    continue;
                }
                let mut next = lighthouses.list[index];
                next.expires_at =
                    (next.expires_at.max(now) + TEND_HOURS * 3_600).min(now + MAX_HOURS * 3_600);
                if let Some(store) = &store.0 {
                    if let Err(error) = store.save_lighthouse(&next) {
                        warn!(%error, "farol não gravou");
                        refuse(
                            &mut connection_manager,
                            "O farol não respondeu. Tente de novo.",
                        );
                        continue;
                    }
                }
                let _ = ship.hold.remove(dev.timber, TEND_TIMBER);
                // O farol já gravou: o porão sem a madeira grava logo atrás
                // (antes do checkpoint, um crash devolvia o material).
                crate::net::save_ship_now(&store, &ship);
                lighthouses.list[index] = next;
                lighthouses.changed = true;
                send_action(
                    &mut connection_manager,
                    client_id,
                    ActionKind::Lighthouse,
                    true,
                    format!("Farol reforçado: {}h aceso.", hours_left(&next, now)),
                );
            }
            Plan::Build => {
                let timber = in_hold(&ship.hold, dev.timber);
                let ore = in_hold(&ship.hold, dev.ore);
                if timber < BUILD_TIMBER || ore < BUILD_ORE {
                    refuse(
                        &mut connection_manager,
                        &format!(
                            "Erguer pede {BUILD_TIMBER} Madeira e {BUILD_ORE} Minério no porão."
                        ),
                    );
                    continue;
                }
                let lighthouse = Lighthouse {
                    id: lighthouses.list.iter().map(|l| l.id).max().unwrap_or(0) + 1,
                    builder: ship.character,
                    x: at.x,
                    y: at.y,
                    expires_at: now + LIFE_HOURS * 3_600,
                };
                if let Some(store) = &store.0 {
                    if let Err(error) = store.save_lighthouse(&lighthouse) {
                        warn!(%error, "farol não gravou");
                        refuse(
                            &mut connection_manager,
                            "O farol não respondeu. Tente de novo.",
                        );
                        continue;
                    }
                }
                let _ = ship.hold.remove(dev.timber, BUILD_TIMBER);
                let _ = ship.hold.remove(dev.ore, BUILD_ORE);
                crate::net::save_ship_now(&store, &ship);
                lighthouses.list.push(lighthouse);
                lighthouses.changed = true;
                info!(id = lighthouse.id, x = at.x, y = at.y, "farol erguido");
                renown.send(crate::renown::RenownEarned {
                    character: ship.character,
                    amount: BUILD_RENOWN,
                    reason: BUILD_REASON,
                });
                discoveries.send(crate::progress::Discovered {
                    character: ship.character,
                    entry: "Farol erguido",
                });
                send_action(
                    &mut connection_manager,
                    client_id,
                    ActionKind::Lighthouse,
                    true,
                    format!("Farol erguido: {LIFE_HOURS}h aceso."),
                );
            }
        }
    }
}

fn in_hold(
    hold: &marvyr_domain_items::CargoHold,
    item: marvyr_shared::ids::ItemDefinitionId,
) -> u32 {
    marvyr_domain_items::storage::quantity_of(hold.items(), item)
}

fn hours_left(lighthouse: &Lighthouse, now: u64) -> u32 {
    u32::try_from(lighthouse.expires_at.saturating_sub(now).div_ceil(3_600)).unwrap_or(u32::MAX)
}

/// Apaga os vencidos e paga quem ergueu pelos capitães na luz.
fn sweep(
    time: Res<Time>,
    ships: Query<&ServerShip>,
    store: Res<StoreHandle>,
    mut lighthouses: ResMut<Lighthouses>,
    mut renown: EventWriter<crate::renown::RenownEarned>,
) {
    lighthouses.clock += time.delta_secs();
    if lighthouses.clock < SWEEP_EVERY {
        return;
    }
    lighthouses.clock = 0.0;
    let now = now();
    let (day, _) = crate::progress::today();
    let expired: Vec<u32> = lighthouses
        .list
        .iter()
        .filter(|l| l.expires_at <= now)
        .map(|l| l.id)
        .collect();
    for id in expired {
        if let Some(store) = &store.0 {
            if let Err(error) = store.remove_lighthouse(id) {
                warn!(%error, id, "farol apagado não saiu do banco; tenta de novo");
                continue;
            }
        }
        lighthouses.list.retain(|l| l.id != id);
        lighthouses.changed = true;
        info!(id, "farol apagou");
    }
    lighthouses.visited.retain(|(_, _, d)| *d == day);
    let mut paid = Vec::new();
    for ship in &ships {
        if ship.client_id.is_none() || ship.presence != VesselPresence::AtSea {
            continue;
        }
        let at = Vec2::new(ship.motion.x, ship.motion.y);
        for lighthouse in &lighthouses.list {
            if lighthouse.builder == ship.character || lighthouse.at().distance(at) > LIGHT {
                continue;
            }
            paid.push((lighthouse.id, lighthouse.builder, ship.character));
        }
    }
    // ponytail: visitas e teto só em memória; restart reabre o dia (o teto
    // por dia segura o estrago). Gravar se deploy virar coisa de todo dia.
    lighthouses.paid_today.retain(|(_, d), _| *d == day);
    for (id, builder, visitor) in paid {
        if lighthouses.visited.insert((id, visitor, day)) {
            let count = lighthouses.paid_today.entry((builder, day)).or_default();
            if *count >= VISITS_PAID_PER_DAY {
                continue;
            }
            *count += 1;
            renown.send(crate::renown::RenownEarned {
                character: builder,
                amount: RENOWN_PER_VISIT,
                reason: VISIT_REASON,
            });
        }
    }
}

fn broadcast(
    ships: Query<&ServerShip>,
    mut lighthouses: ResMut<Lighthouses>,
    mut connection_manager: ResMut<ConnectionManager>,
) {
    let clients: Vec<ClientId> = ships.iter().filter_map(|s| s.client_id).collect();
    let fresh: Vec<ClientId> = clients
        .iter()
        .copied()
        .filter(|c| !lighthouses.told.contains(c))
        .collect();
    if !lighthouses.changed && fresh.is_empty() {
        return;
    }
    let now = now();
    let update = LighthousesUpdate {
        list: lighthouses
            .list
            .iter()
            .map(|l| LighthouseLine {
                id: l.id,
                x: l.x,
                y: l.y,
                hours_left: hours_left(l, now),
                builder: crate::season::captain_label(l.builder),
            })
            .collect(),
    };
    let targets = if lighthouses.changed {
        clients.clone()
    } else {
        fresh
    };
    for client in targets {
        let _ = connection_manager.send_message::<ReliableChannel, _>(client, &update);
    }
    lighthouses.changed = false;
    lighthouses.told = clients.into_iter().collect();
}

#[cfg(test)]
mod tests {
    use super::*;
    use marvyr_domain_world::WorldMap;

    fn coast_point(map: &WorldMap) -> Vec2 {
        let mass = map.land().iter().find(|m| !m.cliff).expect("terra");
        Vec2::new(mass.x + mass.radius + 40.0, mass.y)
    }

    #[test]
    fn plan_builds_on_the_coast_and_tends_next_to_one() {
        let map = WorldMap::from_seed(0);
        let captain = CharacterId::new();
        let at = coast_point(&map);
        assert_eq!(plan(&[], &map, captain, at), Plan::Build);
        let lit = Lighthouse {
            id: 1,
            builder: CharacterId::new(),
            x: at.x,
            y: at.y,
            expires_at: u64::MAX,
        };
        assert_eq!(
            plan(&[lit], &map, captain, at + Vec2::X * 50.0),
            Plan::Tend(0)
        );
        assert!(matches!(
            plan(&[lit], &map, captain, at + Vec2::X * 200.0),
            Plan::Refuse(_)
        ));
    }

    #[test]
    fn open_sea_and_a_fourth_lighthouse_are_refused() {
        let map = WorldMap::from_seed(0);
        let captain = CharacterId::new();
        // Mar aberto: o centro de um setor de mar, longe de toda terra.
        let sector = map.features().sea_sectors[0];
        let open = Vec2::new((sector.0 + sector.1) / 2.0, (sector.2 + sector.3) / 2.0);
        if !map.land().iter().any(|m| m.contains(open.x, open.y, COAST)) {
            assert!(matches!(plan(&[], &map, captain, open), Plan::Refuse(_)));
        }
        let at = coast_point(&map);
        let mine: Vec<Lighthouse> = (0..3)
            .map(|i| Lighthouse {
                id: i + 1,
                builder: captain,
                x: at.x + 10_000.0 * (i + 1) as f32,
                y: at.y,
                expires_at: u64::MAX,
            })
            .collect();
        assert_eq!(
            plan(&mine, &map, captain, at),
            Plan::Refuse("Você já tem 3 faróis acesos.")
        );
    }
}
