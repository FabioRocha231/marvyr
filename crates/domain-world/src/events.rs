//! Eventos de mundo (MV-061): de tempos em tempos o mar anuncia algo que
//! puxa jogadores para o risco — uma Tormenta, uma Frota do Tesouro, um
//! Kraken, uma Maré de pérolas. Diretor seedado, determinístico e puro: o
//! servidor materializa cada evento (NPCs, tempestade, nó de recurso).

/// Tipo de evento.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum SeaEventKind {
    Tempest,
    TreasureFleet,
    Kraken,
    ContestedTide,
    /// Maré Sangrenta (Helltide do D4): o mar sem lei ferve, saqueadores
    /// de elite chegam em ondas, afundar solta Cinza Sangrenta e as cinzas
    /// abrem os Baús Malditos.
    BloodTide,
}

impl SeaEventKind {
    pub const ALL: [SeaEventKind; 5] = [
        SeaEventKind::Tempest,
        SeaEventKind::TreasureFleet,
        SeaEventKind::Kraken,
        SeaEventKind::ContestedTide,
        SeaEventKind::BloodTide,
    ];

    pub fn name(self) -> &'static str {
        match self {
            Self::Tempest => "Tormenta",
            Self::TreasureFleet => "Frota do Tesouro",
            Self::Kraken => "Kraken",
            Self::ContestedTide => "Maré de Pérolas",
            Self::BloodTide => "Maré Sangrenta",
        }
    }

    /// Duração (s) do evento.
    pub fn duration(self) -> f32 {
        match self {
            Self::Tempest => 240.0,
            Self::TreasureFleet => 420.0,
            Self::Kraken => 360.0,
            Self::ContestedTide => 300.0,
            Self::BloodTide => 360.0,
        }
    }

    /// Raio (m) da área anunciada.
    pub fn radius(self) -> f32 {
        match self {
            Self::Tempest => 420.0,
            Self::TreasureFleet => 260.0,
            Self::Kraken => 200.0,
            Self::ContestedTide => 120.0,
            Self::BloodTide => 380.0,
        }
    }
}

/// Evento em curso.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct SeaEvent {
    pub id: u32,
    pub kind: SeaEventKind,
    pub x: f32,
    pub y: f32,
    pub radius: f32,
    pub remaining: f32,
}

/// Mudanças de um passo do diretor.
#[derive(Debug, Clone, PartialEq)]
pub enum DirectorChange {
    Started(SeaEvent),
    Ended(SeaEvent),
}

/// Intervalo (s) entre o fim de um evento e o próximo.
/// MV-067: mar com evento quase sempre à vista (antes 4–8 min de espera).
const INTERVAL: (f32, f32) = (90.0, 180.0);
/// Primeiro evento depois do boot (s): dá tempo de o servidor encher.
const FIRST_EVENT: f32 = 60.0;

#[derive(Debug, Clone)]
pub struct SeaEventDirector {
    /// Locais de cada tipo (na ordem de `SeaEventKind::ALL`), do mapa.
    sites: [Vec<(f32, f32)>; 5],
    rng: u64,
    next_in: f32,
    next_id: u32,
    active: Option<SeaEvent>,
}

impl SeaEventDirector {
    pub fn new(seed: u64, map: &crate::map::WorldMap) -> Self {
        Self {
            sites: SeaEventKind::ALL.map(|kind| map.features().event_sites(kind).to_vec()),
            rng: seed ^ 0x9E37_79B9_7F4A_7C15,
            next_in: FIRST_EVENT,
            next_id: 1,
            active: None,
        }
    }

    pub fn active(&self) -> Option<&SeaEvent> {
        self.active.as_ref()
    }

    /// Dev/teste: dispara já o próximo evento deste tipo.
    pub fn force(&mut self, kind: SeaEventKind) -> SeaEvent {
        let event = self.start(kind);
        self.active = Some(event);
        event
    }

    /// Avança o relógio. Um evento por vez: o mar anuncia, o servidor
    /// inteiro converge, o evento termina, respira, e vem outro.
    pub fn step(&mut self, dt: f32) -> Vec<DirectorChange> {
        let mut changes = Vec::new();
        if let Some(event) = &mut self.active {
            event.remaining -= dt;
            if event.remaining <= 0.0 {
                changes.push(DirectorChange::Ended(*event));
                self.active = None;
                self.next_in = self.uniform(INTERVAL.0, INTERVAL.1);
            }
            return changes;
        }
        self.next_in -= dt;
        if self.next_in <= 0.0 {
            let kind =
                SeaEventKind::ALL[(self.next_u64() % SeaEventKind::ALL.len() as u64) as usize];
            let event = self.start(kind);
            self.active = Some(event);
            changes.push(DirectorChange::Started(event));
        }
        changes
    }

    fn start(&mut self, kind: SeaEventKind) -> SeaEvent {
        // `ALL` segue a ordem de declaração do enum.
        let roll = self.next_u64();
        let sites = &self.sites[kind as usize];
        let (x, y) = sites[(roll % sites.len() as u64) as usize];
        let id = self.next_id;
        self.next_id += 1;
        SeaEvent {
            id,
            kind,
            x,
            y,
            radius: kind.radius(),
            remaining: kind.duration(),
        }
    }

    // splitmix64 — mesmo gerador barato do resto do domínio.
    fn next_u64(&mut self) -> u64 {
        self.rng = self.rng.wrapping_add(0x9E37_79B9_7F4A_7C15);
        let mut z = self.rng;
        z = (z ^ (z >> 30)).wrapping_mul(0xBF58_476D_1CE4_E5B9);
        z = (z ^ (z >> 27)).wrapping_mul(0x94D0_49BB_1331_11EB);
        z ^ (z >> 31)
    }

    fn uniform(&mut self, min: f32, max: f32) -> f32 {
        let unit = (self.next_u64() >> 40) as f32 / (1u64 << 24) as f32;
        min + (max - min) * unit
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::map::WorldMap;
    use crate::risk::RiskTier;

    #[test]
    fn one_event_at_a_time_with_a_breather_between() {
        let mut director = SeaEventDirector::new(42, &WorldMap::vertical_slice());
        assert!(director.step(FIRST_EVENT - 1.0).is_empty());
        let started = director.step(2.0);
        let DirectorChange::Started(event) = started[0] else {
            panic!("esperava início, veio {started:?}");
        };
        assert!(director.step(10.0).is_empty(), "sem sobreposição");
        let ended = director.step(event.kind.duration());
        assert_eq!(ended, vec![DirectorChange::Ended(director_event(event))]);
        assert!(director.active().is_none());
        assert!(director.step(INTERVAL.0 - 1.0).is_empty(), "respiro");
    }

    fn director_event(mut event: SeaEvent) -> SeaEvent {
        event.remaining -= 10.0 + event.kind.duration();
        event
    }

    #[test]
    fn same_seed_same_story() {
        let run = |seed| {
            let mut director = SeaEventDirector::new(seed, &WorldMap::vertical_slice());
            (0..20_000)
                .flat_map(|_| director.step(1.0))
                .collect::<Vec<_>>()
        };
        assert_eq!(run(7), run(7));
        assert!(run(7).len() > 4);
    }

    #[test]
    fn events_never_start_in_protected_waters_or_on_land() {
        let map = WorldMap::vertical_slice();
        for kind in SeaEventKind::ALL {
            for &(x, y) in map.features().event_sites(kind) {
                let zone = map.zone_at(x, y).unwrap();
                assert_ne!(zone.tier, RiskTier::Protected, "{kind:?} em {x},{y}");
                assert!(!map.is_land(x, y), "{kind:?} em terra {x},{y}");
            }
        }
        let map = map.with_hidden_islands();
        for leg in map.features().fleet_route.windows(2) {
            let ((ax, ay), (bx, by)) = (leg[0], leg[1]);
            for step in 0..=20 {
                let t = step as f32 / 20.0;
                let (x, y) = (ax + (bx - ax) * t, ay + (by - ay) * t);
                assert!(
                    map.push_out_of_land(x, y, 40.0).is_none(),
                    "rota do comboio raspa terra em {x},{y}"
                );
            }
        }
    }

    #[test]
    fn forced_event_is_active() {
        let mut director = SeaEventDirector::new(1, &WorldMap::vertical_slice());
        let event = director.force(SeaEventKind::Kraken);
        assert_eq!(director.active(), Some(&event));
    }
}
