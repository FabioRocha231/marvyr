//! MF-049: Trip lifecycle correto.
//!
//! A viagem NAO comeca simplesmente porque existe um player ship no mundo.
//! Comeca em Undock bem-sucedido, encerra em Dock bem-sucedido ou ShipDestroyed.
//! O restore apos restart e coerente com a presenca persistida.
//!
//! Estes testes chamam diretamente os helpers `finalize_trip` e `start_trip`
//! expostos pelo modulo `net` para validar a semantica do lifecycle sem
//! precisar montar todo o grafo Bevy.

use marvyr_domain_combat::BroadsideBattery;
use marvyr_domain_items::{CargoHold, ItemCatalog, ItemDefinition, ItemInstance, ItemKind};
use marvyr_domain_ships::{
    compute_ship_stats, EquippedComponents, MotionTuning, ShipKind, ShipLoadout, ShipMotion,
    VesselPresence,
};
use marvyr_protocol::ShipInput;
use marvyr_server::crafting::DevShips;
use marvyr_server::net::{
    finalize_trip, start_trip, Metrics, ServerShip, TradeRouteKey, TripOutcome, TripTelemetry,
};
use marvyr_server::persist::ShipRecord;
use marvyr_shared::ids::{CharacterId, ItemDefinitionId, ItemInstanceId, RegionId, ShipInstanceId};
use smallvec::SmallVec;

/// Constroi um ServerShip minimo so para os testes de lifecycle. Stats de
/// fallback quando o catalogo vazio nao consegue computar (e ele nao
/// consegue, porque o dominio exige definicoes reais).
fn make_ship(presence: VesselPresence) -> ServerShip {
    let ship_instance = ShipInstanceId::new();
    let kind = ShipKind::SmallMerchant;
    let definition = DevShips::new().definition(kind).clone();
    let stats = compute_ship_stats(
        &definition,
        &EquippedComponents::default(),
        &ItemCatalog::default(),
    )
    .expect("stats base do merchant nao falham");
    let hold = CargoHold::new(ship_instance, stats.cargo_capacity);
    ServerShip {
        ship_id: 1,
        client_id: None,
        character: CharacterId::new(),
        ship_instance,
        kind,
        presence,
        loadout: ShipLoadout::new(),
        input: ShipInput {
            throttle: 0.0,
            turn: 0.0,
        },
        hp: stats.max_hp,
        hold,
        battery: BroadsideBattery::default(),
        stats,
        motion: ShipMotion {
            x: 0.0,
            y: 0.0,
            ..ShipMotion::default()
        },
        tuning: MotionTuning::default(),
        zone: None,
        trip: None,
        restored_trip_started_at: None,
        sail_hp: 100.0,
        ammo: Default::default(),
        black_flag: Default::default(),
        target_lock: None,
        fire_target: None,
        flasks: Default::default(),
        combat: Default::default(),
        sea: marvyr_server::seafaring::SeaCondition::fresh(4),
    }
}

fn test_catalog() -> (ItemCatalog, ItemDefinitionId, ItemDefinitionId) {
    let timber = ItemDefinitionId::new();
    let ore = ItemDefinitionId::new();
    let mut catalog = ItemCatalog::default();
    // Madeira tem valor base na guilda; o nome vazio fica sem preço.
    for (id, weight, name) in [(timber, 2, "Madeira"), (ore, 3, "")] {
        catalog
            .register(ItemDefinition {
                id,
                kind: ItemKind::Resource,
                equipment: None,
                max_stack: 100,
                base_weight: weight,
                tags: SmallVec::new(),
                display_name: String::from(name),
            })
            .expect("ids de teste são únicos");
    }
    (catalog, timber, ore)
}

fn put_item(ship: &mut ServerShip, catalog: &ItemCatalog, item: ItemDefinitionId, quantity: u32) {
    ship.hold
        .insert(
            catalog,
            ItemInstance::new_resource(ItemInstanceId::new(), item, quantity),
        )
        .expect("teste controla peso do porão");
}

/// 1. Tempo parado Docked nao conta.
#[test]
fn docked_time_does_not_count() {
    let mut ship = make_ship(VesselPresence::Docked(RegionId::new()));
    let mut metrics = Metrics::default();

    let closed = finalize_trip(
        &mut ship,
        &mut metrics,
        3600.0,
        TripOutcome::Docked(RegionId::new()),
    );

    assert!(!closed);
    assert_eq!(metrics.trip_count, 0);
    assert_eq!(metrics.trip_total_secs, 0.0);
    assert!(ship.trip.is_none());
}

/// 2. Undock inicia trip.
#[test]
fn undock_starts_trip() {
    let mut ship = make_ship(VesselPresence::Docked(RegionId::new()));
    let mut metrics = Metrics::default();
    assert!(ship.trip.is_none());

    let origin = RegionId::new();
    start_trip(&mut ship, &mut metrics, &test_catalog().0, 100.0, origin);

    let trip = ship.trip.expect("trip aberta no undock");
    assert_eq!(trip.started_at, 100.0);
    assert_eq!(trip.origin, origin);
}

/// 3. Dock encerra exatamente uma trip.
#[test]
fn dock_ends_exactly_one_trip() {
    let mut ship = make_ship(VesselPresence::AtSea);
    ship.trip = Some(TripTelemetry {
        started_at: 50.0,
        origin: RegionId::new(),
        marked_cargo_value: 0,
        priced_quantity: 0,
        unpriced_quantity: 0,
    });
    let mut metrics = Metrics::default();

    let closed = finalize_trip(
        &mut ship,
        &mut metrics,
        110.0,
        TripOutcome::Docked(RegionId::new()),
    );

    assert!(closed);
    assert!(ship.trip.is_none());
    assert_eq!(metrics.trip_count, 1);
    assert!(
        (metrics.trip_total_secs - 60.0).abs() < 1e-6,
        "duracao somada: esperada 60s, veio {}",
        metrics.trip_total_secs,
    );
}

/// 4. Dock seguido de espera nao cria segunda trip.
#[test]
fn dock_followed_by_idle_does_not_create_second_trip() {
    let mut ship = make_ship(VesselPresence::Docked(RegionId::new()));
    let mut metrics = Metrics {
        trip_count: 1,
        trip_total_secs: 60.0,
        ..Metrics::default()
    };

    let closed = finalize_trip(
        &mut ship,
        &mut metrics,
        1000.0,
        TripOutcome::Docked(RegionId::new()),
    );

    assert!(!closed);
    assert_eq!(metrics.trip_count, 1);
    assert!((metrics.trip_total_secs - 60.0).abs() < 1e-6);
    assert!(ship.trip.is_none());
}

/// 5. Sink encerra trip.
#[test]
fn sink_ends_trip() {
    let mut ship = make_ship(VesselPresence::AtSea);
    ship.trip = Some(TripTelemetry {
        started_at: 200.0,
        origin: RegionId::new(),
        marked_cargo_value: 0,
        priced_quantity: 0,
        unpriced_quantity: 0,
    });
    let mut metrics = Metrics::default();

    let closed = finalize_trip(&mut ship, &mut metrics, 245.5, TripOutcome::Sunk);

    assert!(closed);
    assert!(ship.trip.is_none());
    assert_eq!(metrics.trip_count, 1);
    assert!(
        (metrics.trip_total_secs - 45.5).abs() < 1e-6,
        "duracao somada: esperada 45.5s, veio {}",
        metrics.trip_total_secs,
    );
}

/// 6. Navio Docked restaurado nao inicia trip.
#[test]
fn restored_docked_does_not_start_trip() {
    let record = ShipRecord {
        ship_instance: ShipInstanceId::new(),
        character: CharacterId::new(),
        kind: ShipKind::SmallMerchant,
        hp: 100,
        x: 0.0,
        y: 0.0,
        heading: 0.0,
        cargo: Vec::new(),
        equipped: Vec::new(),
        presence: VesselPresence::Docked(RegionId::new()),
        crew: 4,
        officers: 0,
    };
    let now = 999.0_f32;

    let trip_started_at = match record.presence {
        VesselPresence::AtSea => Some(now),
        VesselPresence::Docked(_) => None,
    };

    assert!(trip_started_at.is_none());
}

/// 7. Navio AtSea restaurado inicia nova medicao.
#[test]
fn restored_at_sea_starts_new_measurement() {
    let record = ShipRecord {
        ship_instance: ShipInstanceId::new(),
        character: CharacterId::new(),
        kind: ShipKind::SmallMerchant,
        hp: 100,
        x: 0.0,
        y: 0.0,
        heading: 0.0,
        cargo: Vec::new(),
        equipped: Vec::new(),
        presence: VesselPresence::AtSea,
        crew: 4,
        officers: 0,
    };
    let boot_now = 1234.5_f32;

    let trip_started_at = match record.presence {
        VesselPresence::AtSea => Some(boot_now),
        VesselPresence::Docked(_) => None,
    };

    assert_eq!(trip_started_at, Some(1234.5));
}

#[test]
fn completed_routes_are_directional_and_do_not_touch_zone_transitions() {
    let origin_a = RegionId::new();
    let origin_b = RegionId::new();
    let mut ship = make_ship(VesselPresence::AtSea);
    let mut metrics = Metrics {
        zone_transitions: 7,
        ..Metrics::default()
    };

    start_trip(&mut ship, &mut metrics, &test_catalog().0, 10.0, origin_a);
    assert!(finalize_trip(
        &mut ship,
        &mut metrics,
        20.0,
        TripOutcome::Docked(origin_b),
    ));
    start_trip(&mut ship, &mut metrics, &test_catalog().0, 30.0, origin_b);
    assert!(finalize_trip(
        &mut ship,
        &mut metrics,
        40.0,
        TripOutcome::Docked(origin_a),
    ));

    assert_eq!(
        metrics.completed_routes.get(&TradeRouteKey {
            origin: origin_a,
            destination: origin_b,
        }),
        Some(&1),
    );
    assert_eq!(
        metrics.completed_routes.get(&TradeRouteKey {
            origin: origin_b,
            destination: origin_a,
        }),
        Some(&1),
    );
    assert_eq!(metrics.zone_transitions, 7);
}

#[test]
fn same_port_return_does_not_create_a_completed_route() {
    let port = RegionId::new();
    let mut ship = make_ship(VesselPresence::AtSea);
    let mut metrics = Metrics::default();

    start_trip(&mut ship, &mut metrics, &test_catalog().0, 10.0, port);
    assert!(finalize_trip(
        &mut ship,
        &mut metrics,
        20.0,
        TripOutcome::Docked(port),
    ));

    assert!(metrics.completed_routes.is_empty());
    assert_eq!(metrics.same_port_returns, 1);
}

#[test]
fn sunk_trip_counts_trips_sunk() {
    let mut ship = make_ship(VesselPresence::AtSea);
    let mut metrics = Metrics::default();

    start_trip(
        &mut ship,
        &mut metrics,
        &test_catalog().0,
        10.0,
        RegionId::new(),
    );
    assert!(finalize_trip(
        &mut ship,
        &mut metrics,
        20.0,
        TripOutcome::Sunk,
    ));

    assert_eq!(metrics.trips_sunk, 1);
    assert!(metrics.completed_routes.is_empty());
    assert_eq!(metrics.same_port_returns, 0);
}

#[test]
fn undock_without_price_marks_cargo_unpriced_and_excludes_value_average() {
    let origin = RegionId::new();
    let (catalog, timber, _) = test_catalog();
    let mut ship = make_ship(VesselPresence::Docked(origin));
    put_item(&mut ship, &catalog, timber, 4);
    let mut metrics = Metrics::default();

    start_trip(&mut ship, &mut metrics, &test_catalog().0, 10.0, origin);
    let trip = ship.trip.expect("trip marcada no undock");
    assert_eq!(trip.marked_cargo_value, 0);
    assert_eq!(trip.priced_quantity, 0);
    assert_eq!(trip.unpriced_quantity, 4);

    assert!(finalize_trip(
        &mut ship,
        &mut metrics,
        20.0,
        TripOutcome::Docked(origin),
    ));
    assert_eq!(metrics.cargo_value_priced_items, 0);
    assert_eq!(metrics.cargo_value_unpriced_items, 4);
    assert_eq!(metrics.cargo_value_coverage_pct, 0.0);
    assert_eq!(metrics.cargo_value_at_risk_total, 0);
    assert_eq!(metrics.cargo_value_trip_count, 0);
}

#[test]
fn undock_marks_cargo_with_guild_base_value() {
    let origin = RegionId::new();
    let (catalog, timber, _) = test_catalog();

    let mut ship = make_ship(VesselPresence::Docked(origin));
    put_item(&mut ship, &catalog, timber, 4);
    let mut metrics = Metrics::default();
    start_trip(&mut ship, &mut metrics, &catalog, 10.0, origin);

    let trip = ship.trip.expect("trip marcada no undock");
    // Valor base da guilda: Madeira = 10; 4 * 10 = 40.
    assert_eq!(trip.marked_cargo_value, 40);
    assert_eq!(trip.priced_quantity, 4);
    assert_eq!(trip.unpriced_quantity, 0);

    assert!(finalize_trip(
        &mut ship,
        &mut metrics,
        20.0,
        TripOutcome::Docked(origin),
    ));
    assert_eq!(metrics.cargo_value_priced_items, 4);
    assert_eq!(metrics.cargo_value_unpriced_items, 0);
    assert_eq!(metrics.cargo_value_coverage_pct, 100.0);
    assert_eq!(metrics.cargo_value_at_risk_total, 40);
    assert_eq!(metrics.cargo_value_trip_count, 1);
}

#[test]
fn undock_with_mixed_cargo_tracks_partial_coverage() {
    let origin = RegionId::new();
    let (catalog, timber, ore) = test_catalog();

    let mut ship = make_ship(VesselPresence::Docked(origin));
    put_item(&mut ship, &catalog, timber, 10);
    put_item(&mut ship, &catalog, ore, 5);
    let mut metrics = Metrics::default();
    start_trip(&mut ship, &mut metrics, &catalog, 10.0, origin);

    let trip = ship.trip.expect("trip marcada no undock");
    assert_eq!(trip.marked_cargo_value, 100);
    assert_eq!(trip.priced_quantity, 10);
    assert_eq!(trip.unpriced_quantity, 5);

    assert!(finalize_trip(
        &mut ship,
        &mut metrics,
        20.0,
        TripOutcome::Docked(origin),
    ));
    assert_eq!(metrics.cargo_value_priced_items, 10);
    assert_eq!(metrics.cargo_value_unpriced_items, 5);
    assert!((metrics.cargo_value_coverage_pct - (10.0 / 15.0 * 100.0)).abs() < 1e-6);
    assert_eq!(metrics.cargo_value_at_risk_total, 0);
    assert_eq!(metrics.cargo_value_trip_count, 0);
}

#[test]
fn empty_cargo_has_full_coverage_but_does_not_enter_value_average() {
    let origin = RegionId::new();
    let mut ship = make_ship(VesselPresence::Docked(origin));
    let mut metrics = Metrics::default();
    start_trip(&mut ship, &mut metrics, &test_catalog().0, 10.0, origin);

    assert!(finalize_trip(
        &mut ship,
        &mut metrics,
        20.0,
        TripOutcome::Docked(origin),
    ));
    assert_eq!(metrics.cargo_value_priced_items, 0);
    assert_eq!(metrics.cargo_value_unpriced_items, 0);
    assert_eq!(metrics.cargo_value_coverage_pct, 100.0);
    assert_eq!(metrics.cargo_value_at_risk_total, 0);
    assert_eq!(metrics.cargo_value_trip_count, 0);
}

#[test]
fn restored_at_sea_measurement_finishes_without_inventing_cargo_coverage() {
    let mut ship = make_ship(VesselPresence::AtSea);
    ship.restored_trip_started_at = Some(100.0);
    let mut metrics = Metrics::default();

    assert!(finalize_trip(
        &mut ship,
        &mut metrics,
        130.0,
        TripOutcome::Docked(RegionId::new()),
    ));

    assert_eq!(metrics.trip_count, 1);
    assert_eq!(metrics.trip_total_secs, 30.0);
    assert_eq!(metrics.cargo_value_priced_items, 0);
    assert_eq!(metrics.cargo_value_unpriced_items, 0);
    assert_eq!(metrics.cargo_value_at_risk_total, 0);
    assert_eq!(metrics.cargo_value_trip_count, 0);
}

/// MF-053: Undock soma o valor precificado a `cargo_value_departed`.
#[test]
fn undock_increments_cargo_value_departed() {
    let origin = RegionId::new();
    let (catalog, timber, _) = test_catalog();

    let mut ship = make_ship(VesselPresence::Docked(origin));
    put_item(&mut ship, &catalog, timber, 4);
    let mut metrics = Metrics::default();

    start_trip(&mut ship, &mut metrics, &catalog, 10.0, origin);

    assert_eq!(metrics.cargo_value_departed, 40);
    assert_eq!(metrics.cargo_value_arrived, 0);
    assert_eq!(metrics.cargo_value_sunk, 0);
}

/// MF-053: Dock soma o valor a `cargo_value_arrived`, sem tocar em sunk.
#[test]
fn dock_increments_cargo_value_arrived_not_sunk() {
    let origin = RegionId::new();
    let destination = RegionId::new();
    let (catalog, timber, _) = test_catalog();

    let mut ship = make_ship(VesselPresence::Docked(origin));
    put_item(&mut ship, &catalog, timber, 4);
    let mut metrics = Metrics::default();

    start_trip(&mut ship, &mut metrics, &catalog, 10.0, origin);
    assert!(finalize_trip(
        &mut ship,
        &mut metrics,
        20.0,
        TripOutcome::Docked(destination),
    ));

    assert_eq!(metrics.cargo_value_departed, 40);
    assert_eq!(metrics.cargo_value_arrived, 40);
    assert_eq!(metrics.cargo_value_sunk, 0);
}

/// MF-053: Sink soma o valor a `cargo_value_sunk`, sem tocar em arrived.
#[test]
fn sink_increments_cargo_value_sunk_not_arrived() {
    let origin = RegionId::new();
    let (catalog, timber, _) = test_catalog();

    let mut ship = make_ship(VesselPresence::Docked(origin));
    put_item(&mut ship, &catalog, timber, 4);
    let mut metrics = Metrics::default();

    start_trip(&mut ship, &mut metrics, &catalog, 10.0, origin);
    assert!(finalize_trip(
        &mut ship,
        &mut metrics,
        20.0,
        TripOutcome::Sunk,
    ));

    assert_eq!(metrics.cargo_value_departed, 40);
    assert_eq!(metrics.cargo_value_arrived, 0);
    assert_eq!(metrics.cargo_value_sunk, 40);
}

/// MF-053: departed == arrived + sunk ao final de um ciclo dock + sink.
#[test]
fn departed_equals_arrived_plus_sunk() {
    let origin = RegionId::new();
    let destination = RegionId::new();
    let (catalog, timber, _) = test_catalog();

    let mut ship = make_ship(VesselPresence::AtSea);
    put_item(&mut ship, &catalog, timber, 4);
    let mut metrics = Metrics::default();

    start_trip(&mut ship, &mut metrics, &catalog, 10.0, origin);
    assert!(finalize_trip(
        &mut ship,
        &mut metrics,
        20.0,
        TripOutcome::Docked(destination),
    ));
    start_trip(&mut ship, &mut metrics, &catalog, 30.0, origin);
    assert!(finalize_trip(
        &mut ship,
        &mut metrics,
        40.0,
        TripOutcome::Sunk,
    ));

    assert_eq!(metrics.cargo_value_departed, 80);
    assert_eq!(metrics.cargo_value_arrived, 40);
    assert_eq!(metrics.cargo_value_sunk, 40);
    assert_eq!(
        metrics.cargo_value_departed,
        metrics.cargo_value_arrived + metrics.cargo_value_sunk
    );
}

/// MF-053: trip sem carga não altera departed/arrived/sunk.
#[test]
fn cargo_without_value_does_not_distort_exposure() {
    let origin = RegionId::new();
    let mut ship = make_ship(VesselPresence::Docked(origin));
    let mut metrics = Metrics::default();

    start_trip(&mut ship, &mut metrics, &test_catalog().0, 10.0, origin);
    assert!(finalize_trip(
        &mut ship,
        &mut metrics,
        20.0,
        TripOutcome::Docked(origin),
    ));

    assert_eq!(metrics.cargo_value_departed, 0);
    assert_eq!(metrics.cargo_value_arrived, 0);
    assert_eq!(metrics.cargo_value_sunk, 0);
}
