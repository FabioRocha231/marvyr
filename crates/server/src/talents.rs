//! Rosa dos Ventos no servidor (MV-067). As regras da árvore vivem em
//! `domain-ships::talents`; aqui: carrega e grava o que o capitão aprendeu,
//! valida os pontos contra o nível de Renome e aplica o bônus nos stats do
//! navio.
//!
//! Aplicar: quem recalcula stats (spawn, restore, troca de equipamento)
//! grava os stats base em `ServerShip.stats`. `apply_to_ships` percebe que
//! eles não são os que ele deixou lá e reaplica o bônus por cima — nenhum
//! desses caminhos precisa saber que a árvore existe.

use std::collections::HashMap;

use bevy::prelude::*;
use lightyear::prelude::server::*;
use lightyear::prelude::*;
use marvyr_domain_ships::talents::{
    can_allocate, points_for_level, respec_cost, TalentBonus, RESPEC_ITEM,
};
use marvyr_domain_ships::{compute_ship_stats, rescale_hp, ShipStats, VesselPresence};
use marvyr_protocol::{ActionKind, AllocateTalent, RespecTalents, TalentsSnapshot};
use marvyr_shared::ids::{CharacterId, ItemDefinitionId, RegionId};
use tracing::{info, warn};

use crate::market::ServerMarket;
use crate::net::{DevItems, ReliableChannel, ServerShip};
use crate::persist::StoreHandle;
use crate::renown::CaptainRenown;
use crate::seafaring::send_action;
use crate::sets::SimulationSet;

#[derive(Debug, Clone, Default)]
struct Learned {
    allocated: Vec<String>,
    /// Conexão que já recebeu o snapshot (reconexão recarrega).
    client: Option<ClientId>,
    /// O banco não respondeu no connect: a lista em memória não é a
    /// verdade, então nada muda (nem grava) até reconectar.
    is_unread: bool,
}

#[derive(Resource, Default)]
pub struct CaptainTalents {
    captains: HashMap<CharacterId, Learned>,
}

impl CaptainTalents {
    /// Teste: talentos já aprendidos, sem banco.
    #[cfg(test)]
    pub(crate) fn learned_for_test(&mut self, character: CharacterId, allocated: &[&str]) {
        self.captains.entry(character).or_default().allocated =
            allocated.iter().map(|id| (*id).to_owned()).collect();
    }

    /// v34: classe do capitão (nó-mestre aprendido).
    pub fn class(
        &self,
        character: CharacterId,
    ) -> Option<marvyr_domain_ships::talents::CaptainClass> {
        self.captains
            .get(&character)
            .and_then(|learned| marvyr_domain_ships::talents::class_of(&learned.allocated))
    }

    pub fn bonus(&self, character: CharacterId) -> TalentBonus {
        self.captains
            .get(&character)
            .map_or_else(TalentBonus::default, |learned| {
                TalentBonus::of(&learned.allocated)
            })
    }
}

const UNREAD: &str = "Talentos indisponíveis agora; reconecte e tente de novo.";
const SAVE_FAILED: &str = "Não deu para gravar agora; tente de novo.";

/// O que `apply_to_ships` deixou no navio da última vez.
#[derive(Component)]
struct TalentsApplied {
    bonus: TalentBonus,
    stats: ShipStats,
}

pub fn install(app: &mut App) {
    app.init_resource::<CaptainTalents>();
    app.add_systems(
        FixedUpdate,
        (
            load_on_connect,
            handle_allocate,
            handle_respec,
            apply_to_ships,
        )
            .chain()
            .in_set(SimulationSet::Input),
    );
}

fn send_snapshot(
    connection_manager: &mut ConnectionManager,
    client_id: ClientId,
    allocated: &[String],
) {
    let _ = connection_manager.send_message::<ReliableChannel, _>(
        client_id,
        &TalentsSnapshot {
            allocated: allocated.to_vec(),
        },
    );
}

fn load_on_connect(
    ships: Query<&ServerShip>,
    store: Res<StoreHandle>,
    mut talents: ResMut<CaptainTalents>,
    mut connection_manager: ResMut<ConnectionManager>,
) {
    for ship in &ships {
        let Some(client_id) = ship.client_id else {
            continue;
        };
        if talents
            .captains
            .get(&ship.character)
            .is_some_and(|learned| learned.client == Some(client_id))
        {
            continue;
        }
        // Toda mudança grava na hora: o banco é a verdade na reconexão.
        let (allocated, is_unread) = match store.0.as_ref() {
            Some(store) => match store.load_talents(ship.character) {
                Ok(allocated) => (allocated, false),
                Err(error) => {
                    warn!(%error, "talentos não carregaram: sessão sem bônus");
                    (Vec::new(), true)
                }
            },
            None => (
                talents
                    .captains
                    .get(&ship.character)
                    .map(|learned| learned.allocated.clone())
                    .unwrap_or_default(),
                false,
            ),
        };
        send_snapshot(&mut connection_manager, client_id, &allocated);
        talents.captains.insert(
            ship.character,
            Learned {
                allocated,
                client: Some(client_id),
                is_unread,
            },
        );
    }
}

fn save(store: &StoreHandle, character: CharacterId, allocated: &[String]) -> Result<(), String> {
    match store.0.as_ref() {
        Some(store) => store.save_talents(character, allocated),
        None => Ok(()),
    }
}

/// Aprende `node`: valida, grava e só então vale. `Err` é o motivo PT-BR.
fn learn(
    learned: &mut Learned,
    character: CharacterId,
    node: &str,
    points: u32,
    store: &StoreHandle,
) -> Result<(), &'static str> {
    if learned.is_unread {
        return Err(UNREAD);
    }
    can_allocate(&learned.allocated, node, points).map_err(|error| error.reason())?;
    let mut next = learned.allocated.clone();
    next.push(node.to_owned());
    save(store, character, &next).map_err(|error| {
        warn!(%error, "talento não foi gravado");
        SAVE_FAILED
    })?;
    learned.allocated = next;
    Ok(())
}

/// Esquece tudo pagando minério do armazém do porto (sink). Grava antes de
/// cobrar: se o banco falhar, ninguém paga por nada. Devolve o custo; `Err`
/// é o motivo PT-BR.
fn forget_all(
    learned: &mut Learned,
    character: CharacterId,
    docked: Option<RegionId>,
    ore: ItemDefinitionId,
    market: &mut ServerMarket,
    store: &StoreHandle,
) -> Result<u32, String> {
    let Some(region) = docked else {
        return Err("Redistribuir só no porto: atraque primeiro.".into());
    };
    if learned.is_unread {
        return Err(UNREAD.into());
    }
    if learned.allocated.is_empty() {
        return Err("Nenhum talento para esquecer.".into());
    }
    let cost = respec_cost(learned.allocated.len());
    if market.storage_quantity(character, region, ore) < cost {
        return Err(format!(
            "Falta {RESPEC_ITEM} no armazém: redistribuir custa {cost}."
        ));
    }
    save(store, character, &[]).map_err(|error| {
        warn!(%error, "respec não foi gravado");
        String::from(SAVE_FAILED)
    })?;
    market
        .consume_from_storage(character, region, ore, cost)
        .expect("estoque conferido acima, no mesmo tick");
    learned.allocated.clear();
    Ok(cost)
}

fn handle_allocate(
    mut events: EventReader<ServerReceiveMessage<AllocateTalent>>,
    ships: Query<&ServerShip>,
    renown: Res<CaptainRenown>,
    store: Res<StoreHandle>,
    mut talents: ResMut<CaptainTalents>,
    mut connection_manager: ResMut<ConnectionManager>,
) {
    for event in events.read() {
        let client_id = event.from();
        let Some(ship) = ships.iter().find(|ship| ship.client_id == Some(client_id)) else {
            continue;
        };
        let character = ship.character;
        let node = &event.message().node;
        let points = points_for_level(renown.level(character));
        let learned = talents.captains.entry(character).or_default();
        match learn(learned, character, node, points, &store) {
            Ok(()) => {
                info!(?character, node, "talento aprendido");
                send_snapshot(&mut connection_manager, client_id, &learned.allocated);
            }
            Err(reason) => send_action(
                &mut connection_manager,
                client_id,
                ActionKind::Talent,
                false,
                reason,
            ),
        }
    }
}

fn handle_respec(
    mut events: EventReader<ServerReceiveMessage<RespecTalents>>,
    ships: Query<&ServerShip>,
    store: Res<StoreHandle>,
    mut market: ResMut<ServerMarket>,
    mut talents: ResMut<CaptainTalents>,
    mut connection_manager: ResMut<ConnectionManager>,
    dev: Res<DevItems>,
) {
    for event in events.read() {
        let client_id = event.from();
        let Some(ship) = ships.iter().find(|ship| ship.client_id == Some(client_id)) else {
            continue;
        };
        let character = ship.character;
        let docked = match ship.presence {
            VesselPresence::Docked(region) => Some(region),
            VesselPresence::AtSea => None,
        };
        let learned = talents.captains.entry(character).or_default();
        let cost = match forget_all(learned, character, docked, dev.ore, &mut market, &store) {
            Ok(cost) => cost,
            Err(reason) => {
                send_action(
                    &mut connection_manager,
                    client_id,
                    ActionKind::Talent,
                    false,
                    reason,
                );
                continue;
            }
        };
        info!(?character, cost, "talentos redistribuídos");
        send_snapshot(&mut connection_manager, client_id, &[]);
        send_action(
            &mut connection_manager,
            client_id,
            ActionKind::Talent,
            true,
            format!("Rosa dos Ventos zerada (-{cost} {RESPEC_ITEM}). Pontos de volta."),
        );
    }
}

fn apply_to_ships(
    mut commands: Commands,
    talents: Res<CaptainTalents>,
    dev_ships: Res<crate::crafting::DevShips>,
    dev: Res<DevItems>,
    mut ships: Query<(Entity, &mut ServerShip, Option<&TalentsApplied>)>,
) {
    for (entity, mut ship, applied) in &mut ships {
        let bonus = talents.bonus(ship.character);
        if applied.is_some_and(|applied| applied.bonus == bonus && applied.stats == ship.stats) {
            continue;
        }
        let Ok(base) = compute_ship_stats(
            dev_ships.definition(ship.kind),
            &ship.loadout.components(),
            &dev.catalog,
        ) else {
            continue;
        };
        let stats = bonus.apply(&base);
        // Mesma regra do equipamento: a fração de casco se mantém (navio
        // novo, inteiro, nasce de casco cheio com o bônus).
        ship.hp = rescale_hp(ship.hp, ship.stats.max_hp, stats.max_hp);
        ship.hold.set_capacity(stats.cargo_capacity);
        ship.stats = stats.clone();
        commands
            .entity(entity)
            .insert(TalentsApplied { bonus, stats });
    }
}

#[cfg(test)]
mod tests {
    use bevy::ecs::system::RunSystemOnce;
    use marvyr_domain_ships::ShipKind;
    use marvyr_domain_world::WorldMap;

    use super::*;
    use crate::crafting::DevShips;
    use crate::net::{spawn_ship_for, ShipIdCounter, DEFAULT_WORLD_SEED};

    fn teach(app: &mut App, character: CharacterId, allocated: &[&str]) {
        app.world_mut()
            .resource_mut::<CaptainTalents>()
            .captains
            .insert(
                character,
                Learned {
                    allocated: allocated.iter().map(|id| id.to_string()).collect(),
                    ..Learned::default()
                },
            );
    }

    fn ship(app: &mut App) -> (ShipStats, u32) {
        let world = app.world_mut();
        let mut ships = world.query::<&ServerShip>();
        let ship = ships.single(world);
        (ship.stats.clone(), ship.hp)
    }

    #[test]
    fn bonus_rides_on_top_of_base_stats_and_survives_recalc() {
        let mut app = App::new();
        app.insert_resource(DevItems::new())
            .insert_resource(DevShips::new())
            .init_resource::<CaptainTalents>()
            .add_systems(Update, apply_to_ships);
        let character = CharacterId::new();
        teach(&mut app, character, &["com.olho", "com.estiva"]);
        app.world_mut()
            .run_system_once(
                move |mut commands: Commands, dev: Res<DevItems>, ships: Res<DevShips>| {
                    let map = WorldMap::from_seed(DEFAULT_WORLD_SEED);
                    spawn_ship_for(
                        &mut commands,
                        &mut ShipIdCounter(1),
                        &dev,
                        &ships,
                        &map,
                        ShipKind::SmallMerchant,
                        None,
                        character,
                        Vec::new(),
                        None,
                    );
                },
            )
            .expect("spawn");
        app.update();
        let base = DevShips::new()
            .definition(ShipKind::SmallMerchant)
            .cargo_capacity;
        let (boosted, hp) = ship(&mut app);
        assert_eq!(boosted.cargo_capacity, base * 105 / 100);
        assert_eq!(hp, boosted.max_hp, "nasce de casco cheio");

        // Troca de equipamento grava stats base: o bônus volta por cima.
        {
            let world = app.world_mut();
            let mut ships = world.query::<&mut ServerShip>();
            let mut ship = ships.single_mut(world);
            ship.stats.cargo_capacity = base;
            ship.hp = 10;
        }
        teach(
            &mut app,
            character,
            &["com.olho", "com.estiva", "nav.leme", "nav.costado"],
        );
        app.update();
        let (again, hp) = ship(&mut app);
        assert_eq!(again.cargo_capacity, base * 105 / 100);
        assert!(again.max_hp > boosted.max_hp);
        assert_eq!(
            hp,
            rescale_hp(10, boosted.max_hp, again.max_hp),
            "casco maior mantém a fração, não cura"
        );
    }

    #[test]
    fn learning_and_respec_charge_only_what_was_saved() {
        let store = StoreHandle(None);
        let character = CharacterId::new();
        let mut learned = Learned::default();
        assert!(learn(&mut learned, character, "nav.leme", 0, &store).is_err());
        learn(&mut learned, character, "nav.leme", 2, &store).expect("aprende");
        learn(&mut learned, character, "nav.pano", 2, &store).expect("aprende");
        assert_eq!(learned.allocated, ["nav.leme", "nav.pano"]);

        let mut market = ServerMarket::new();
        let region = RegionId::new();
        let (catalog, ore) = ore_catalog();
        let stock = |market: &mut ServerMarket, quantity| {
            market.grant_to_storage(character, region, ore, quantity, &catalog)
        };
        stock(&mut market, 5);
        // No mar, ou sem minério bastante: nada muda.
        assert!(forget_all(&mut learned, character, None, ore, &mut market, &store).is_err());
        assert!(forget_all(
            &mut learned,
            character,
            Some(region),
            ore,
            &mut market,
            &store
        )
        .is_err());
        assert_eq!(
            (
                market.storage_quantity(character, region, ore),
                learned.allocated.len()
            ),
            (5, 2)
        );
        stock(&mut market, 3);
        let cost = forget_all(
            &mut learned,
            character,
            Some(region),
            ore,
            &mut market,
            &store,
        )
        .expect("paga");
        assert_eq!(cost, 6);
        assert_eq!(market.storage_quantity(character, region, ore), 2);
        assert!(learned.allocated.is_empty());

        // Banco não respondeu no connect: nada muda até reconectar.
        let mut unread = Learned {
            allocated: vec![String::from("nav.leme")],
            is_unread: true,
            ..Learned::default()
        };
        assert!(learn(&mut unread, character, "nav.pano", 5, &store).is_err());
        assert!(forget_all(
            &mut unread,
            character,
            Some(region),
            ore,
            &mut market,
            &store
        )
        .is_err());
        assert_eq!(market.storage_quantity(character, region, ore), 2);
    }

    fn ore_catalog() -> (marvyr_domain_items::ItemCatalog, ItemDefinitionId) {
        let dev = DevItems::new();
        (dev.catalog.clone(), dev.ore)
    }
}
