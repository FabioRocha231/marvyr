use marvyr_domain_world::WorldMap;
fn main() {
    for (label, map) in [
        ("classic", WorldMap::from_seed(0)),
        ("seed", WorldMap::from_seed(0x4D41_5256_5952_0001)),
    ] {
        let f = map.features();
        println!("== {label}: areas={} nodes={} pirates={} raiders={} navy={} tempest={} kraken={} tide={} hidden={}",
            f.areas.len(), f.nodes.len(), f.pirate_spawns.len(), f.raider_spawns.len(), f.navy_spawns.len(),
            f.tempest_sites.len(), f.kraken_sites.len(), f.tide_sites.len(), f.hidden_islands.len());
        let mut water = 0.0;
        for n in f.nodes.iter().take(6) {
            println!("  node {:?} ({:.0},{:.0})", n.name, n.x, n.y);
        }
        for a in &f.areas {
            println!("  centro {} = ({:.0}, {:.0})", a.name, a.x, a.y);
            let in_area = |p: &(f32, f32)| (p.0 - a.x).hypot(p.1 - a.y) <= a.radius;
            let nodes = f.nodes.iter().filter(|n| in_area(&(n.x, n.y))).count();
            let npcs = f
                .pirate_spawns
                .iter()
                .chain(&f.raider_spawns)
                .chain(&f.navy_spawns)
                .filter(|p| in_area(p))
                .count();
            water += std::f32::consts::PI * a.radius * a.radius;
            println!(
                "  {:<28} {:?} r={:>5.0} nodes={} npcs={}",
                a.name, a.tier, a.radius, nodes, npcs
            );
        }
        let route: f32 = f
            .caravan_route
            .windows(2)
            .filter(|w| map.area_at(w[0].0, w[0].1) == map.area_at(w[1].0, w[1].1))
            .map(|w| (w[0].0 - w[1].0).hypot(w[0].1 - w[1].1))
            .sum();
        println!(
            "  agua navegavel={:.1} km2, serra->mina navegando={:.0} m",
            water / 1e6,
            route
        );
    }
}
