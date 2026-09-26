//! Interest management por grid espacial (ADR-0009, MF-031).
//!
//! O servidor não replica o mundo inteiro para todos: cada cliente recebe um
//! snapshot construído do **seu** ponto de vista — entidades dos chunks
//! visíveis mais um anel de borda. O anel é o que evita "pop" brusco: uma
//! entidade continua visível por um chunk inteiro depois de cruzar a fronteira.
//!
//! Constantes de configuração vivem AQUI (MF-031: nada espalhado pelo código).
//! Toda a lógica é pura: o sistema ECS só coleta os estados e chama
//! [`build_snapshot`] por destinatário.

use marvyr_protocol::{ProjectileState, ShipState, WorldSnapshot, WreckState};

/// Lado do chunk da grade espacial, em metros (ADR-0009: 256 m — métrica a
/// validar com benchmarks; não mudar sem nova decisão).
pub const CHUNK_SIZE: f32 = 256.0;

/// Anel de borda em chunks: entidades até `VIEW_RING` chunks de distância
/// Chebyshev do chunk do observador entram no snapshot dele.
pub const VIEW_RING: i32 = 1;

/// Índice do chunk numa coordenada (divisão de piso: cobre negativos).
pub fn chunk_of(coordinate: f32) -> i32 {
    (coordinate / CHUNK_SIZE).floor() as i32
}

/// Distância Chebyshev entre chunks (o anel é quadrado, não circular).
pub fn chunk_distance(a: (i32, i32), b: (i32, i32)) -> i32 {
    (a.0 - b.0).abs().max((a.1 - b.1).abs())
}

/// Uma entidade em `target` entra no snapshot de quem observa de `viewer`?
/// O próprio observador (distância 0) sempre se enxerga.
pub fn is_visible(viewer: (f32, f32), target: (f32, f32)) -> bool {
    chunk_distance(
        (chunk_of(viewer.0), chunk_of(viewer.1)),
        (chunk_of(target.0), chunk_of(target.1)),
    ) <= VIEW_RING
}

/// Snapshot do ponto de vista de um observador (ADR-0009): cada lista de
/// entidades é recortada pelos chunks visíveis + anel de borda. Puro — o
/// teste cobre os cinco cenários exigidos pelo MF-031.
pub fn build_snapshot(
    tick: u64,
    viewer: (f32, f32),
    ships: &[ShipState],
    projectiles: &[ProjectileState],
    wrecks: &[WreckState],
) -> WorldSnapshot {
    WorldSnapshot {
        tick,
        ships: ships
            .iter()
            .copied()
            .filter(|state| is_visible(viewer, (state.x, state.y)))
            .collect(),
        projectiles: projectiles
            .iter()
            .copied()
            .filter(|state| is_visible(viewer, (state.x, state.y)))
            .collect(),
        wrecks: wrecks
            .iter()
            .copied()
            .filter(|state| is_visible(viewer, (state.x, state.y)))
            .collect(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn ship(id: u32, x: f32, y: f32) -> ShipState {
        ShipState {
            ship_id: id,
            kind: marvyr_domain_ships::ShipKind::SmallMerchant,
            x,
            y,
            heading: 0.0,
            speed: 0.0,
            cargo_weight: 0,
            hp: 100,
            max_hp: 100,
            max_speed: 30.0,
            weapon_damage: 20,
            weapon_range: 50.0,
            port_cooldown_secs: 0.0,
            starboard_cooldown_secs: 0.0,
            is_npc: false,
            cargo_capacity: 100,
            sail_hp: 100.0,
            ammo: Default::default(),
            faction: marvyr_protocol::Faction::Player,
            notoriety_tier: 0,
            rudder_hp: 100.0,
            crew: 0,
            crew_max: 0,
            repairing: false,
            dig_progress: 0.0,
            sail_cosmetic: 0,
            flag_cosmetic: 0,
            black_flag: 0,
            fire_target: None,
            aura: 0,
            flasks: Default::default(),
        }
    }

    fn viewer_ships(viewer: (f32, f32), ships: &[ShipState]) -> Vec<u32> {
        build_snapshot(0, viewer, ships, &[], &[])
            .ships
            .iter()
            .map(|state| state.ship_id)
            .collect()
    }

    /// MF-031: cliente A não recebe navio fora do AOI.
    #[test]
    fn ship_outside_the_grid_is_invisible() {
        // 600 m no eixo X = 3 chunks de distância (> anel 1).
        let seen = viewer_ships((0.0, 0.0), &[ship(1, 600.0, 0.0)]);
        assert!(seen.is_empty());
    }

    /// MF-031: cliente A recebe o navio quando ele entra no AOI.
    #[test]
    fn ship_becomes_visible_when_entering() {
        let ships = [ship(1, 250.0, 0.0)];
        assert!(viewer_ships((0.0, 0.0), &ships).contains(&1));
    }

    /// MF-031: o anel de borda evita pop brusco — uma entidade logo depois
    /// da fronteira do chunk (513 m) ainda é visível para quem está no
    /// chunk 0, porque ela cai no chunk 1 (dentro do anel).
    #[test]
    fn border_ring_smooths_chunk_transitions() {
        let just_across = [ship(1, CHUNK_SIZE + 1.0, 0.0)];
        assert!(viewer_ships((0.0, 0.0), &just_across).contains(&1));

        // Mas o anel tem fim: dois chunks além já somem (sem pop dentro do
        // anel, e banda cortada fora dele).
        let past_ring = [ship(2, CHUNK_SIZE * 2.0 + 1.0, 0.0)];
        assert!(!viewer_ships((0.0, 0.0), &past_ring).contains(&2));
    }

    /// MF-031: projétil fora do AOI não aparece no snapshot do observador.
    #[test]
    fn projectile_outside_aoi_is_not_sent() {
        let projectiles = [ProjectileState {
            projectile_id: 7,
            x: 900.0,
            y: 0.0,
            heading: 0.0,
        }];
        let snapshot = build_snapshot(0, (0.0, 0.0), &[], &projectiles, &[]);
        assert!(snapshot.projectiles.is_empty());
    }

    /// MF-031: dois clientes no mesmo tick recebem snapshots diferentes —
    /// cada um vê o que está perto de SI.
    #[test]
    fn two_viewers_receive_different_snapshots() {
        let ships = [ship(1, 250.0, 0.0), ship(2, -5000.0, 5000.0)];
        let near_a = build_snapshot(0, (250.0, 0.0), &ships, &[], &[]);
        let far_b = build_snapshot(0, (-5000.0, 5000.0), &ships, &[], &[]);
        assert_eq!(
            near_a.ships.iter().map(|s| s.ship_id).collect::<Vec<_>>(),
            vec![1]
        );
        assert_eq!(
            far_b.ships.iter().map(|s| s.ship_id).collect::<Vec<_>>(),
            vec![2]
        );
    }

    /// Wrecks seguem a mesma visibilidade dos navios (MF-031: "quando
    /// possível" — aqui é obrigatório e sai de graça pelo mesmo caminho).
    #[test]
    fn wrecks_follow_the_same_visibility() {
        let wrecks = [WreckState {
            wreck_id: 9,
            x: 300.0,
            y: 0.0,
            stack_count: 2,
            best_rarity: 0,
        }];
        let near = build_snapshot(0, (250.0, 0.0), &[], &[], &wrecks);
        assert_eq!(near.wrecks.len(), 1);
        let far = build_snapshot(0, (-4000.0, 0.0), &[], &[], &wrecks);
        assert!(far.wrecks.is_empty());
    }

    /// O observador sempre se enxerga, não importa onde esteja.
    #[test]
    fn viewer_always_sees_itself() {
        let ships = [ship(1, -4200.0, 3300.0)];
        assert!(viewer_ships((-4200.0, 3300.0), &ships).contains(&1));
    }

    /// Coordenadas negativas dividem por piso (chunk -1 começa em -256).
    #[test]
    fn negative_coordinates_floor_partition() {
        assert_eq!(chunk_of(-1.0), -1);
        assert_eq!(chunk_of(-CHUNK_SIZE), -1);
        assert_eq!(chunk_of(-CHUNK_SIZE - 1.0), -2);
        assert_eq!(chunk_of(CHUNK_SIZE), 1);
        // Borda exata entre chunks -1 e 0: -256 pertence a -1, então quem
        // está em -0.5 (chunk -1) vê quem está em -256.0.
        assert!(is_visible((-0.5, 0.0), (-CHUNK_SIZE, 0.0)));
    }
}
