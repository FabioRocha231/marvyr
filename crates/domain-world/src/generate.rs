//! Mundo procedural em zonas (MV-065, MV-066). A seed monta um grafo de
//! zonas no estilo Albion — as duas capitais da coroa em baías protegidas, a
//! Rota da Costa e os corredores de fronteira, o Mar do Coral Negro sem lei
//! e as Águas Negras além dele, mais 1-2 portos livres pendurados em lugares
//! sorteados — e gera a geografia de cada zona.
//!
//! Cada zona é um mar redondo cercado de paredão, numa região distante do
//! mesmo plano; portões nos vãos do paredão levam à vizinha. A distância
//! entre as zonas isola AOI, combate e streaming sem sharding.
//!
//! A estrutura local (costa atrás da capital, corpo da ilha, nós, pontos de
//! NPC e de evento) vem de moldes medidos no mapa clássico, reposicionados
//! no referencial de cada lugar. O que é espalhado (rochedos, ilhotas, ilhas
//! ocultas) é rejeitado perto de rotas, portos, nós e pontos de conteúdo —
//! garantia construtiva que os testes conferem em centenas de seeds.
//!
//! Como no Albion, o mapa-base é fixo depois de gerado (a seed é
//! configuração do servidor); o que muda sempre é a camada viva por cima.

use std::f32::consts::TAU;
use std::ops::{Add, Mul, Sub};

use crate::features::{
    instance_nodes, maelstrom_label, Area, Features, NodeSpot, Sector, ZoneExit, ISLAND,
};
use crate::land::{push_out_of_land, LandMass};
use crate::map::{
    instance_land, instance_zones, region, zone, WorldMap, AREA_WALL_BAND, BLACK_WATERS,
    PIRATE_PORT,
};
use crate::region::{Port, Region};
use crate::risk::RiskTier;
use crate::treasure::HiddenIsland;
use crate::zone::Zone;

const SERRA: &str = "Porto da Serra";
const MINA: &str = "Porto da Mina";
/// Nós das fronteiras (MV-067): madeira e minério no meio do caminho.
pub(crate) const DRIFTWOOD: &str = "Madeira à Deriva";
pub(crate) const SUNKEN_ORE: &str = "Veio Submerso";
/// Raio das águas protegidas em volta de cada porto.
const PORT_WATERS: f32 = 200.0;

/// Zonas vizinhas ficam a esta distância no plano (bem além de qualquer AOI).
const SPACING: f32 = 6000.0;
/// O grafo começa longe das instâncias (Cerração a x=4200, Sorvedouro a -4200).
const ORIGIN_Y: f32 = 20_000.0;
const WALL_RADIUS: f32 = 170.0;
const WALL_STEP: f32 = 200.0;
/// Batentes do portão: o vão fica com ±230 m de largura no anel.
const GATE_JAMB: f32 = 400.0;
/// O portal cobre o vão inteiro, inclusive a quina externa do paredão.
const GATE_RADIUS: f32 = 320.0;
/// Quem atravessa reaparece esta distância para dentro do raio navegável.
const ENTRY_DEPTH: f32 = 250.0;

/// Costa atrás de cada capital: (para fora, de lado, raio) — o molde da
/// costa da Serra no mapa clássico.
const COAST: [(f32, f32, f32); 6] = [
    (220.0, 0.0, 190.0),
    (200.0, -220.0, 150.0),
    (190.0, 230.0, 150.0),
    (350.0, 420.0, 250.0),
    (350.0, -440.0, 250.0),
    (550.0, 0.0, 260.0),
];
/// Nós do porto livre no mesmo referencial (direção a partir do cais; o nó
/// mesmo fica fora das águas protegidas).
const CAPITAL_NODES: [(f32, f32); 5] = [
    (20.0, 175.0),
    (-100.0, 130.0),
    (10.0, -165.0),
    (-130.0, -70.0),
    (-170.0, 20.0),
];
/// Depósitos da Serra e da Mina na Rota da Costa: (da boca da baía para o
/// centro da rota, de lado da faixa das caravanas, estoque). O último é o
/// "do Caminho". Poucos nós fartos: cada nó reserva mar livre em volta, e a
/// rota ainda precisa de espaço para rochedos e ilhas ocultas.
const ROAD_DEPOSITS: [(f32, f32, u32); 3] = [
    (170.0, -180.0, 120),
    (260.0, 250.0, 120),
    (340.0, -230.0, 60),
];
/// Corpo da Ilha do Coral Negro: (de lado, para fora, raio).
const ISLAND_BODY: [(f32, f32, f32); 4] = [
    (0.0, 0.0, 65.0),
    (-50.0, -30.0, 40.0),
    (50.0, 20.0, 42.0),
    (10.0, 55.0, 45.0),
];
/// Recifes das Águas Negras em volta do centro.
const REEFS: [(f32, f32, f32); 6] = [
    (-220.0, -250.0, 30.0),
    (180.0, -170.0, 36.0),
    (-60.0, 40.0, 26.0),
    (260.0, 150.0, 32.0),
    (-300.0, 210.0, 38.0),
    (60.0, 360.0, 28.0),
];
const HIDDEN_NAMES: [&str; 4] = [
    "Ilhota da Caveira",
    "Recife dos Afogados",
    "Baixio do Enforcado",
    "Atol Sem Nome",
];
/// Portos livres: (nome da zona, nome do porto). Neutros — fora da coroa.
pub(crate) const FREE_PORTS: [(&str, &str); 4] = [
    ("Enseada das Gaivotas", "Porto das Gaivotas"),
    ("Costa do Farol", "Porto do Farol"),
    ("Baixios da Areia Branca", "Porto da Areia Branca"),
    ("Mar de Santa Luzia", "Porto Santa Luzia"),
];
pub(crate) const FREE_TIMBER: &str = "Mata Costeira";
pub(crate) const FREE_ORE: &str = "Jazida Costeira";

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Role {
    Serra,
    Mina,
    Road,
    Dawn,
    Dusk,
    Coral,
    Black,
    Free(usize),
}

impl Role {
    fn name(self) -> &'static str {
        match self {
            Role::Serra => "Baía da Serra",
            Role::Mina => "Baía da Mina",
            Role::Road => "Rota da Costa",
            Role::Dawn => "Corredor do Amanhecer",
            Role::Dusk => "Corredor do Poente",
            Role::Coral => "Mar do Coral Negro",
            Role::Black => BLACK_WATERS,
            Role::Free(index) => FREE_PORTS[index].0,
        }
    }

    fn tier(self) -> RiskTier {
        match self {
            Role::Serra | Role::Mina => RiskTier::Protected,
            Role::Road | Role::Dawn | Role::Dusk | Role::Free(_) => RiskTier::Frontier,
            Role::Coral | Role::Black => RiskTier::Lawless,
        }
    }

    /// Raio navegável. MV-067: compacto de propósito — cada zona cabe em
    /// poucas telas e sempre tem algo à vista (o mundo grande e vazio
    /// cansava em 10 minutos).
    fn radius(self) -> f32 {
        match self {
            Role::Serra | Role::Mina => 780.0,
            Role::Road | Role::Dawn | Role::Dusk => 720.0,
            Role::Free(_) => 720.0,
            Role::Coral => 1050.0,
            Role::Black => 1000.0,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq)]
struct V(f32, f32);

impl Add for V {
    type Output = V;
    fn add(self, o: V) -> V {
        V(self.0 + o.0, self.1 + o.1)
    }
}
impl Sub for V {
    type Output = V;
    fn sub(self, o: V) -> V {
        V(self.0 - o.0, self.1 - o.1)
    }
}
impl Mul<f32> for V {
    type Output = V;
    fn mul(self, k: f32) -> V {
        V(self.0 * k, self.1 * k)
    }
}
impl V {
    fn len(self) -> f32 {
        self.0.hypot(self.1)
    }
    fn norm(self) -> V {
        self * (1.0 / self.len().max(f32::EPSILON))
    }
    fn dot(self, o: V) -> f32 {
        self.0 * o.0 + self.1 * o.1
    }
    fn perp(self) -> V {
        V(-self.1, self.0)
    }
    fn polar(angle: f32, r: f32) -> V {
        V(angle.cos() * r, angle.sin() * r)
    }
    fn angle(self) -> f32 {
        self.1.atan2(self.0)
    }
    fn t(self) -> (f32, f32) {
        (self.0, self.1)
    }
}

/// splitmix64 — o mesmo gerador barato do resto do domínio.
pub(crate) struct Rng(pub(crate) u64);

impl Rng {
    fn next(&mut self) -> u64 {
        self.0 = self.0.wrapping_add(0x9E37_79B9_7F4A_7C15);
        let mut z = self.0;
        z = (z ^ (z >> 30)).wrapping_mul(0xBF58_476D_1CE4_E5B9);
        z = (z ^ (z >> 27)).wrapping_mul(0x94D0_49BB_1331_11EB);
        z ^ (z >> 31)
    }
    pub(crate) fn range(&mut self, min: f32, max: f32) -> f32 {
        min + (max - min) * ((self.next() >> 40) as f32 / (1u64 << 24) as f32)
    }
    fn coin(&mut self) -> bool {
        self.next() & 1 == 1
    }
    fn pick(&mut self, len: usize) -> usize {
        (self.next() % len as u64) as usize
    }
}

/// Distância de `p` ao segmento `a`-`b`.
fn segment_distance(p: V, a: V, b: V) -> f32 {
    let ab = b - a;
    let t = ((p - a).dot(ab) / ab.dot(ab).max(f32::EPSILON)).clamp(0.0, 1.0);
    (p - (a + ab * t)).len()
}

fn angle_gap(a: f32, b: f32) -> f32 {
    let d = (a - b).rem_euclid(TAU);
    d.min(TAU - d)
}

/// O que o espalhamento não pode tocar: rotas (segmentos) e pontos de
/// conteúdo, cada um com sua folga.
#[derive(Default)]
struct KeepClear {
    lanes: Vec<(V, V)>,
    points: Vec<(V, f32)>,
}

impl KeepClear {
    fn allows(&self, at: V, radius: f32, lane_gap: f32) -> bool {
        self.lanes
            .iter()
            .all(|&(a, b)| segment_distance(at, a, b) > radius + lane_gap)
            && self
                .points
                .iter()
                .all(|&(p, gap)| (at - p).len() > radius + gap)
    }
}

fn clear_of_land(land: &[LandMass], at: V, radius: f32, gap: f32) -> bool {
    land.iter()
        .all(|m| (at - V(m.x, m.y)).len() > radius + m.radius + gap)
}

/// Ponto de conteúdo na água: sai da terra com folga se o molde caiu na
/// costa.
fn settle(land: &[LandMass], at: V, clearance: f32) -> V {
    push_out_of_land(land, at.0, at.1, clearance)
        .map(|(x, y)| V(x, y))
        .unwrap_or(at)
}

fn sector(center: V, half: f32) -> Sector {
    (
        center.0 - half,
        center.0 + half,
        center.1 - half,
        center.1 + half,
    )
}

fn disc(at: V, radius: f32) -> LandMass {
    LandMass::new(at.0, at.1, radius)
}

fn node(name: &'static str, region: &'static str, at: V, max_stock: u32) -> NodeSpot {
    NodeSpot {
        name,
        region,
        x: at.0,
        y: at.1,
        max_stock,
    }
}

/// Uma zona em construção.
struct Plan {
    role: Role,
    center: V,
    radius: f32,
    cell: (i32, i32),
}

impl Plan {
    fn inside(&self, at: V, span: f32, margin: f32) -> bool {
        (at - self.center).len() + span < self.radius - margin
    }
}

/// Topologia: células fixas do núcleo, portos livres e atalhos sorteados.
fn plan_graph(rng: &mut Rng) -> (Vec<Plan>, Vec<(usize, usize)>) {
    let mirror = rng.coin();
    let mut roles = vec![
        (Role::Serra, (0, 0)),
        (Role::Road, (1, 0)),
        (Role::Mina, (2, 0)),
        (Role::Dawn, (0, 1)),
        (Role::Dusk, (2, 1)),
        (Role::Coral, (1, 2)),
        (Role::Black, (1, 3)),
    ];
    let (serra, road, mina, dawn, dusk, coral, black) = (0, 1, 2, 3, 4, 5, 6);
    let mut edges = vec![
        (serra, road),
        (road, mina),
        (serra, dawn),
        (mina, dusk),
        (dawn, coral),
        (dusk, coral),
        (coral, black),
    ];
    // Portos livres: 1-2 dos lugares candidatos, cada um com seu nome.
    let candidates: [((i32, i32), &[usize]); 4] = [
        ((-1, 1), &[dawn]),
        ((3, 1), &[dusk]),
        ((1, -1), &[road]),
        ((1, 1), &[road, coral]),
    ];
    let mut free_slots: Vec<usize> = (0..candidates.len()).collect();
    let mut names: Vec<usize> = (0..FREE_PORTS.len()).collect();
    for _ in 0..1 + usize::from(rng.coin()) {
        let slot = free_slots.remove(rng.pick(free_slots.len()));
        let name = names.remove(rng.pick(names.len()));
        let (cell, links) = candidates[slot];
        let index = roles.len();
        roles.push((Role::Free(name), cell));
        edges.extend(links.iter().map(|&link| (link, index)));
    }
    // Atalho de fronteira: um corredor às vezes também toca a Rota.
    if rng.coin() {
        edges.push((if rng.coin() { dawn } else { dusk }, road));
    }
    let plans = roles
        .into_iter()
        .map(|(role, (col, row))| {
            let col = if mirror { 2 - col } else { col };
            let center = V(
                col as f32 * SPACING + rng.range(-400.0, 400.0),
                ORIGIN_Y + row as f32 * SPACING + rng.range(-400.0, 400.0),
            );
            Plan {
                role,
                center,
                radius: role.radius() * rng.range(0.92, 1.08),
                cell: (col, row),
            }
        })
        .collect();
    (plans, edges)
}

pub(crate) fn generate(seed: u64) -> WorldMap {
    let mut rng = Rng(seed);
    let (plans, edges) = plan_graph(&mut rng);
    let index_of = |role: Role| plans.iter().position(|p| p.role == role).unwrap_or(0);

    // 1. Portões: um par por aresta. `mouth[a][b]` é o ponto de chegada em
    // `a` vindo de `b` (e a boca de onde se parte de `a` para `b`).
    let mut exits: Vec<ZoneExit> = Vec::new();
    for &(a, b) in &edges {
        for (from, to) in [(a, b), (b, a)] {
            let (pf, pt) = (&plans[from], &plans[to]);
            let dir = (pt.center - pf.center).norm();
            let gate = pf.center + dir * (pf.radius + WALL_RADIUS);
            let dest = pt.center - dir * (pt.radius - ENTRY_DEPTH);
            exits.push(ZoneExit {
                x: gate.0,
                y: gate.1,
                radius: GATE_RADIUS,
                from,
                to,
                dest: dest.t(),
            });
        }
    }
    let mouth = |from: usize, to: usize| -> V {
        let (pf, pt) = (&plans[from], &plans[to]);
        pf.center + (pt.center - pf.center).norm() * (pf.radius - ENTRY_DEPTH)
    };
    let exit_between = |from: usize, to: usize| -> ZoneExit {
        *exits
            .iter()
            .find(|e| e.from == from && e.to == to)
            .expect("aresta tem portão nos dois sentidos")
    };

    // 2. Paredão de cada zona, com vão (e batentes) em cada portão.
    let mut land: Vec<LandMass> = Vec::new();
    for (index, plan) in plans.iter().enumerate() {
        let ring = plan.radius + WALL_RADIUS;
        let gates: Vec<f32> = exits
            .iter()
            .filter(|e| e.from == index)
            .map(|e| (V(e.x, e.y) - plan.center).angle())
            .collect();
        let count = (TAU * ring / WALL_STEP).ceil() as usize;
        for k in 0..count {
            let angle = k as f32 / count as f32 * TAU;
            if gates
                .iter()
                .any(|&g| angle_gap(angle, g) * ring < GATE_JAMB)
            {
                continue;
            }
            let at = plan.center + V::polar(angle, ring);
            land.push(LandMass::cliff(at.0, at.1, WALL_RADIUS));
        }
        for &g in &gates {
            for side in [-1.0, 1.0] {
                let at = plan.center + V::polar(g + side * GATE_JAMB / ring, ring);
                land.push(LandMass::cliff(at.0, at.1, WALL_RADIUS));
            }
        }
    }

    // 3. Conteúdo ancorado por papel da zona.
    let mut keep: Vec<KeepClear> = plans.iter().map(|_| KeepClear::default()).collect();
    // Rotas dentro de cada zona: de cada chegada até cada outro portão (o
    // trecho que um NPC percorre), e as próprias chegadas.
    for (index, _) in plans.iter().enumerate() {
        let gates: Vec<(V, V)> = exits
            .iter()
            .filter(|e| e.from == index)
            .map(|e| (mouth(index, e.to), V(e.x, e.y)))
            .collect();
        for (i, &(arrival, _)) in gates.iter().enumerate() {
            keep[index].points.push((arrival, 150.0));
            for (j, &(_, gate)) in gates.iter().enumerate() {
                if i != j {
                    keep[index].lanes.push((arrival, gate));
                }
            }
        }
    }

    let mut nodes: Vec<NodeSpot> = Vec::new();
    let mut regions: Vec<Region> = Vec::new();
    let mut zones: Vec<Zone> = Vec::new();
    let mut labels: Vec<(&'static str, f32, f32)> = Vec::new();
    let mut tempest_sites: Vec<V> = Vec::new();
    let mut kraken_sites: Vec<V> = Vec::new();
    let mut tide_sites: Vec<V> = Vec::new();
    let mut pirate_spawns: Vec<V> = Vec::new();
    let mut raider_spawns: Vec<V> = Vec::new();
    let mut navy_spawns: Vec<V> = Vec::new();
    let mut fleet_route: Vec<V> = Vec::new();
    let mut docks: Vec<(Role, V)> = Vec::new();

    let road = index_of(Role::Road);
    for (index, plan) in plans.iter().enumerate() {
        let c = plan.center;
        let r = plan.radius;
        let gate_dirs: Vec<V> = exits
            .iter()
            .filter(|e| e.from == index)
            .map(|e| (V(e.x, e.y) - c).norm())
            .collect();
        let mean = gate_dirs.iter().fold(V(0.0, 0.0), |acc, &d| acc + d);
        // Para longe dos portões: onde a costa e o porto ficam.
        let away = if mean.len() > 0.2 {
            mean.norm() * -1.0
        } else {
            gate_dirs.first().copied().unwrap_or(V(1.0, 0.0)).perp()
        };
        match plan.role {
            Role::Serra | Role::Mina | Role::Free(_) => {
                let (port_name, scale): (_, f32) = match plan.role {
                    Role::Serra => (SERRA, 1.0),
                    Role::Mina => (MINA, 1.0),
                    Role::Free(i) => (FREE_PORTS[i].1, 0.7),
                    _ => unreachable!("só portos chegam aqui"),
                };
                let port = c + away * (r * 0.35);
                let side = if rng.coin() {
                    away.perp()
                } else {
                    away.perp() * -1.0
                };
                let frame = |a: f32, b: f32| port + away * a + side * b;
                for (a, b, radius) in COAST {
                    let at = frame(
                        (a + rng.range(0.0, 30.0)) * scale.max(0.85),
                        (b + rng.range(-25.0, 25.0)) * scale,
                    );
                    land.push(disc(at, radius * rng.range(0.9, 1.0) * scale));
                }
                // Recurso nunca nas águas protegidas (o spawn): quem quer
                // material sai do porto. Os nós da Serra e da Mina moram na
                // Rota da Costa; os do porto livre, logo além do cais.
                if matches!(plan.role, Role::Free(_)) {
                    for (k, (a, b)) in CAPITAL_NODES.into_iter().enumerate() {
                        let name = if k % 2 == 0 { FREE_TIMBER } else { FREE_ORE };
                        let out = port + (frame(a, b) - port).norm() * (PORT_WATERS + 70.0);
                        nodes.push(node(name, port_name, settle(&land, out, 25.0), 40));
                    }
                }
                let dock = port - away * 40.0;
                docks.push((plan.role, dock));
                regions.push(region(
                    port_name,
                    Port {
                        name: port_name,
                        x: port.0,
                        y: port.1,
                        service_radius: 60.0,
                    },
                ));
                zones.push(zone(
                    port_waters_name(port_name),
                    RiskTier::Protected,
                    port.0,
                    port.1,
                    PORT_WATERS,
                ));
                for exit in exits.iter().filter(|e| e.from == index) {
                    keep[index].lanes.push((dock, V(exit.x, exit.y)));
                }
                keep[index].points.push((port, PORT_WATERS + 60.0));
                if matches!(plan.role, Role::Serra | Role::Mina) {
                    // A marinha patrulha a boca que dá para a Rota e a baía.
                    let toward_road = mouth(index, road);
                    navy_spawns.push(toward_road + (c - toward_road).norm() * 150.0);
                    navy_spawns.push(settle(&land, c - away * (r * 0.3), 40.0));
                } else {
                    tempest_sites.push(settle(&land, c - away * (r * 0.45), 60.0));
                    navy_spawns.push(settle(&land, c - away * (r * 0.2), 40.0));
                }
            }
            Role::Road => {
                let serra_mouth = mouth(index, index_of(Role::Serra));
                let mina_mouth = mouth(index, index_of(Role::Mina));
                let lane = (mina_mouth - serra_mouth).norm();
                let n = lane.perp();
                // Sem saqueador NPC na Rota: é aqui que o novato coleta pela
                // primeira vez (depósitos das capitais), e a rota é curta
                // demais para um pirata não enxergar um depósito. O risco da
                // Rota é o PvP de fronteira; saqueador NPC mora nos
                // corredores, mais longe das capitais.
                tempest_sites.push(settle(&land, c + n * (0.45 * r), 60.0));
                // Os depósitos das capitais: logo depois da boca de cada
                // baía, dos dois lados da faixa das caravanas.
                for (mouth_at, along, region, deposit, road_node) in [
                    (
                        serra_mouth,
                        lane,
                        SERRA,
                        "Bosque da Serra",
                        "Bosque do Caminho",
                    ),
                    (
                        mina_mouth,
                        lane * -1.0,
                        MINA,
                        "Mina Profunda",
                        "Mina do Caminho",
                    ),
                ] {
                    for (k, (a, b, stock)) in ROAD_DEPOSITS.into_iter().enumerate() {
                        let name = if k == ROAD_DEPOSITS.len() - 1 {
                            road_node
                        } else {
                            deposit
                        };
                        let at = settle(&land, mouth_at + along * a + n * b, 25.0);
                        nodes.push(node(name, region, at, stock));
                    }
                }
                // Fora da faixa das caravanas: coleta no meio do caminho.
                nodes.push(node(DRIFTWOOD, SERRA, c - n * (0.5 * r), 30));
                nodes.push(node(
                    SUNKEN_ORE,
                    MINA,
                    c + n * (0.55 * r) + lane * 120.0,
                    30,
                ));
            }
            Role::Dawn | Role::Dusk => {
                tempest_sites.push(settle(&land, c + away * (0.3 * r), 60.0));
                // Corredor tem dono: saqueadores e o que vale a travessia.
                let side = away.perp();
                raider_spawns.push(settle(&land, c + side * (0.35 * r), 40.0));
                raider_spawns.push(settle(&land, c - side * (0.3 * r) - away * 80.0, 40.0));
                nodes.push(node(DRIFTWOOD, SERRA, c - away * (0.35 * r), 30));
                nodes.push(node(
                    SUNKEN_ORE,
                    MINA,
                    c + side * (-0.55 * r) + away * 150.0,
                    30,
                ));
            }
            Role::Coral => {
                let black = index_of(Role::Black);
                let o = (mouth(index, black) - c).norm();
                let p = V(o.1, -o.0);
                let island = c + o * (r * 0.35);
                let isl = |x: f32, y: f32| island + p * x + o * y;
                for (x, y, radius) in ISLAND_BODY {
                    land.push(disc(isl(x, y), radius));
                }
                let pirate_port = isl(10.0, 133.0);
                regions.push(region(
                    ISLAND,
                    Port {
                        name: PIRATE_PORT,
                        x: pirate_port.0,
                        y: pirate_port.1,
                        service_radius: 60.0,
                    },
                ));
                zones.push(zone(
                    "Águas da Ilha do Coral Negro",
                    RiskTier::Lawless,
                    island.0,
                    island.1,
                    350.0,
                ));
                for (x, y) in [(0.0, -90.0), (-125.0, 5.0), (135.0, 55.0)] {
                    nodes.push(node("Recife do Coral", ISLAND, isl(x, y), 30));
                }
                for (x, y) in [(0.0, -145.0), (-190.0, -85.0), (190.0, 5.0)] {
                    pirate_spawns.push(settle(&land, isl(x, y), 30.0));
                }
                for (x, y) in [(0.0, 355.0), (-500.0, 155.0), (500.0, 155.0)] {
                    tide_sites.push(settle(&land, isl(x, y), 60.0));
                }
                for side in [-1.0, 1.0] {
                    kraken_sites.push(settle(
                        &land,
                        c + p * (side * 0.6 * r) - o * (0.1 * r),
                        60.0,
                    ));
                }
                // O comboio cruza a metade longe da ilha, de bordo a bordo.
                let base = c - o * (r * 0.5);
                let half = (r * r - (r * 0.5).powi(2)).sqrt() - 260.0;
                fleet_route = [-1.0, -0.5, 0.0, 0.5, 1.0]
                    .into_iter()
                    .map(|t| base + p * (t * half))
                    .collect();
                for leg in fleet_route.windows(2) {
                    keep[index].lanes.push((leg[0], leg[1]));
                }
                keep[index].points.push((island, 300.0));
                labels.push(("Ilha do Coral Negro", isl(0.0, -45.0).0, isl(0.0, -45.0).1));
            }
            Role::Black => {
                let o = away;
                let p = V(o.1, -o.0);
                let bw = |x: f32, y: f32| c + p * x + o * y;
                for (x, y, radius) in REEFS {
                    let at = bw(x + rng.range(-20.0, 20.0), y + rng.range(-20.0, 20.0));
                    land.push(disc(at, radius));
                }
                for (x, y) in [(-140.0, -130.0), (120.0, 10.0), (-110.0, 250.0)] {
                    nodes.push(node("Recife Abissal", ISLAND, bw(x, y), 12));
                }
                kraken_sites.push(settle(&land, bw(0.0, -500.0), 60.0));
                // Águas Negras não são vazias: piratas rondam os recifes.
                for (x, y) in [(-420.0, -250.0), (400.0, 300.0)] {
                    pirate_spawns.push(settle(&land, bw(x, y), 40.0));
                }
                keep[index].points.push((c, 560.0));
                labels.push(("ÁGUAS NEGRAS", bw(0.0, 650.0).0, bw(0.0, 650.0).1));
            }
        }
        zones.push(zone(
            plan.role.name(),
            plan.role.tier(),
            c.0,
            c.1,
            r + AREA_WALL_BAND,
        ));
    }

    // Caravanas: da doca da Serra à doca da Mina pela Rota da Costa.
    let serra = index_of(Role::Serra);
    let mina = index_of(Role::Mina);
    let dock_of = |role: Role| docks.iter().find(|(r, _)| *r == role).map(|(_, d)| *d);
    let serra_dock = dock_of(Role::Serra).expect("Serra tem doca");
    let mina_dock = dock_of(Role::Mina).expect("Mina tem doca");
    // Portão leva num sentido só: ida e volta têm cada uma os seus.
    let lane = |from_dock: V, first: ZoneExit, second: ZoneExit, to_dock: V| {
        vec![
            from_dock,
            V(first.x, first.y),
            V(first.dest.0, first.dest.1),
            V(second.x, second.y),
            V(second.dest.0, second.dest.1),
            to_dock,
        ]
    };
    let caravan_route = lane(
        serra_dock,
        exit_between(serra, road),
        exit_between(road, mina),
        mina_dock,
    );
    let caravan_return = lane(
        mina_dock,
        exit_between(mina, road),
        exit_between(road, serra),
        serra_dock,
    );

    // Pontos de conteúdo que o espalhamento precisa respeitar.
    for (index, plan) in plans.iter().enumerate() {
        let near = |p: &V| (*p - plan.center).len() < plan.radius + AREA_WALL_BAND;
        let points = tempest_sites
            .iter()
            .chain(&kraken_sites)
            .chain(&tide_sites)
            .filter(|p| near(p))
            .map(|&p| (p, 90.0))
            .chain(
                pirate_spawns
                    .iter()
                    .chain(&raider_spawns)
                    .chain(&navy_spawns)
                    .filter(|p| near(p))
                    .map(|&p| (p, 60.0)),
            )
            .chain(
                nodes
                    .iter()
                    .map(|spot| V(spot.x, spot.y))
                    .filter(|p| near(p))
                    .map(|p| (p, 70.0)),
            )
            .collect::<Vec<_>>();
        keep[index].points.extend(points);
    }

    // 4. Espalhamento por zona: rochedos, ilhotas; depois as ilhas ocultas.
    for (index, plan) in plans.iter().enumerate() {
        let (rocks_wanted, islets_wanted) = match plan.role {
            Role::Serra | Role::Mina => (3, 0),
            Role::Road => (6, 1),
            Role::Free(_) => (5, 1),
            Role::Dawn | Role::Dusk => (6, 1),
            Role::Coral => (6, 1),
            Role::Black => (6, 1),
        };
        let mut rocks = 0;
        for _ in 0..400 {
            if rocks == rocks_wanted {
                break;
            }
            let at = plan.center + V::polar(rng.range(0.0, TAU), rng.range(150.0, plan.radius));
            let radius = rng.range(18.0, 38.0);
            if plan.inside(at, radius, 120.0)
                && keep[index].allows(at, radius, 70.0)
                && clear_of_land(&land, at, radius, 60.0)
            {
                land.push(disc(at, radius));
                rocks += 1;
            }
        }
        let mut islets = 0;
        for _ in 0..400 {
            if islets == islets_wanted {
                break;
            }
            let at = plan.center + V::polar(rng.range(0.0, TAU), rng.range(200.0, plan.radius));
            let radius = rng.range(50.0, 110.0);
            let lobes: Vec<(V, f32)> = (0..1 + rng.next() % 3)
                .map(|_| {
                    let lobe_r = radius * rng.range(0.4, 0.75);
                    let off = V::polar(rng.range(0.0, TAU), radius * rng.range(0.6, 0.95));
                    (at + off, lobe_r)
                })
                .collect();
            let span = lobes
                .iter()
                .map(|&(lobe, lobe_r)| (lobe - at).len() + lobe_r)
                .fold(radius, f32::max);
            if plan.inside(at, span, 150.0)
                && keep[index].allows(at, span, 140.0)
                && clear_of_land(&land, at, span, 140.0)
            {
                land.push(disc(at, radius));
                land.extend(lobes.into_iter().map(|(lobe, r)| disc(lobe, r)));
                islets += 1;
            }
        }
    }
    let risky: Vec<usize> = (0..plans.len())
        .filter(|&i| plans[i].role.tier() != RiskTier::Protected)
        .collect();
    let mut hidden_islands: Vec<HiddenIsland> = Vec::new();
    for attempt in 0..4000 {
        if hidden_islands.len() == HIDDEN_NAMES.len() {
            break;
        }
        // Uma por zona enquanto der; depois qualquer zona de risco.
        let index = if attempt < 2000 {
            risky[(hidden_islands.len() * 3 + attempt / 500) % risky.len()]
        } else {
            risky[rng.pick(risky.len())]
        };
        let plan = &plans[index];
        let at = plan.center + V::polar(rng.range(0.0, TAU), rng.range(200.0, plan.radius));
        let radius = rng.range(45.0, 60.0);
        let spaced = hidden_islands
            .iter()
            .all(|h| (at - V(h.x, h.y)).len() > 400.0);
        if !(spaced
            && plan.inside(at, radius, 200.0)
            && keep[index].allows(at, radius, 120.0)
            && clear_of_land(&land, at, radius, 160.0))
        {
            continue;
        }
        let dig = at + (plan.center - at).norm() * (radius + 15.0);
        hidden_islands.push(HiddenIsland {
            id: hidden_islands.len() as u32 + 1,
            name: HIDDEN_NAMES[hidden_islands.len()],
            x: at.0,
            y: at.1,
            radius,
            dig_x: dig.0,
            dig_y: dig.1,
        });
    }

    zones.extend(instance_zones());
    land.extend(instance_land());
    nodes.extend(instance_nodes());
    labels.push(maelstrom_label());

    let areas: Vec<Area> = plans
        .iter()
        .map(|plan| Area {
            name: plan.role.name(),
            tier: plan.role.tier(),
            x: plan.center.0,
            y: plan.center.1,
            radius: plan.radius,
            cell: plan.cell,
        })
        .collect();
    let box_of = |role: Role| {
        let plan = &plans[index_of(role)];
        sector(plan.center, plan.radius * 0.5)
    };
    let features = Features {
        seed,
        spawn: serra_dock.t(),
        sea_sectors: risky
            .iter()
            .map(|&i| sector(plans[i].center, plans[i].radius * 0.55))
            .collect(),
        areas,
        exits,
        hidden_islands,
        tempest_sites: tempest_sites.into_iter().map(V::t).collect(),
        kraken_sites: kraken_sites.into_iter().map(V::t).collect(),
        tide_sites: tide_sites.into_iter().map(V::t).collect(),
        fleet_route: fleet_route.into_iter().map(V::t).collect(),
        nodes,
        pirate_spawns: pirate_spawns.into_iter().map(V::t).collect(),
        raider_spawns: raider_spawns.into_iter().map(V::t).collect(),
        navy_spawns: navy_spawns.into_iter().map(V::t).collect(),
        caravan_route: caravan_route.into_iter().map(V::t).collect(),
        caravan_return: caravan_return.into_iter().map(V::t).collect(),
        whirlpool_sectors: [box_of(Role::Dawn), box_of(Role::Dusk), box_of(Role::Black)],
        labels,
    };
    WorldMap::assemble(zones, regions, land, features)
}

/// "Águas do Porto X": o client reconhece o porto pelo nome das águas.
fn port_waters_name(port: &'static str) -> &'static str {
    match port {
        SERRA => "Águas do Porto da Serra",
        MINA => "Águas do Porto da Mina",
        "Porto das Gaivotas" => "Águas do Porto das Gaivotas",
        "Porto do Farol" => "Águas do Porto do Farol",
        "Porto da Areia Branca" => "Águas do Porto da Areia Branca",
        "Porto Santa Luzia" => "Águas do Porto Santa Luzia",
        _ => "Águas do Porto",
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::events::SeaEventKind;

    const SEEDS: std::ops::Range<u64> = 1..300;

    fn tier(map: &WorldMap, (x, y): (f32, f32)) -> RiskTier {
        map.zone_at(x, y)
            .expect("conteúdo fica em mar declarado")
            .tier
    }

    #[test]
    fn same_seed_same_world_and_seeds_differ() {
        // Zonas ganham ids aleatórios; o mundo é a terra, as regiões e o
        // conteúdo.
        let same = |a: &WorldMap, b: &WorldMap| {
            a.land() == b.land() && a.regions() == b.regions() && a.features() == b.features()
        };
        assert!(same(&generate(7), &generate(7)));
        assert_ne!(generate(7).land(), generate(8).land());
        assert!(same(&WorldMap::from_seed(0), &WorldMap::vertical_slice()));
    }

    #[test]
    fn free_port_names_have_their_waters() {
        for (_, port) in FREE_PORTS {
            assert_eq!(port_waters_name(port), format!("Águas do {port}"));
        }
    }

    #[test]
    fn graph_is_connected_and_gates_come_in_pairs() {
        for seed in SEEDS {
            let map = generate(seed);
            let f = map.features();
            assert!((8..=9).contains(&f.areas.len()), "seed {seed}");
            for exit in &f.exits {
                assert!(
                    f.exits
                        .iter()
                        .any(|back| back.from == exit.to && back.to == exit.from),
                    "seed {seed}: portão sem volta"
                );
                // Quem chega não cai de volta no portão de retorno.
                assert!(
                    map.exit_at(exit.dest.0, exit.dest.1).is_none(),
                    "seed {seed}"
                );
                assert_eq!(map.area_at(exit.dest.0, exit.dest.1), Some(exit.to));
                let blocker = map
                    .land()
                    .iter()
                    .find(|m| m.contains(exit.dest.0, exit.dest.1, 25.0));
                assert!(
                    blocker.is_none(),
                    "seed {seed}: chegada em {} bloqueada por {blocker:?}",
                    f.areas[exit.to].name
                );
            }
            // De qualquer porto dá para chegar a qualquer outro.
            let ports: Vec<(f32, f32)> = map
                .regions()
                .iter()
                .filter_map(|r| r.port.as_ref().map(|p| (p.x, p.y)))
                .collect();
            for &from in &ports {
                for &to in &ports {
                    assert!(map.next_hop(from, to).is_some(), "seed {seed}");
                }
            }
        }
    }

    #[test]
    fn walls_seal_every_zone_except_through_the_gates() {
        for seed in SEEDS {
            let map = generate(seed);
            let f = map.features();
            for (index, area) in f.areas.iter().enumerate() {
                let ring = area.radius + WALL_RADIUS;
                for k in 0..720 {
                    let a = k as f32 / 720.0 * TAU;
                    let (x, y) = (area.x + a.cos() * ring, area.y + a.sin() * ring);
                    let gated = map.exit_at(x, y).is_some_and(|e| e.from == index);
                    assert!(
                        gated || map.is_land(x, y),
                        "seed {seed}: brecha no paredão de {} em {k}",
                        area.name
                    );
                }
            }
        }
    }

    #[test]
    fn ports_are_open_water_and_spawn_docks_at_serra() {
        for seed in SEEDS {
            let map = generate(seed);
            for region in map.regions() {
                let port = region.port.as_ref().unwrap();
                assert!(
                    map.push_out_of_land(port.x, port.y, 20.0).is_none(),
                    "seed {seed}: {} sem água no cais",
                    port.name
                );
            }
            let spawn = map.features().spawn;
            assert_eq!(tier(&map, spawn), RiskTier::Protected, "seed {seed}");
            assert!(map.push_out_of_land(spawn.0, spawn.1, 20.0).is_none());
            let serra = map.region_by_name(SERRA).unwrap().port.as_ref().unwrap();
            assert!(serra.contains(spawn.0, spawn.1), "seed {seed}");
            let pirate = map.region_by_name(ISLAND).unwrap().port.as_ref().unwrap();
            assert_eq!(tier(&map, (pirate.x, pirate.y)), RiskTier::Lawless);
            assert!(map.regions().len() >= 4, "seed {seed}: sem porto livre");
        }
    }

    /// Trecho de rota dentro de uma zona (portão -> chegada é teleporte).
    fn same_area(map: &WorldMap, a: (f32, f32), b: (f32, f32)) -> bool {
        map.area_at(a.0, a.1) == map.area_at(b.0, b.1) && map.exit_at(a.0, a.1).is_none()
    }

    #[test]
    fn routes_never_scrape_land_even_with_hidden_islands() {
        for seed in SEEDS {
            let map = generate(seed).with_hidden_islands();
            let f = map.features();
            for (route, clearance) in [
                (&f.fleet_route, 40.0),
                (&f.caravan_route, 20.0),
                (&f.caravan_return, 20.0),
            ] {
                for leg in route.windows(2) {
                    let ((ax, ay), (bx, by)) = (leg[0], leg[1]);
                    if !same_area(&map, leg[0], leg[1]) {
                        continue;
                    }
                    for step in 0..=20 {
                        let t = step as f32 / 20.0;
                        let (x, y) = (ax + (bx - ax) * t, ay + (by - ay) * t);
                        if map.exit_at(x, y).is_some() {
                            break;
                        }
                        assert!(
                            map.push_out_of_land(x, y, clearance).is_none(),
                            "seed {seed}: rota raspa terra em {x},{y}"
                        );
                    }
                }
            }
        }
    }

    #[test]
    fn caravan_route_crosses_zones_through_real_gates() {
        for seed in SEEDS {
            let map = generate(seed);
            let f = map.features();
            assert_eq!(f.caravan_route.first(), f.caravan_return.last());
            assert_eq!(f.caravan_route.last(), f.caravan_return.first());
            for leg in f
                .caravan_route
                .windows(2)
                .chain(f.caravan_return.windows(2))
            {
                if map.area_at(leg[0].0, leg[0].1) != map.area_at(leg[1].0, leg[1].1) {
                    let exit = map
                        .exit_at(leg[0].0, leg[0].1)
                        .expect("salto começa num portão");
                    assert_eq!(exit.dest, leg[1], "seed {seed}");
                }
            }
        }
    }

    #[test]
    fn content_points_are_open_water_in_the_right_waters() {
        for seed in SEEDS {
            let map = generate(seed).with_hidden_islands();
            let f = map.features();
            for kind in SeaEventKind::ALL {
                assert!(!f.event_sites(kind).is_empty(), "seed {seed} {kind:?}");
                for &at in f.event_sites(kind) {
                    assert_ne!(tier(&map, at), RiskTier::Protected, "seed {seed} {kind:?}");
                    assert!(!map.is_land(at.0, at.1), "seed {seed} {kind:?} em terra");
                }
            }
            for &at in &f.pirate_spawns {
                assert_eq!(tier(&map, at), RiskTier::Lawless, "seed {seed}");
                assert!(!map.is_land(at.0, at.1), "seed {seed}");
            }
            for &at in f.raider_spawns.iter().chain(&f.navy_spawns) {
                assert!(
                    map.push_out_of_land(at.0, at.1, 20.0).is_none(),
                    "seed {seed}"
                );
            }
            for spot in &f.nodes {
                assert!(
                    map.push_out_of_land(spot.x, spot.y, 10.0).is_none(),
                    "seed {seed}: nó {} em terra",
                    spot.name
                );
                assert!(map.region_by_name(spot.region).is_ok());
            }
            for &(x0, x1, y0, y1) in f.sea_sectors.iter().chain(&f.whirlpool_sectors) {
                let center = ((x0 + x1) / 2.0, (y0 + y1) / 2.0);
                assert_ne!(tier(&map, center), RiskTier::Protected, "seed {seed}");
            }
        }
    }

    /// Spawn sem recurso: quem fica no porto não farma até ficar forte.
    #[test]
    fn no_resource_node_in_protected_waters() {
        for seed in SEEDS {
            let map = generate(seed);
            let nodes = &map.features().nodes;
            for spot in nodes {
                assert_ne!(
                    tier(&map, (spot.x, spot.y)),
                    RiskTier::Protected,
                    "seed {seed}: {} em água protegida",
                    spot.name
                );
            }
            // O material das capitais só mudou de lugar: o estoque é o mesmo
            // de antes (4 nós de 60 + o "do Caminho").
            let stock = |region: &str| {
                nodes
                    .iter()
                    .filter(|spot| spot.region == region)
                    .filter(|spot| spot.name != DRIFTWOOD && spot.name != SUNKEN_ORE)
                    .map(|spot| spot.max_stock)
                    .sum::<u32>()
            };
            assert_eq!(stock(SERRA), 300, "seed {seed}");
            assert_eq!(stock(MINA), 300, "seed {seed}");
        }
    }

    /// A primeira coleta fora do porto não pode ser emboscada: nenhum
    /// saqueador nasce a ponto de enxergar um depósito das capitais.
    #[test]
    fn raiders_spawn_out_of_sight_of_capital_deposits() {
        // Detecção do pirata é 380 m (server/src/npc.rs); folga por cima.
        const CLEAR: f32 = 400.0;
        let capital = [
            "Bosque da Serra",
            "Mina Profunda",
            "Bosque do Caminho",
            "Mina do Caminho",
        ];
        for seed in SEEDS {
            let map = generate(seed);
            let f = map.features();
            for spot in f.nodes.iter().filter(|n| capital.contains(&n.name)) {
                for raider in &f.raider_spawns {
                    let gap = (raider.0 - spot.x).hypot(raider.1 - spot.y);
                    assert!(
                        gap >= CLEAR,
                        "seed {seed}: saqueador a {gap:.0} m de {}",
                        spot.name
                    );
                }
            }
        }
    }

    #[test]
    fn hidden_islands_are_risky_and_off_the_charts() {
        for seed in SEEDS {
            let map = generate(seed);
            let f = map.features();
            assert_eq!(f.hidden_islands.len(), 4, "seed {seed}");
            for island in &f.hidden_islands {
                assert_ne!(tier(&map, (island.x, island.y)), RiskTier::Protected);
                assert!(
                    !map.is_land(island.x, island.y),
                    "carta mostra {}",
                    island.name
                );
                assert!(!map.is_land(island.dig_x, island.dig_y));
                assert!(!island.land().contains(island.dig_x, island.dig_y, 0.0));
            }
        }
    }

    #[test]
    fn zones_stay_clear_of_instances() {
        for seed in SEEDS {
            for area in &generate(seed).features().areas {
                assert!(area.y - area.radius > 8000.0, "seed {seed}: {}", area.name);
            }
        }
    }

    #[test]
    fn next_hop_walks_the_graph_toward_the_port() {
        let map = generate(3);
        let serra = map.region_by_name(SERRA).unwrap().port.clone().unwrap();
        let pirate = map.region_by_name(ISLAND).unwrap().port.clone().unwrap();
        let mut at = (serra.x, serra.y);
        for _ in 0..10 {
            let hop = map.next_hop(at, (pirate.x, pirate.y)).unwrap();
            if hop == (pirate.x, pirate.y) {
                return;
            }
            at = map.exit_at(hop.0, hop.1).expect("salto é um portão").dest;
        }
        panic!("não chegou ao porto pirata em 10 saltos");
    }
}
