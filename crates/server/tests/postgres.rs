//! Integração do PostgresStateStore (MF-034, ADR-0004/0010).
//!
//! Roda contra um PostgreSQL real (Docker serve):
//!
//! ```text
//! docker run --rm -d --name marvyr-pg -p 54329:5432 \
//!   -e POSTGRES_PASSWORD=marvyr postgres:16-alpine
//! MARVYR_TEST_DATABASE_URL=postgres://postgres:marvyr@localhost:54329/postgres \
//!   cargo test -p marvyr-server --test postgres
//! ```
//!
//! Sem a variável, o teste pula com aviso — CI sem banco não quebra, mas a
//! verificação do adapter também não acontece (reporte honesto > falso verde).

use std::collections::HashMap;
use std::sync::{Arc, Mutex, OnceLock};

use chrono::Utc;
use marvyr_domain_economy::{MarketOrder, OrderStatus};
use marvyr_domain_items::{
    CargoHold, Custody, ItemCatalog, ItemDefinition, ItemInstance, ItemKind, ItemLocation,
};
use marvyr_domain_ships::ShipKind;
use marvyr_server::market::{MarketSnapshot, ServerMarket};
use marvyr_server::persist::{PostgresStateStore, ShipRecord, StateStore};
use marvyr_shared::ids::{
    CharacterId, ItemDefinitionId, ItemInstanceId, MarketOrderId, RegionId, ShipInstanceId,
};

/// Os três testes compartilham UM banco (o estado é global por natureza):
/// mutex serializa e o reset limpa o estado antes de cada cenário.
fn test_lock() -> std::sync::MutexGuard<'static, ()> {
    static LOCK: OnceLock<Mutex<()>> = OnceLock::new();
    LOCK.get_or_init(|| Mutex::new(()))
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner())
}

/// Limpa todas as tabelas (o ledger é append-only no jogo; no TESTE o
/// cenário começa limpo para as asserções serem exatas).
fn reset_database(url: &str) {
    let runtime = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .expect("runtime de teste");
    runtime.block_on(async {
        let pool = sqlx::PgPool::connect(url).await.expect("pool de reset");
        sqlx::query(
            "TRUNCATE accounts, characters, ship_instances, item_instances, \
             ledger_entries, market_orders, wallets CASCADE",
        )
        .execute(&pool)
        .await
        .expect("reset do banco de teste");
        pool.close().await;
    });
}

fn store_or_skip() -> Option<(Arc<PostgresStateStore>, String)> {
    match std::env::var("MARVYR_TEST_DATABASE_URL") {
        Ok(url) => {
            // Conectar PRIMEIRO (roda as migrations), resetar DEPOIS.
            let store = match PostgresStateStore::connect(&url) {
                Ok(store) => store,
                Err(error) => panic!("banco de teste configurado mas não abriu: {error}"),
            };
            reset_database(&url);
            Some((Arc::new(store), url))
        }
        Err(_) => {
            eprintln!("PULANDO: defina MARVYR_TEST_DATABASE_URL para testar o PostgresStateStore");
            None
        }
    }
}

fn sample_snapshot() -> (MarketSnapshot, CharacterId) {
    let character = CharacterId::new();
    let region = RegionId::new();
    let item = ItemDefinitionId::new();
    let ask_item = ItemDefinitionId::new();
    let order_id = MarketOrderId::new();
    let order_num = 0u32;

    let mut identities = HashMap::new();
    identities.insert("token-alfa".to_string(), character);

    let stack = Custody {
        instance: ItemInstance::new_resource(ItemInstanceId::new(), item, 30),
        location: ItemLocation::PortStorage(region),
    };
    let escrowed = Custody {
        instance: ItemInstance::new_resource(ItemInstanceId::new(), item, 5),
        location: ItemLocation::MarketEscrow(order_id),
    };

    let snapshot = MarketSnapshot {
        identities,
        storage: vec![marvyr_server::market::StorageEntry {
            character,
            region,
            stacks: vec![stack],
        }],
        escrow: vec![marvyr_server::market::EscrowEntry {
            order_num,
            stacks: vec![escrowed],
        }],
        board: vec![MarketOrder {
            id: order_id,
            seller: character,
            item,
            quantity: 5,
            ask_item,
            ask_quantity: 3,
            region,
            status: OrderStatus::Open,
            created_at: Utc::now(),
            expires_at: Utc::now(),
        }],
        order_nums: HashMap::from([(order_num, order_id)]),
        next_order_num: 1,
    };
    (snapshot, character)
}

fn quantity_of(snapshot: &MarketSnapshot, character: CharacterId) -> u32 {
    snapshot
        .storage
        .iter()
        .filter(|entry| entry.character == character)
        .flat_map(|entry| entry.stacks.iter())
        .map(|custody| custody.instance.quantity)
        .sum()
}

/// MF-034: o estado econômico completo sobrevive ao banco — salvo em uma
/// transação, lido de volta idêntico (storage, escrow, ofertas).
#[test]
fn market_state_roundtrips_through_postgres() {
    let _guard = test_lock();
    let Some((store, _url)) = store_or_skip() else {
        return;
    };
    let (snapshot, character) = sample_snapshot();

    store
        .save_market(&snapshot)
        .expect("save_market transacional");
    let restored = store
        .load_market()
        .expect("load_market")
        .expect("estado após save");

    assert_eq!(restored.identities.get("token-alfa"), Some(&character));
    assert_eq!(quantity_of(&restored, character), 30);
    assert_eq!(restored.board.len(), 1);
    assert_eq!(restored.board[0].ask_quantity, 3);
    assert_eq!(restored.board[0].ask_item, snapshot.board[0].ask_item);
    assert_eq!(restored.order_nums.len(), 1);
    assert_eq!(restored.escrow.len(), 1);
    assert_eq!(restored.escrow[0].stacks[0].instance.quantity, 5);
    assert_eq!(restored.next_order_num, 1);
}

/// A unidade atômica é o estado inteiro (ADR-0010 no Alpha single-writer):
/// salvar duas vezes seguidas não duplica linhas nem entra em conflito.
#[test]
fn repeated_saves_stay_consistent() {
    let _guard = test_lock();
    let Some((store, _url)) = store_or_skip() else {
        return;
    };
    let (snapshot, character) = sample_snapshot();
    store.save_market(&snapshot).expect("primeiro save");
    store.save_market(&snapshot).expect("segundo save");
    let restored = store
        .load_market()
        .expect("load_market")
        .expect("estado após saves");
    assert_eq!(quantity_of(&restored, character), 30, "carga não duplica");
    assert_eq!(restored.board.len(), 1, "order não duplica entre saves");
}

/// MF-035/034: o navio do personagem sobrevive — casco, HP, posição e a
/// carga embarcada (item_instances com location ShipCargo).
#[test]
fn ship_record_roundtrips_through_postgres() {
    let _guard = test_lock();
    let Some((store, _url)) = store_or_skip() else {
        return;
    };
    let character = CharacterId::new();
    // No fluxo real o personagem já existe no banco (market.character →
    // save_market). O teste reproduz a ordem: identidade primeiro, navio
    // depois — a FK de ship_instances é o fail-closed do banco.
    let seed = MarketSnapshot {
        identities: HashMap::from([("token-navio".to_string(), character)]),
        storage: Vec::new(),
        escrow: Vec::new(),
        board: Vec::new(),
        order_nums: HashMap::new(),
        next_order_num: 0,
    };
    store.save_market(&seed).expect("identidade no banco");

    let ship_instance = ShipInstanceId::new();
    let item = ItemDefinitionId::new();
    let sail_item = ItemDefinitionId::new();
    let record = ShipRecord {
        ship_instance,
        character,
        kind: ShipKind::Corsair,
        hp: 55,
        x: -300.5,
        y: 42.25,
        heading: 1.25,
        cargo: vec![Custody {
            instance: ItemInstance::new_resource(ItemInstanceId::new(), item, 12),
            location: ItemLocation::ShipCargo(ship_instance),
        }],
        equipped: vec![Custody {
            instance: ItemInstance::new_equipment(ItemInstanceId::new(), sail_item, 100),
            location: ItemLocation::Equipped {
                ship: ship_instance,
                slot: marvyr_domain_items::EquipmentSlot::Sail,
            },
        }],
        // MF-049: testa o roundtrip da presença. Navio do teste estava
        // fora do porto no momento da persistência — restaura igual.
        presence: marvyr_domain_ships::VesselPresence::AtSea,
        crew: 7,
    };

    store.save_ship(&record).expect("save_ship");
    let restored = store
        .load_ship(character)
        .expect("load_ship")
        .expect("navio persistido");

    assert_eq!(restored.ship_instance, ship_instance);
    assert_eq!(restored.character, character);
    assert_eq!(restored.kind, ShipKind::Corsair);
    assert_eq!(restored.hp, 55);
    assert_eq!(
        restored.presence,
        marvyr_domain_ships::VesselPresence::AtSea
    );
    assert_eq!(restored.cargo.len(), 1);
    assert_eq!(restored.cargo[0].instance.quantity, 12);
    assert_eq!(restored.equipped.len(), 1, "loadout persistiu");
    assert_eq!(
        restored.equipped[0].location,
        ItemLocation::Equipped {
            ship: ship_instance,
            slot: marvyr_domain_items::EquipmentSlot::Sail,
        }
    );
    assert_eq!(
        restored.cargo[0].location,
        ItemLocation::ShipCargo(ship_instance)
    );

    // Depositar no porto mantém o id do item: o save_market seguinte não
    // pode bater na linha de carga ainda gravada pelo último save_ship.
    let region = RegionId::new();
    let mut deposited = restored.cargo[0].clone();
    deposited.location = ItemLocation::PortStorage(region);
    let mut after_deposit = seed.clone();
    after_deposit.storage = vec![marvyr_server::market::StorageEntry {
        character,
        region,
        stacks: vec![deposited],
    }];
    store
        .save_market(&after_deposit)
        .expect("save_market depois do depósito");
    let market = store.load_market().expect("load").expect("snapshot");
    assert_eq!(quantity_of(&market, character), 12, "item mudou de lugar");
    let ship = store.load_ship(character).expect("load").expect("navio");
    assert!(ship.cargo.is_empty(), "item não fica em dois lugares");

    // Naufrágio: o casco checkpointado não volta no próximo login, e o
    // equipamento a bordo some com ele; o storage do porto fica.
    store.delete_ships_of(character).expect("delete_ships_of");
    assert!(store.load_ship(character).expect("load").is_none());
    let market = store.load_market().expect("load").expect("snapshot");
    assert_eq!(quantity_of(&market, character), 12, "storage intocado");
}

/// v22: a peça Rara guarda raridade e afixos em qualquer lugar onde more
/// (armazém → equipada → armazém), e nunca em dois lugares ao mesmo tempo.
#[test]
fn affixes_survive_storage_equip_and_unequip() {
    let _guard = test_lock();
    let Some((store, _url)) = store_or_skip() else {
        return;
    };
    let character = CharacterId::new();
    let region = RegionId::new();
    let ship_instance = ShipInstanceId::new();
    let cannon = ItemDefinitionId::new();
    let quality = marvyr_domain_items::roll_quality(marvyr_domain_items::Rarity::Rare, 77);
    let piece = ItemInstance {
        quality: quality.clone(),
        ..ItemInstance::new_equipment(ItemInstanceId::new(), cannon, 100)
    };
    let market_with = |stacks: Vec<Custody>| MarketSnapshot {
        identities: HashMap::from([("token-afixo".to_string(), character)]),
        storage: vec![marvyr_server::market::StorageEntry {
            character,
            region,
            stacks,
        }],
        escrow: Vec::new(),
        board: Vec::new(),
        order_nums: HashMap::new(),
        next_order_num: 0,
    };
    let stored = Custody {
        instance: piece.clone(),
        location: ItemLocation::PortStorage(region),
    };
    store
        .save_market(&market_with(vec![stored.clone()]))
        .expect("save_market");
    let market = store.load_market().expect("load").expect("snapshot");
    assert_eq!(
        market.storage[0].stacks[0].instance.quality, quality,
        "no armazém"
    );

    // Equipar: sai do armazém, entra no navio.
    store
        .save_market(&market_with(Vec::new()))
        .expect("save_market");
    let record = ShipRecord {
        ship_instance,
        character,
        kind: ShipKind::Corsair,
        hp: 100,
        x: 0.0,
        y: 0.0,
        heading: 0.0,
        cargo: Vec::new(),
        equipped: vec![Custody {
            instance: piece.clone(),
            location: ItemLocation::Equipped {
                ship: ship_instance,
                slot: marvyr_domain_items::EquipmentSlot::Weapon,
            },
        }],
        presence: marvyr_domain_ships::VesselPresence::AtSea,
        crew: 4,
    };
    store.save_ship(&record).expect("save_ship");
    let ship = store.load_ship(character).expect("load").expect("navio");
    assert_eq!(ship.equipped[0].instance.quality, quality, "equipada");
    let market = store.load_market().expect("load").expect("snapshot");
    assert_eq!(
        quantity_of(&market, character),
        0,
        "não fica em dois lugares"
    );

    // Desequipar: volta ao armazém com os mesmos afixos.
    store
        .save_ship(&ShipRecord {
            equipped: Vec::new(),
            ..record
        })
        .expect("save_ship");
    store
        .save_market(&market_with(vec![stored]))
        .expect("save_market");
    let ship = store.load_ship(character).expect("load").expect("navio");
    assert!(ship.equipped.is_empty());
    let market = store.load_market().expect("load").expect("snapshot");
    assert_eq!(
        market.storage[0].stacks[0].instance.quality, quality,
        "de volta"
    );
}

/// v24: gema encaixada mora dentro da peça equipada; tirada, volta à pilha
/// do armazém. Em nenhum ponto da sequência ela está nos dois lugares.
#[test]
fn gems_live_in_one_place_through_socket_and_unsocket() {
    let _guard = test_lock();
    let Some((store, _url)) = store_or_skip() else {
        return;
    };
    use marvyr_domain_items::{gem, GemKind};
    let character = CharacterId::new();
    let region = RegionId::new();
    let ship_instance = ShipInstanceId::new();
    let ruby = GemKind::Ruby.item_id();
    let gems_in = |quantity: u32| Custody {
        instance: ItemInstance::new_resource(ItemInstanceId::new(), ruby, quantity),
        location: ItemLocation::PortStorage(region),
    };
    let market_with = |stacks: Vec<Custody>| MarketSnapshot {
        identities: HashMap::from([("token-gema".to_string(), character)]),
        storage: vec![marvyr_server::market::StorageEntry {
            character,
            region,
            stacks,
        }],
        escrow: Vec::new(),
        board: Vec::new(),
        order_nums: HashMap::new(),
        next_order_num: 0,
    };
    let mut piece =
        ItemInstance::new_equipment(ItemInstanceId::new(), ItemDefinitionId::new(), 100);
    let record_with = |piece: &ItemInstance| ShipRecord {
        ship_instance,
        character,
        kind: ShipKind::Corsair,
        hp: 100,
        x: 0.0,
        y: 0.0,
        heading: 0.0,
        cargo: Vec::new(),
        equipped: vec![Custody {
            instance: piece.clone(),
            location: ItemLocation::Equipped {
                ship: ship_instance,
                slot: marvyr_domain_items::EquipmentSlot::Weapon,
            },
        }],
        presence: marvyr_domain_ships::VesselPresence::AtSea,
        crew: 4,
    };
    let stored_gems = || {
        let market = store.load_market().expect("load").expect("snapshot");
        market.storage[0]
            .stacks
            .iter()
            .filter(|c| c.instance.definition == ruby)
            .map(|c| c.instance.quantity)
            .sum::<u32>()
    };
    let socketed = || {
        let ship = store.load_ship(character).expect("load").expect("navio");
        ship.equipped[0].instance.gems().to_vec()
    };
    store
        .save_market(&market_with(vec![gems_in(2)]))
        .expect("save_market");
    store.save_ship(&record_with(&piece)).expect("save_ship");

    // Encaixar: armazém grava sem a gema, depois o navio com ela.
    store
        .save_market(&market_with(vec![gems_in(1)]))
        .expect("save_market");
    gem::socket(&mut piece.quality, GemKind::Ruby).unwrap();
    store.save_ship(&record_with(&piece)).expect("save_ship");
    assert_eq!(stored_gems(), 1);
    assert_eq!(socketed(), vec![GemKind::Ruby], "encaixada na peça");

    // Tirar: navio grava sem a gema, depois o armazém com ela.
    gem::unsocket(&mut piece.quality, 0).unwrap();
    store.save_ship(&record_with(&piece)).expect("save_ship");
    assert!(socketed().is_empty());
    store
        .save_market(&market_with(vec![gems_in(2)]))
        .expect("save_market");
    assert_eq!(stored_gems(), 2, "de volta à pilha, sem cópia");
}

/// MF-041: um Expired persistido no banco volta com o status preservado e
/// sem escrow, porque o servidor já devolveu o item ao storage do seller.
#[test]
fn expired_order_roundtrips_with_escrow_returned() {
    let _guard = test_lock();
    let Some((store, _url)) = store_or_skip() else {
        return;
    };
    let region = RegionId::new();
    let item = ItemDefinitionId::new();
    let mut catalog = ItemCatalog::default();
    catalog
        .register(ItemDefinition {
            id: item,
            kind: ItemKind::Resource,
            equipment: None,
            max_stack: 100,
            base_weight: 1,
            tags: Default::default(),
            display_name: String::from("Madeira"),
        })
        .expect("catálogo de teste não registra duplicatas");

    let mut market = ServerMarket::with_store(Some(store.clone()));
    let character = market.character("token-expired");
    let mut hold = CargoHold::new(ShipInstanceId::new(), 1_000);
    hold.insert(
        &catalog,
        ItemInstance::new_resource(ItemInstanceId::new(), item, 10),
    )
    .expect("teste cabe no porão");
    market
        .deposit_all(character, region, &mut hold, &catalog)
        .expect("teste deposita no storage");
    market.order_duration_secs = 60;
    market
        .create_order(character, region, item, 10, ItemDefinitionId::new(), 8)
        .expect("storage tem estoque");
    let order = market.snapshot().board[0].clone();
    market.expire_orders(order.expires_at + chrono::Duration::seconds(1));

    let restored = store
        .load_market()
        .expect("load_market")
        .expect("estado após expire");
    let restored_order = restored.board[0].clone();
    assert_eq!(restored_order.status, OrderStatus::Expired);
    assert!(restored.escrow.is_empty(), "escrow devolvido ao storage");
    let storage_quantity = restored
        .storage
        .iter()
        .filter(|entry| entry.character == character && entry.region == region)
        .flat_map(|entry| entry.stacks.iter())
        .filter(|custody| custody.instance.definition == item)
        .map(|custody| custody.instance.quantity)
        .sum::<u32>();
    assert_eq!(storage_quantity, 10);
}

/// Escambo: a oferta sai do armazém (escrow) e, aceita, os dois lados
/// trocam de dono no MESMO save — depois do reload cada unidade mora em um
/// lugar só (nada de escrow órfão, nada de id repetido da pilha dividida).
#[test]
fn barter_trade_persists_both_sides() {
    let _guard = test_lock();
    let Some((store, _url)) = store_or_skip() else {
        return;
    };
    let region = RegionId::new();
    let (wood, ore) = (ItemDefinitionId::new(), ItemDefinitionId::new());
    let mut catalog = ItemCatalog::default();
    for (id, name) in [(wood, "Madeira"), (ore, "Minério")] {
        catalog
            .register(ItemDefinition {
                id,
                kind: ItemKind::Resource,
                equipment: None,
                max_stack: 100,
                base_weight: 1,
                tags: Default::default(),
                display_name: String::from(name),
            })
            .expect("catálogo de teste");
    }
    let mut market = ServerMarket::with_store(Some(store.clone()));
    let seller = market.character("token-seller");
    let buyer = market.character("token-buyer");
    for (who, item) in [(seller, wood), (buyer, ore)] {
        let mut hold = CargoHold::new(ShipInstanceId::new(), 1_000);
        hold.insert(
            &catalog,
            ItemInstance::new_resource(ItemInstanceId::new(), item, 20),
        )
        .expect("cabe");
        market
            .deposit_all(who, region, &mut hold, &catalog)
            .expect("deposita");
    }
    // Pilhas divididas dos dois lados: 20 → 12 + 8 e 20 → 5 + 15.
    let order_num = market
        .create_order(seller, region, wood, 12, ore, 5)
        .expect("oferta");
    let listed = store.load_market().expect("load").expect("snapshot");
    assert_eq!(listed.escrow.len(), 1, "escrow persistido na criação");
    market.buy(buyer, region, order_num).expect("troca");

    let restored = store.load_market().expect("load").expect("snapshot");
    let held = |who: CharacterId, item: ItemDefinitionId| {
        restored
            .storage
            .iter()
            .filter(|entry| entry.character == who && entry.region == region)
            .flat_map(|entry| entry.stacks.iter())
            .filter(|custody| custody.instance.definition == item)
            .map(|custody| custody.instance.quantity)
            .sum::<u32>()
    };
    assert_eq!((held(seller, wood), held(seller, ore)), (8, 5));
    assert_eq!((held(buyer, wood), held(buyer, ore)), (12, 15));
    assert!(restored.board.is_empty(), "oferta aceita sai do quadro");
    assert!(restored.escrow.is_empty(), "sem escrow órfão");
}

/// Oferta da era do ouro (sem `ask_*`): o load cancela e devolve o escrow
/// ao armazém do vendedor, no porto da oferta.
#[test]
fn legacy_gold_order_is_cancelled_on_load() {
    let _guard = test_lock();
    let Some((store, url)) = store_or_skip() else {
        return;
    };
    let (mut snapshot, character) = sample_snapshot();
    let region = snapshot.board[0].region;
    let order_id = snapshot.board[0].id;
    store.save_market(&snapshot).expect("save");
    let runtime = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .expect("runtime de teste");
    runtime.block_on(async {
        let pool = sqlx::PgPool::connect(&url).await.expect("pool");
        sqlx::query(
            "UPDATE market_orders SET unit_price = 8, ask_item_definition_id = NULL, \
             ask_quantity = NULL WHERE id = $1",
        )
        .bind(order_id.0)
        .execute(&pool)
        .await
        .expect("vira oferta antiga");
        pool.close().await;
    });

    let restored = store.load_market().expect("load").expect("snapshot");
    assert!(restored.board.is_empty(), "oferta antiga cancelada");
    assert!(restored.escrow.is_empty(), "escrow devolvido");
    // 30 no armazém + 5 que estavam em escrow, no porto da oferta.
    snapshot.storage.clear();
    assert_eq!(quantity_of(&restored, character), 35);
    assert!(restored
        .storage
        .iter()
        .flat_map(|entry| entry.stacks.iter())
        .all(|custody| custody.location == ItemLocation::PortStorage(region)));
}

/// MF-027 cont.: o snapshot de wrecks sobrevive ao banco. Dois wrecks
/// ativos (com e sem exclusive_looter) roundtrippam com todos os campos,
/// e `delete_wreck` remove pontualmente.
#[test]
fn wreck_snapshot_roundtrips_through_postgres() {
    let _guard = test_lock();
    let Some((store, url)) = store_or_skip() else {
        return;
    };
    reset_database(&url);

    let killer = CharacterId::new();
    let records = vec![
        marvyr_server::persist::WreckRecord {
            wreck_num: 0,
            wreck_id: marvyr_shared::ids::WreckId::new(),
            x: 120.0,
            y: -45.0,
            exclusive_looter: Some(killer),
            spawned_at_secs: 12.5,
        },
        marvyr_server::persist::WreckRecord {
            wreck_num: 1,
            wreck_id: marvyr_shared::ids::WreckId::new(),
            x: -300.0,
            y: 80.0,
            exclusive_looter: None,
            spawned_at_secs: 90.25,
        },
    ];

    store
        .save_wreck_snapshot(&records)
        .expect("save_wreck_snapshot grava dois wrecks");

    let restored = store
        .load_wreck_snapshot()
        .expect("load_wreck_snapshot lê o snapshot");
    assert_eq!(restored.len(), 2);
    assert_eq!(restored[0].wreck_num, 0);
    assert_eq!(restored[0].x, 120.0);
    assert_eq!(restored[0].y, -45.0);
    assert_eq!(restored[0].exclusive_looter, Some(killer));
    assert!((restored[0].spawned_at_secs - 12.5).abs() < 1e-6);
    assert_eq!(restored[1].wreck_num, 1);
    assert_eq!(restored[1].x, -300.0);
    assert_eq!(restored[1].exclusive_looter, None);

    store.delete_wreck(0).expect("delete_wreck");
    let remaining = store.load_wreck_snapshot().expect("load após delete");
    assert_eq!(remaining.len(), 1);
    assert_eq!(remaining[0].wreck_num, 1);
}

#[test]
fn cosmetics_roundtrip_through_postgres() {
    let _guard = test_lock();
    let Some((store, url)) = store_or_skip() else {
        return;
    };
    reset_database(&url);

    // O personagem nasce no save do mercado; a concessão vem da ferramenta
    // de admin (aqui, o mesmo INSERT que ela faz).
    let (snapshot, character) = sample_snapshot();
    store
        .save_market(&snapshot)
        .expect("save_market cria o personagem");
    assert_eq!(
        store.load_cosmetics(character).expect("load sem concessão"),
        marvyr_server::persist::CosmeticsRecord::default()
    );
    let runtime = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .expect("runtime de teste");
    runtime.block_on(async {
        let pool = sqlx::PgPool::connect(&url).await.expect("pool");
        sqlx::query(
            "INSERT INTO character_cosmetics (character_id, cosmetic_id, granted_by) \
             VALUES ($1, 'sail-gold', 'teste')",
        )
        .bind(character.0)
        .execute(&pool)
        .await
        .expect("concessão");
        pool.close().await;
    });

    store
        .save_cosmetic_choice(character, Some("sail-gold"), None)
        .expect("save_cosmetic_choice");
    let record = store.load_cosmetics(character).expect("load_cosmetics");
    assert_eq!(record.owned, vec![String::from("sail-gold")]);
    assert_eq!(record.sail.as_deref(), Some("sail-gold"));
    assert_eq!(record.flag, None);
}

#[test]
fn renown_roundtrips_through_postgres() {
    let _guard = test_lock();
    let Some((store, url)) = store_or_skip() else {
        return;
    };
    reset_database(&url);

    let (snapshot, character) = sample_snapshot();
    store
        .save_market(&snapshot)
        .expect("save_market cria o personagem");
    assert_eq!(store.load_renown(character).expect("load inicial"), 0);
    store.save_renown(&[(character, 420)]).expect("save_renown");
    store
        .save_renown(&[(character, 555)])
        .expect("save_renown de novo");
    assert_eq!(store.load_renown(character).expect("load_renown"), 555);
    // Sessão que não leu o banco começa do zero: nunca apaga o gravado.
    store.save_renown(&[(character, 100)]).expect("save menor");
    assert_eq!(store.load_renown(character).expect("load_renown"), 555);
}

#[test]
fn talents_roundtrip_through_postgres() {
    let _guard = test_lock();
    let Some((store, url)) = store_or_skip() else {
        return;
    };
    reset_database(&url);

    let (snapshot, character) = sample_snapshot();
    store
        .save_market(&snapshot)
        .expect("save_market cria o personagem");
    assert!(store
        .load_talents(character)
        .expect("load inicial")
        .is_empty());
    let learned = vec![String::from("nav.leme"), String::from("nav.pano")];
    store
        .save_talents(character, &learned)
        .expect("save_talents");
    assert_eq!(store.load_talents(character).expect("load"), learned);
    store.save_talents(character, &[]).expect("respec");
    assert!(store.load_talents(character).expect("load").is_empty());
    // Personagem que não existe: erro, não silêncio.
    assert!(store
        .save_talents(marvyr_shared::ids::CharacterId::new(), &learned)
        .is_err());
}

#[test]
fn captain_progress_roundtrips_through_postgres() {
    let _guard = test_lock();
    let Some((store, url)) = store_or_skip() else {
        return;
    };
    reset_database(&url);

    let (snapshot, character) = sample_snapshot();
    store
        .save_market(&snapshot)
        .expect("save_market cria o personagem");
    let empty = store.load_progress(character).expect("load inicial");
    assert_eq!(empty, Default::default());
    let progress = marvyr_domain_economy::logbook::CaptainProgress {
        day: 20_000,
        daily: [3, 0, 1],
        week: 2857,
        weekly: 4,
        unpaid: vec![(String::from("Minério"), 30)],
        abyss_best: 4,
        found: ["Kraken", "Cerração"].map(String::from).into(),
        mastery: [(String::from("Corsário"), 1_200)].into(),
        season: 7,
        season_points: 900,
        crowns: 1,
        influence: [(String::from("Porto do Coral Negro"), 250)].into(),
        influence_week: 2_961,
        tribute_day: 20_720,
    };
    store.save_progress(character, &progress).expect("save");
    assert_eq!(store.load_progress(character).expect("load"), progress);
    // v43: o placar da temporada lê direto do JSON.
    assert_eq!(
        store.load_season_top(7, 10).expect("placar"),
        vec![(character, 900)]
    );
    assert!(store.load_season_top(8, 10).expect("placar").is_empty());
    // v45: Senhor do Porto também lê do JSON.
    assert_eq!(
        store
            .load_port_lord(2_961, "Porto do Coral Negro")
            .expect("senhor"),
        Some((character, 250))
    );
    assert_eq!(
        store
            .load_port_lord(2_962, "Porto do Coral Negro")
            .expect("senhor"),
        None
    );
    assert!(store
        .save_progress(marvyr_shared::ids::CharacterId::new(), &progress)
        .is_err());
}

#[test]
fn web_cert_is_single_row_with_latest_digest() {
    let _guard = test_lock();
    let Some((store, url)) = store_or_skip() else {
        return;
    };
    // Cada boot publica o hash novo: a linha é única e fica com o último.
    store
        .publish_web_cert(&"aa".repeat(32))
        .expect("primeiro boot");
    store.publish_web_cert(&"bb".repeat(32)).expect("reboot");
    let runtime = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .expect("runtime de teste");
    let rows: Vec<String> = runtime.block_on(async {
        let pool = sqlx::PgPool::connect(&url).await.expect("pool");
        let rows = sqlx::query_scalar("SELECT digest FROM web_cert")
            .fetch_all(&pool)
            .await
            .expect("leitura");
        pool.close().await;
        rows
    });
    assert_eq!(rows, vec!["bb".repeat(32)]);
}
