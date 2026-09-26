//! v48: correntes marítimas — camada rotativa semanal. Cada zona ganha uma
//! faixa de água rápida (uma corda do mar redondo), sorteada por (seed,
//! semana): o mapa base não muda, as rotas rápidas mudam toda semana.

use crate::map::WorldMap;
use crate::portal::Rng;

/// Largura da faixa (m).
pub const WIDTH: f32 = 80.0;
/// Quanto a corrente empurra (m/s).
pub const PUSH: f32 = 3.5;

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Current {
    pub from: (f32, f32),
    pub to: (f32, f32),
}

impl Current {
    /// Empurrão (m/s) no ponto: ao longo da faixa, zero fora dela.
    pub fn push_at(&self, x: f32, y: f32) -> (f32, f32) {
        let (dx, dy) = (self.to.0 - self.from.0, self.to.1 - self.from.1);
        let length_sq = dx * dx + dy * dy;
        if length_sq <= f32::EPSILON {
            return (0.0, 0.0);
        }
        let t = ((x - self.from.0) * dx + (y - self.from.1) * dy) / length_sq;
        if !(0.0..=1.0).contains(&t) {
            return (0.0, 0.0);
        }
        let (px, py) = (self.from.0 + dx * t, self.from.1 + dy * t);
        if (x - px).powi(2) + (y - py).powi(2) > (WIDTH / 2.0).powi(2) {
            return (0.0, 0.0);
        }
        let length = length_sq.sqrt();
        (dx / length * PUSH, dy / length * PUSH)
    }
}

/// As correntes da semana: uma por zona, mesma entrada, mesmas faixas.
pub fn currents(map: &WorldMap, week: u32) -> Vec<Current> {
    let seed = map.features().seed;
    let mut rng =
        Rng::new(seed ^ u64::from(week).wrapping_mul(0x9E37_79B9_7F4A_7C15) ^ 0x00C0_22E7);
    map.features()
        .areas
        .iter()
        .map(|area| {
            let angle = rng.next_f32() * std::f32::consts::TAU;
            let offset = (rng.next_f32() - 0.5) * 0.8 * area.radius;
            let (dx, dy) = (angle.cos(), angle.sin());
            let (cx, cy) = (area.x - dy * offset, area.y + dx * offset);
            let half = 0.6 * area.radius;
            Current {
                from: (cx - dx * half, cy - dy * half),
                to: (cx + dx * half, cy + dy * half),
            }
        })
        .collect()
}

/// Soma dos empurrões no ponto (faixas não se cruzam na prática).
pub fn push_at(currents: &[Current], x: f32, y: f32) -> (f32, f32) {
    currents.iter().fold((0.0, 0.0), |(ax, ay), current| {
        let (px, py) = current.push_at(x, y);
        (ax + px, ay + py)
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn same_week_same_lanes_next_week_new_ones() {
        let map = WorldMap::from_seed(0);
        let a = currents(&map, 2_960);
        assert_eq!(a, currents(&map, 2_960));
        assert_ne!(a, currents(&map, 2_961));
        assert_eq!(a.len(), map.features().areas.len());
    }

    #[test]
    fn a_lane_pushes_along_itself_and_nowhere_else() {
        let lane = Current {
            from: (0.0, 0.0),
            to: (1_000.0, 0.0),
        };
        assert_eq!(lane.push_at(500.0, 10.0), (PUSH, 0.0));
        assert_eq!(lane.push_at(500.0, WIDTH), (0.0, 0.0));
        assert_eq!(lane.push_at(-50.0, 0.0), (0.0, 0.0));
    }

    #[test]
    fn lanes_stay_inside_their_zone() {
        let map = WorldMap::from_seed(0);
        for (lane, area) in currents(&map, 3_000).iter().zip(&map.features().areas) {
            for end in [lane.from, lane.to] {
                let d = ((end.0 - area.x).powi(2) + (end.1 - area.y).powi(2)).sqrt();
                assert!(d < area.radius, "{} fora do mar", area.name);
            }
        }
    }
}
