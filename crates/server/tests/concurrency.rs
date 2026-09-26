//! Testes de concorrência (PRD §70, MF-028): duplo clique, retry, duplo
//! loot, duplo craft e disconnect. O servidor é autoritativo e
//! single-threaded por tick — a ameaça real é a INTENÇÃO DUPLICADA chegando
//! no mesmo tick. Cada teste prova que apenas uma operação vence.

use chrono::Utc;
use marvyr_domain_combat::{resolve_ship_destruction, LootPolicy, SurvivorItem, WreckChest};
use marvyr_domain_crafting::{craft_in_storage, Ingredient, Recipe, StationKind};
use marvyr_domain_economy::MarketError;
use marvyr_domain_items::{
    CargoHold, Custody, ItemCatalog, ItemDefinition, ItemInstance, ItemKind, ItemLocation,
};
use marvyr_server::market::ServerMarket;
use marvyr_shared::ids::{
    CharacterId, DestructionEventId, ItemDefinitionId, ItemInstanceId, RecipeId, RegionId,
    ShipInstanceId, WreckId,
};

fn wood_definition() -> (ItemCatalog, ItemDefinitionId) {
    let (catalog, wood, _) = wood_and_ore();
    (catalog, wood)
}

fn wood_and_ore() -> (ItemCatalog, ItemDefinitionId, ItemDefinitionId) {
    let mut catalog = ItemCatalog::default();
    let mut ids = Vec::new();
    for name in ["Madeira", "Minério"] {
        let id = ItemDefinitionId::new();
        catalog
            .register(ItemDefinition {
                id,
                kind: ItemKind::Resource,
                equipment: None,
                max_stack: 100,
                base_weight: 2,
                tags: Default::default(),
                display_name: String::from(name),
            })
            .unwrap();
        ids.push(id);
    }
    (catalog, ids[0], ids[1])
}

/// Mundo mínimo: catálogo + dois personagens com 20 de madeira e 10 de
/// minério depositados.
struct World {
    market: ServerMarket,
    catalog: ItemCatalog,
    wood: ItemDefinitionId,
    ore: ItemDefinitionId,
    region: RegionId,
    a: CharacterId,
    b: CharacterId,
}

impl World {
    fn new() -> Self {
        let (catalog, wood, ore) = wood_and_ore();
        let mut market = ServerMarket::new();
        let a = market.character("token-a");
        let b = market.character("token-b");
        let region = RegionId::new();

        for character in [a, b] {
            let mut hold = CargoHold::new(ShipInstanceId::new(), 100);
            hold.insert(
                &catalog,
                ItemInstance::new_resource(ItemInstanceId::new(), wood, 20),
            )
            .unwrap();
            hold.insert(
                &catalog,
                ItemInstance::new_resource(ItemInstanceId::new(), ore, 10),
            )
            .unwrap();
            market
                .deposit_all(character, region, &mut hold, &catalog)
                .unwrap();
        }

        Self {
            market,
            catalog,
            wood,
            ore,
            region,
            a,
            b,
        }
    }
}

/// §70 double buy: dois cliques (ou dois compradores) na mesma oferta.
/// Apenas uma troca vence; a segunda é recusada sem efeito.
#[test]
fn double_accept_of_an_offer_wins_once() {
    let mut world = World::new();

    // A oferece 1 madeira por 2 minério.
    let order_num = world
        .market
        .create_order(world.a, world.region, world.wood, 1, world.ore, 2)
        .unwrap();

    // Primeiro aceite vence.
    assert!(world.market.buy(world.b, world.region, order_num).is_ok());

    // Duplo clique do MESMO comprador: a oferta já saiu do board — o
    // segundo intent bate em OrderNotOpen e nada mais é pago.
    assert_eq!(
        world.market.buy(world.b, world.region, order_num),
        Err(MarketError::OrderNotOpen)
    );

    // Ninguém pagou duas vezes e o vendedor recebeu uma só vez.
    let quantity =
        |market: &ServerMarket, who, item| market.storage_quantity(who, world.region, item);
    assert_eq!(quantity(&world.market, world.b, world.ore), 10 - 2);
    assert_eq!(quantity(&world.market, world.b, world.wood), 20 + 1);
    assert_eq!(quantity(&world.market, world.a, world.ore), 10 + 2);
    assert_eq!(quantity(&world.market, world.a, world.wood), 20 - 1);
}

/// §70 double loot: dois eventos de loot no mesmo tick. O baú drena uma
/// vez; o segundo encontra o vazio e não duplica item.
#[test]
fn double_loot_transfers_cargo_once() {
    let (catalog, wood) = wood_definition();
    let mut chest = WreckChest::new(WreckId::new());
    chest.insert(
        SurvivorItem {
            definition: wood,
            quantity: 5,
            durability: None,
            quality: None,
        },
        ItemInstanceId::new(),
    );

    let mut hold = CargoHold::new(ShipInstanceId::new(), 100);

    // Primeiro LootWreck do tick: vence e drena o baú.
    let incoming = chest.drain();
    assert_eq!(incoming.len(), 1);
    hold.take_all(&catalog, incoming).unwrap();

    // Segundo LootWreck do tick (despawn do Bevy ainda é deferido):
    // baú vazio — nada a transferir, e é aqui que o guard barra o duplo.
    assert!(chest.is_empty());
    assert!(chest.drain().is_empty());
    assert_eq!(hold.items().len(), 1);
}

/// §70 double craft no fluxo de oficina (MF-037): dois cliques consumindo
/// os MESMOS insumos do storage. O primeiro vence; o segundo falha sem
/// produzir item duplicado.
#[test]
fn double_craft_consumes_ingredients_once() {
    let (catalog, wood) = wood_definition();
    let recipe = Recipe {
        id: RecipeId::new(),
        display_name: String::from("teste"),
        output_item: wood,
        output_quantity: 1,
        ingredients: vec![Ingredient {
            item: wood,
            quantity: 15,
        }],
        required_station: StationKind::Workbench,
        craft_time_secs: 0,
        output_rarity: Default::default(),
        output_tier: 1,
    };

    let region = RegionId::new();
    let mut storage = vec![Custody {
        instance: ItemInstance::new_resource(ItemInstanceId::new(), wood, 15),
        location: ItemLocation::PortStorage(region),
    }];

    assert!(craft_in_storage(
        &recipe,
        &mut storage,
        &catalog,
        StationKind::Workbench,
        region
    )
    .is_ok());
    assert!(craft_in_storage(
        &recipe,
        &mut storage,
        &catalog,
        StationKind::Workbench,
        region
    )
    .is_err());
    // Exatamente 1 unidade produzida (15 consumidas, 1 gerada).
    let total: u32 = storage
        .iter()
        .map(|custody| custody.instance.quantity)
        .sum();
    assert_eq!(total, 1);
}

/// §70 retry: reenviar a listagem depois de sucesso não duplica escrow —
/// sem estoque restante, a segunda falha com NotInStorage.
#[test]
fn retry_of_sell_order_does_not_duplicate_escrow() {
    let mut world = World::new();
    assert!(world
        .market
        .create_order(world.a, world.region, world.wood, 20, world.ore, 5)
        .is_ok());
    assert_eq!(
        world
            .market
            .create_order(world.a, world.region, world.wood, 20, world.ore, 5),
        Err(MarketError::NotInStorage)
    );
}

/// §70 + MF-035: a conexão cai, mas o armazém é da personagem e sobrevive;
/// reconectar com o MESMO TOKEN de identidade encontra o mesmo personagem —
/// conexão nunca foi dona de nada.
#[test]
fn disconnect_then_reconnect_keeps_storage() {
    let mut world = World::new();
    let reconnected = world.market.character("token-a");
    assert_eq!(reconnected, world.a);
    assert_eq!(
        world
            .market
            .storage_quantity(reconnected, world.region, world.wood),
        20
    );

    // Token diferente = personagem diferente (fail-closed por identidade).
    let stranger = world.market.character("token-43");
    assert_ne!(stranger, world.a);
}

/// Full loot + mercado: a troca move os dois lados e a resolução de
/// destruição continua determinística.
#[test]
fn loot_and_market_keep_every_unit_accounted() {
    let mut world = World::new();
    let order_num = world
        .market
        .create_order(world.a, world.region, world.wood, 10, world.ore, 10)
        .unwrap();
    world.market.buy(world.b, world.region, order_num).unwrap();

    // Nenhuma unidade surge nem some numa troca entre jogadores.
    let total = |item| {
        [world.a, world.b]
            .iter()
            .map(|who| world.market.storage_quantity(*who, world.region, item))
            .sum::<u32>()
    };
    assert_eq!((total(world.wood), total(world.ore)), (40, 20));

    // 10 de madeira no porão geram sobreviventes no wreck (~80%).
    let mut hold = CargoHold::new(ShipInstanceId::new(), 100);
    hold.insert(
        &world.catalog,
        ItemInstance::new_resource(ItemInstanceId::new(), world.wood, 10),
    )
    .unwrap();
    let cargo: Vec<_> = hold
        .items()
        .iter()
        .map(|custody| custody.instance.clone())
        .collect();
    let outcome = resolve_ship_destruction(
        DestructionEventId::new(),
        &[],
        &cargo,
        &LootPolicy::default(),
    );
    assert!(!outcome.wreck_items.is_empty());
}

/// Silencia avisos de import condicional (Utc entra via MarketOrder serde).
#[allow(dead_code)]
fn _touch(_: Utc) {}
