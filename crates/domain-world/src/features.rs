//! Conteúdo ancorado na geografia (MV-065): onde nascem jogadores, NPCs,
//! eventos, nós de recurso e ilhas ocultas. Quem monta o mapa (à mão ou pela
//! seed) decide; o servidor só lê daqui — nenhuma coordenada mágica fora do
//! `WorldMap`.

use crate::events::SeaEventKind;
use crate::map::{MAELSTROM_POINTS, MAELSTROM_X};
use crate::risk::RiskTier;
use crate::treasure::HiddenIsland;

/// Uma zona do mundo (MV-066): um mar redondo, cercado de paredão, ligado às
/// vizinhas por portões. Zonas vivem em regiões distantes do mesmo plano —
/// a distância entre elas é o que isola AOI, combate e streaming.
#[derive(Debug, Clone, PartialEq)]
pub struct Area {
    pub name: &'static str,
    pub tier: RiskTier,
    pub x: f32,
    pub y: f32,
    /// Raio navegável (o paredão começa aqui).
    pub radius: f32,
    /// Célula na carta de zonas (coluna, linha).
    pub cell: (i32, i32),
}

/// Portão de saída: quem entra em (x, y, radius) reaparece em `dest`, já
/// dentro da zona `to`, aproado para longe do portão de volta.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct ZoneExit {
    pub x: f32,
    pub y: f32,
    pub radius: f32,
    pub from: usize,
    pub to: usize,
    pub dest: (f32, f32),
}

impl ZoneExit {
    pub fn catches(&self, x: f32, y: f32) -> bool {
        (x - self.x).powi(2) + (y - self.y).powi(2) <= self.radius * self.radius
    }
}

/// Um depósito de recurso: nome do nó, região dona e estoque máximo.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct NodeSpot {
    pub name: &'static str,
    pub region: &'static str,
    pub x: f32,
    pub y: f32,
    pub max_stock: u32,
}

/// Caixa (x0, x1, y0, y1).
pub type Sector = (f32, f32, f32, f32);

#[derive(Debug, Clone, PartialEq)]
pub struct Features {
    /// Seed que gerou o mundo (0 = mapa clássico feito à mão).
    pub seed: u64,
    /// Doca do Porto da Serra: spawn, respawn e navio novo.
    pub spawn: (f32, f32),
    /// Caixas de mar aberto (dentro das zonas): tempestades e cerrações
    /// nascem aqui.
    pub sea_sectors: Vec<Sector>,
    /// Zonas do mundo (o mapa clássico é uma zona só) e seus portões.
    pub areas: Vec<Area>,
    pub exits: Vec<ZoneExit>,
    pub hidden_islands: Vec<HiddenIsland>,
    pub tempest_sites: Vec<(f32, f32)>,
    pub kraken_sites: Vec<(f32, f32)>,
    pub tide_sites: Vec<(f32, f32)>,
    /// Rota do comboio do tesouro (o evento começa no primeiro ponto).
    pub fleet_route: Vec<(f32, f32)>,
    pub nodes: Vec<NodeSpot>,
    /// Corsários em volta da ilha.
    pub pirate_spawns: Vec<(f32, f32)>,
    /// Piratas que rondam a Rota da Costa.
    pub raider_spawns: Vec<(f32, f32)>,
    pub navy_spawns: Vec<(f32, f32)>,
    /// Serra -> Mina.
    pub caravan_route: Vec<(f32, f32)>,
    /// Mina -> Serra. Não é a ida invertida: cada portão só leva num
    /// sentido (MV-066), a volta usa os portões do outro lado.
    pub caravan_return: Vec<(f32, f32)>,
    /// Onde os três redemoinhos nascem (ligam aos `MAELSTROM_POINTS`).
    pub whirlpool_sectors: [Sector; 3],
    /// Nomes de águas perigosas que o client escreve no mar.
    pub labels: Vec<(&'static str, f32, f32)>,
}

impl Features {
    /// Onde o evento pode acontecer: sempre fora de águas protegidas.
    pub fn event_sites(&self, kind: SeaEventKind) -> &[(f32, f32)] {
        match kind {
            SeaEventKind::Tempest => &self.tempest_sites,
            SeaEventKind::TreasureFleet => &self.fleet_route[..1],
            SeaEventKind::Kraken => &self.kraken_sites,
            SeaEventKind::ContestedTide => &self.tide_sites,
            // Mesmo mar fundo e sem lei do Kraken.
            SeaEventKind::BloodTide => &self.kraken_sites,
        }
    }

    /// O conteúdo do mapa clássico (vertical slice), coordenada por
    /// coordenada como era antes do mundo procedural.
    pub fn classic() -> Self {
        let mut nodes: Vec<NodeSpot> = [
            ("Bosque da Serra", "Porto da Serra", -620.0, 175.0, 60),
            ("Bosque da Serra", "Porto da Serra", -500.0, 130.0, 60),
            ("Bosque da Serra", "Porto da Serra", -610.0, -165.0, 60),
            ("Bosque da Serra", "Porto da Serra", -470.0, -70.0, 60),
            ("Bosque do Caminho", "Porto da Serra", -430.0, 20.0, 60),
            ("Mina Profunda", "Porto da Mina", 620.0, 175.0, 60),
            ("Mina Profunda", "Porto da Mina", 500.0, 130.0, 60),
            ("Mina Profunda", "Porto da Mina", 610.0, -165.0, 60),
            ("Mina Profunda", "Porto da Mina", 470.0, -70.0, 60),
            ("Mina do Caminho", "Porto da Mina", 430.0, 20.0, 60),
            ("Recife do Coral", ISLAND, 0.0, 855.0, 30),
            ("Recife do Coral", ISLAND, -125.0, 950.0, 30),
            ("Recife do Coral", ISLAND, 135.0, 1000.0, 30),
            ("Recife Abissal", ISLAND, -140.0, 1620.0, 12),
            ("Recife Abissal", ISLAND, 120.0, 1760.0, 12),
            ("Recife Abissal", ISLAND, -110.0, 2000.0, 12),
        ]
        .into_iter()
        .map(|(name, region, x, y, max_stock)| NodeSpot {
            name,
            region,
            x,
            y,
            max_stock,
        })
        .collect();
        nodes.extend(instance_nodes());
        let fleet_route = vec![
            (-1300.0, -760.0),
            (-650.0, -720.0),
            (0.0, -760.0),
            (650.0, -720.0),
            (1300.0, -760.0),
        ];
        Self {
            seed: 0,
            spawn: (-560.0, 0.0),
            sea_sectors: vec![(-1400.0, 1400.0, -700.0, 1500.0)],
            areas: vec![Area {
                name: "Mar do Triângulo",
                tier: RiskTier::Frontier,
                x: 0.0,
                y: 450.0,
                radius: 2700.0,
                cell: (0, 0),
            }],
            exits: Vec::new(),
            hidden_islands: CLASSIC_HIDDEN_ISLANDS.to_vec(),
            tempest_sites: vec![(0.0, 500.0), (-700.0, 700.0), (700.0, 700.0)],
            kraken_sites: vec![(0.0, 1700.0), (-900.0, 1100.0), (900.0, 1100.0)],
            tide_sites: vec![(0.0, 1300.0), (-500.0, 1100.0), (500.0, 1100.0)],
            fleet_route,
            nodes,
            pirate_spawns: vec![(0.0, 800.0), (-190.0, 860.0), (190.0, 950.0)],
            raider_spawns: vec![(-240.0, 60.0), (180.0, -60.0)],
            navy_spawns: vec![(-430.0, 60.0), (430.0, -60.0)],
            caravan_route: vec![
                (-560.0, 0.0),
                (-300.0, 0.0),
                (0.0, 0.0),
                (300.0, 0.0),
                (560.0, 0.0),
            ],
            caravan_return: vec![
                (560.0, 0.0),
                (300.0, 0.0),
                (0.0, 0.0),
                (-300.0, 0.0),
                (-560.0, 0.0),
            ],
            whirlpool_sectors: [
                (-1000.0, -200.0, -500.0, 800.0),
                (200.0, 1000.0, -500.0, 800.0),
                (-600.0, 600.0, 1300.0, 2150.0),
            ],
            labels: vec![
                ("Ilha do Coral Negro", 0.0, 900.0),
                ("ÁGUAS NEGRAS", 0.0, 1400.0),
                maelstrom_label(),
            ],
        }
    }
}

pub(crate) const ISLAND: &str = "Ilha do Coral Negro";

pub(crate) fn maelstrom_label() -> (&'static str, f32, f32) {
    (
        "PASSAGEM DO SORVEDOURO",
        MAELSTROM_X,
        MAELSTROM_POINTS[0].1 - 180.0,
    )
}

/// Recursos raros das instâncias (Cerração e Sorvedouro): as instâncias são
/// fixas, longe do mundo gerado.
pub(crate) fn instance_nodes() -> Vec<NodeSpot> {
    [
        ("Coração da Cerração", 4000.0, -1250.0),
        ("Coração da Cerração", 4380.0, -1500.0),
        ("Coração da Cerração", 4020.0, 160.0),
        ("Coração da Cerração", 4380.0, -120.0),
        ("Coração da Cerração", 4020.0, 1560.0),
        ("Coração da Cerração", 4050.0, 1200.0),
        ("Veio Abissal", -4280.0, -700.0),
        ("Veio Abissal", -4120.0, -300.0),
        ("Veio Abissal", -4280.0, 300.0),
        ("Veio Abissal", -4120.0, 700.0),
    ]
    .into_iter()
    .map(|(name, x, y)| NodeSpot {
        name,
        region: ISLAND,
        x,
        y,
        max_stock: 10,
    })
    .collect()
}

/// As ilhas ocultas do mapa clássico.
const CLASSIC_HIDDEN_ISLANDS: [HiddenIsland; 4] = [
    HiddenIsland {
        id: 1,
        name: "Ilhota da Caveira",
        x: -600.0,
        y: 1250.0,
        radius: 55.0,
        dig_x: -600.0,
        dig_y: 1180.0,
    },
    HiddenIsland {
        id: 2,
        name: "Recife dos Afogados",
        x: 650.0,
        y: 1300.0,
        radius: 50.0,
        dig_x: 650.0,
        dig_y: 1235.0,
    },
    HiddenIsland {
        id: 3,
        name: "Baixio do Enforcado",
        x: -300.0,
        y: -950.0,
        radius: 45.0,
        dig_x: -300.0,
        dig_y: -885.0,
    },
    HiddenIsland {
        id: 4,
        name: "Atol Sem Nome",
        x: 820.0,
        y: 1450.0,
        radius: 60.0,
        dig_x: 820.0,
        dig_y: 1375.0,
    },
];
