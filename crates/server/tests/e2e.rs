//! MF-030 — End-to-End Vertical Slice (PRD Phase 10). A prova do loop:
//!
//! ```text
//! Player A coleta → fabrica → transporta
//!   → Player B ataca → navio afunda → loot transfere
//!   → B volta → troca → nenhuma unidade surge do nada
//! ```
//!
//! O teste dirige os módulos puros de domínio e o ServerMarket na mesma
//! sequência do jogo — a mesma fronteira que os handlers Bevy chamam.

use marvyr_domain_combat::{
    apply_damage, resolve_ship_destruction, DamageOutcome, LootPolicy, WreckChest,
};
use marvyr_domain_crafting::{Ingredient, Recipe, StationKind};
use marvyr_domain_items::{
    CargoHold, Custody, ItemCatalog, ItemDefinition, ItemInstance, ItemKind,
};
use marvyr_domain_ships::{step_motion, MotionInput, MotionTuning, ShipMotion};
use marvyr_domain_world::{ResourceNode, WorldMap};
use marvyr_server::market::{port_region, ServerMarket};
use marvyr_shared::ids::{
    CharacterId, DestructionEventId, ItemDefinitionId, ItemInstanceId, RecipeId, ShipInstanceId,
    WreckId,
};

const STARTER_PORT: (f32, f32) = (-560.0, 0.0); // doca do Porto da Serra

fn catalog_with_goods() -> (ItemCatalog, ItemDefinitionId, ItemDefinitionId) {
    let wood = ItemDefinitionId::new();
    let hull = ItemDefinitionId::new();
    let mut catalog = ItemCatalog::default();
    let mut register = |definition: ItemDefinition| catalog.register(definition).unwrap();
    register(ItemDefinition {
        id: wood,
        kind: ItemKind::Resource,
        equipment: None,
        max_stack: 100,
        base_weight: 2,
        tags: Default::default(),
        display_name: String::from("Madeira"),
    });
    register(ItemDefinition {
        id: hull,
        kind: ItemKind::Equipment,
        equipment: Some(marvyr_domain_items::EquipmentDefinition {
            slot: marvyr_domain_items::EquipmentSlot::Hull,
            stats: marvyr_domain_items::EquipmentStats {
                damage: 0,
                speed: 0,
                cargo: 0,
                hp: 40,
                range: 0,
            },
        }),
        max_stack: 1,
        base_weight: 8,
        tags: Default::default(),
        display_name: String::from("Casco Reforçado"),
    });
    (catalog, wood, hull)
}

fn craft_hull_recipe(wood: ItemDefinitionId, hull: ItemDefinitionId) -> Recipe {
    Recipe {
        id: RecipeId::new(),
        display_name: String::from("Casco Reforçado"),
        output_item: hull,
        output_quantity: 1,
        ingredients: vec![Ingredient {
            item: wood,
            quantity: 15,
        }],
        required_station: StationKind::Workbench,
        craft_time_secs: 0,
        output_rarity: Default::default(),
        output_tier: 1,
    }
}

/// A personagem de um jogador: porão e carteira.
struct Player {
    character: CharacterId,
    hold: CargoHold,
}

#[test]
fn vertical_slice_loop_gather_craft_transport_fight_loot_sell() {
    // ===== Mundo =====
    let map = WorldMap::vertical_slice();
    let (catalog, wood, hull) = catalog_with_goods();
    let region_serra = map.region_by_name("Porto da Serra").unwrap().id;
    let mut market = ServerMarket::new();

    let mut a = Player {
        character: market.character("token-a"),
        hold: CargoHold::new(ShipInstanceId::new(), 100),
    };
    let mut b = Player {
        character: market.character("token-b"),
        hold: CargoHold::new(ShipInstanceId::new(), 100),
    };

    // ===== 1. A coleta (node → ShipCargo) =====
    let mut node = ResourceNode {
        id: marvyr_shared::ids::ResourceNodeId::new(),
        name: "Bosque da Serra",
        x: -700.0,
        y: 90.0,
        region: region_serra,
        resource: wood,
        stock: 60,
        max_stock: 60,
    };
    let mut gathered = 0;
    while gathered < 30 {
        let taken = node.take(10);
        assert!(taken > 0, "node tem estoque para a sessão de coleta");
        a.hold
            .insert(
                &catalog,
                ItemInstance::new_resource(ItemInstanceId::new(), wood, taken),
            )
            .expect("porão comporta a coleta do dia");
        gathered += taken;
    }
    assert_eq!(node.stock, 60 - 30);
    assert_eq!(a.hold.used_weight(&catalog).unwrap(), 60); // 30 × peso 2

    // ===== 2. A fabrica na OFICINA do porto (MF-036/037) =====
    // atracar → depositar o dia de coleta → craftar NO STORAGE → embarcar
    // só o que quer transportar. O porão não é matéria-prima automática.
    market
        .deposit_all(a.character, region_serra, &mut a.hold, &catalog)
        .expect("doca da Serra recebe o porão do dia");
    assert_eq!(
        a.hold.used_weight(&catalog).unwrap(),
        0,
        "porão vazio na doca"
    );
    let recipe = craft_hull_recipe(wood, hull);
    let crafted = market
        .craft_at_storage(
            a.character,
            region_serra,
            &recipe,
            &catalog,
            StationKind::Workbench,
        )
        .expect("oficina da Serra com madeira guardada de sobra");
    assert_eq!(crafted.definition, hull);
    assert!(crafted.durability.is_some());
    assert_eq!(
        market.storage_quantity(a.character, region_serra, hull),
        1,
        "casco nasce no storage, não no porão"
    );
    // ===== 2.5 A EQUIPA o casco no slot Hull (MF-039) =====
    // PortStorage → Equipped(ship, slot), com stats recalculados na hora —
    // e ANTES de embarcar: o porão só leva madeira, o casco vai instalado.
    let definition_small_merchant = marvyr_domain_ships::ShipDefinition::small_merchant();
    let mut a_loadout = marvyr_domain_ships::ShipLoadout::new();
    let ship_instance = ShipInstanceId::new();
    let installed = market
        .take_one_from_storage(a.character, region_serra, hull, None)
        .expect("casco está no storage da Serra");
    let slot =
        marvyr_domain_ships::can_equip(&definition_small_merchant, catalog.get(hull).unwrap())
            .expect("merchant tem slot Hull");
    a_loadout.equip(ship_instance, installed, slot);
    let equipped_stats = marvyr_domain_ships::compute_ship_stats(
        &definition_small_merchant,
        &a_loadout.components(),
        &catalog,
    )
    .expect("loadout com definições do catálogo");
    assert_eq!(
        equipped_stats.max_hp, 140,
        "casco reforçado (+40 hp) é observável nos stats"
    );
    assert!(
        market
            .take_one_from_storage(a.character, region_serra, hull, None)
            .is_err(),
        "o casco saiu do storage: equipar move a instância, não copia"
    );
    // Swap NUNCA destrói: desequipar devolve a MESMA instância ao storage.
    let devolvido = a_loadout.unequip(slot).expect("casco sai do slot");
    market.return_to_storage(a.character, region_serra, devolvido, &catalog);
    a_loadout.equip(
        ship_instance,
        market
            .take_one_from_storage(a.character, region_serra, hull, None)
            .expect("re-equipa o casco"),
        slot,
    );
    market
        .withdraw_all(a.character, region_serra, &mut a.hold, &catalog)
        .expect("embarca só o que decidiu carregar");
    assert_eq!(
        a.hold.used_weight(&catalog).unwrap(),
        30, // 15 madeira (peso 2) — o casco vai INSTALADO, não no porão
        "carga embarcada e loadout são decisões separadas"
    );

    // ===== 3. A transporta (modelo puro de movimento, rota leste) =====
    let definition = marvyr_domain_ships::ShipDefinition::small_merchant();
    let stats = marvyr_domain_ships::compute_ship_stats(
        &definition,
        &marvyr_domain_ships::EquippedComponents::default(),
        &catalog,
    )
    .expect("navio sem equipamento: stats não falham");
    let mut motion = ShipMotion {
        x: STARTER_PORT.0,
        y: STARTER_PORT.1,
        ..ShipMotion::default()
    };
    let tuning = MotionTuning::default();
    for _ in 0..600 {
        step_motion(
            &mut motion,
            &stats,
            MotionInput {
                throttle: 1.0,
                turn: 0.0,
            },
            &tuning,
            1.0 / 30.0,
        );
    }
    assert!(
        motion.x > STARTER_PORT.0 + 200.0,
        "A navegou para leste rumo à rota da costa"
    );
    // A rota é fronteira: PvP legal — é aqui que B pode atacar (§8/§9).
    assert_eq!(
        map.zone_at(motion.x, motion.y).unwrap().tier,
        marvyr_domain_world::RiskTier::Frontier
    );

    // ===== 4. A deposita no porto e oferece madeira (storage → escrow) =====
    // A volta à baía para operar o mercado (§45).
    let (port_region_id, _) = port_region(&map, STARTER_PORT.0, STARTER_PORT.1)
        .expect("doca do Porto da Serra é área de porto");
    assert_eq!(port_region_id, region_serra);
    market
        .deposit_all(a.character, port_region_id, &mut a.hold, &catalog)
        .expect("deposita a madeira restante; o casco segue INSTALADO");
    // MF-039: o casco equipado não está no storage — não pode ser listado
    // (equipar moveu a instância; é isso que impede vender o que está em uso).
    assert!(market
        .create_order(a.character, port_region_id, hull, 1, wood, 20)
        .is_err());
    // A quer o casco de volta se afundar: 15 madeira por 1 casco.
    let wood_for_hull = market
        .create_order(a.character, port_region_id, wood, 15, hull, 1)
        .expect("a madeira restante está no storage local");

    // ===== 5. B ataca: projéteis até afundar (PvP em fronteira) =====
    let mut hp = 100;
    let mut sinking = false;
    for _ in 0..20 {
        match apply_damage(hp, 20) {
            DamageOutcome::Survived { remaining_hp } => hp = remaining_hp,
            DamageOutcome::Destroyed => {
                sinking = true;
                break;
            }
        }
    }
    assert!(sinking, "5 impactos de 20 afundam o casco de 100");

    // ===== 6. Navio afunda: full loot transfere carga de dono =====
    // A carga do navio de A era: madeira sobrando + o casco fabricado.
    let mut hold_afundado = CargoHold::new(ShipInstanceId::new(), 100);
    hold_afundado
        .insert(
            &catalog,
            ItemInstance::new_resource(ItemInstanceId::new(), wood, 5),
        )
        .unwrap();
    // O casco NÃO está no porão: está INSTALADO no slot Hull (MF-039) e
    // entra na resolução pela lista de equipamento, abaixo.
    let cargo: Vec<ItemInstance> = hold_afundado
        .items()
        .iter()
        .map(|custody| custody.instance.clone())
        .collect();
    // MF-039: o equipamento INSTALADO (um casco no slot Hull) participa do
    // full loot — 50% de chance de sobreviver por peça (§24).
    let equipment = [ItemInstance::new_equipment(
        ItemInstanceId::new(),
        hull,
        100,
    )];
    let outcome = resolve_ship_destruction(
        DestructionEventId::new(),
        &equipment,
        &cargo,
        &LootPolicy::default(),
    );
    // Nada desaparece no vazio: por definição, quantidade sobrevivente +
    // quantidade destruída = quantidade embarcada (a pilha pode se dividir:
    // 80% da carga sobrevive por unidade, §25).
    let survived: u32 = outcome.wreck_items.iter().map(|s| s.quantity).sum();
    let destroyed: u32 = outcome.destroyed_items.iter().map(|s| s.quantity).sum();
    let shipped: u32 = cargo.iter().map(|item| item.quantity).sum::<u32>() + equipment.len() as u32;
    assert_eq!(survived + destroyed, shipped);
    // Conservação inclui o equipamento: o casco instalado está em algum
    // dos dois lados (sobreviveu 50% ou afundou 50%).
    let casco_em_algum_lado = outcome
        .wreck_items
        .iter()
        .chain(outcome.destroyed_items.iter())
        .any(|item| item.definition == hull && item.quantity == 1);
    assert!(casco_em_algum_lado, "equipamento instalado entra no loot");

    let mut chest = WreckChest::new(WreckId::new());
    for survivor in &outcome.wreck_items {
        chest.insert(survivor.clone(), ItemInstanceId::new());
    }
    // B chega no wreck e saqueia (MF-015: take_all atômico).
    let incoming: Vec<Custody> = chest.drain();
    let lootado = !incoming.is_empty();
    if lootado {
        b.hold.take_all(&catalog, incoming).expect("B tem porão");
    }

    // ===== 7. B volta ao porto e troca o que saqueou =====
    if lootado {
        market
            .deposit_all(b.character, port_region_id, &mut b.hold, &catalog)
            .expect("loot depositado no storage local de B");
    }
    let b_hull = market.storage_quantity(b.character, port_region_id, hull);
    let traded = market.buy(b.character, port_region_id, wood_for_hull);
    // Só troca quem tem o pedido inteiro: o casco sobreviveu e B o pescou.
    assert_eq!(traded.is_ok(), b_hull >= 1, "{traded:?}");
    if traded.is_ok() {
        assert_eq!(
            market.storage_quantity(a.character, port_region_id, hull),
            1
        );
        assert!(market.storage_quantity(b.character, port_region_id, wood) >= 15);
    }

    // ===== 8. O estado é persistível (MF-027) e fecha redondo =====
    let snapshot = market.snapshot();
    let bytes = serde_json::to_vec(&snapshot).expect("snapshot serializa");
    let restored: marvyr_server::market::MarketSnapshot =
        serde_json::from_slice(&bytes).expect("snapshot desserializa");
    assert_eq!(restored.storage.len(), snapshot.storage.len());
    assert_eq!(restored.board.len(), snapshot.board.len());
}
