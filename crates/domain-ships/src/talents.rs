//! Rosa dos Ventos (MV-067): a árvore de talentos do capitão. Cada nível de
//! Renome dá um ponto; cada nó dá poucos por cento e os notáveis mudam o
//! jeito de jogar (trocam um atributo por outro). Nenhum atributo de combate
//! passa de +15% com a árvore inteira — quem joga mais anda à frente, não
//! fica invencível. Não se compra: só se ganha jogando.

use crate::stats::ShipStats;

/// Teto de cada atributo de combate somando a árvore toda (%).
pub const MAX_COMBAT_PCT: i32 = 15;
/// Recurso cobrado por ponto devolvido ao redistribuir (só no porto, sai do
/// armazém).
pub const RESPEC_ITEM: &str = "Minério";
pub const RESPEC_PER_POINT: u32 = 3;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Branch {
    Navigation,
    Gunnery,
    Trade,
}

impl Branch {
    pub const ALL: [Branch; 3] = [Branch::Navigation, Branch::Gunnery, Branch::Trade];

    /// Nome PT-BR (o client traduz).
    pub fn name(self) -> &'static str {
        match self {
            Branch::Navigation => "Navegação",
            Branch::Gunnery => "Artilharia",
            Branch::Trade => "Comércio",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Stat {
    Speed,
    Turn,
    Hull,
    Damage,
    Range,
    Cargo,
    /// Unidades a mais por coleta.
    Gather,
}

impl Stat {
    pub fn is_combat(self) -> bool {
        !matches!(self, Stat::Cargo | Stat::Gather)
    }

    /// Nome PT-BR (o client traduz).
    pub fn name(self) -> &'static str {
        match self {
            Stat::Speed => "velocidade",
            Stat::Turn => "giro",
            Stat::Hull => "casco",
            Stat::Damage => "dano",
            Stat::Range => "alcance",
            Stat::Cargo => "porão",
            Stat::Gather => "coleta",
        }
    }
}

#[derive(Debug)]
pub struct TalentNode {
    /// Id estável: é o que se grava no banco.
    pub id: &'static str,
    pub name: &'static str,
    pub branch: Branch,
    /// Posição na tela: linha (0 = raiz) e raia (0..=2) dentro do ramo.
    pub row: u8,
    pub lane: u8,
    pub requires: Option<&'static str>,
    pub notable: bool,
    pub effects: &'static [(Stat, i32)],
}

const fn node(
    id: &'static str,
    name: &'static str,
    branch: Branch,
    (row, lane): (u8, u8),
    requires: Option<&'static str>,
    effects: &'static [(Stat, i32)],
) -> TalentNode {
    TalentNode {
        id,
        name,
        branch,
        row,
        lane,
        requires,
        notable: effects.len() > 1,
        effects,
    }
}

use Branch::{Gunnery as G, Navigation as N, Trade as T};
use Stat::*;

/// A árvore inteira. Notável = nó com troca (mais de um efeito).
pub const TREE: &[TalentNode] = &[
    node("nav.leme", "Mão no Leme", N, (0, 1), None, &[(Turn, 3)]),
    node(
        "nav.pano",
        "Pano Bem Cazado",
        N,
        (1, 0),
        Some("nav.leme"),
        &[(Speed, 2)],
    ),
    node(
        "nav.costado",
        "Costado Firme",
        N,
        (1, 2),
        Some("nav.leme"),
        &[(Hull, 3)],
    ),
    node(
        "nav.rumo",
        "Rumo Certo",
        N,
        (2, 0),
        Some("nav.pano"),
        &[(Turn, 3)],
    ),
    node(
        "nav.bolina",
        "Bolina Cerrada",
        N,
        (2, 1),
        Some("nav.pano"),
        &[(Speed, 3)],
    ),
    node(
        "nav.tabuas",
        "Tábuas Dobradas",
        N,
        (2, 2),
        Some("nav.costado"),
        &[(Hull, 3)],
    ),
    node(
        "nav.timoneiro",
        "Timoneiro Veterano",
        N,
        (3, 0),
        Some("nav.rumo"),
        &[(Turn, 4)],
    ),
    node(
        "nav.popa",
        "Vento de Popa",
        N,
        (3, 1),
        Some("nav.bolina"),
        &[(Speed, 5), (Hull, -5)],
    ),
    node(
        "nav.carvalho",
        "Casco de Carvalho",
        N,
        (3, 2),
        Some("nav.tabuas"),
        &[(Hull, 6), (Speed, -4)],
    ),
    node(
        "nav.contravento",
        "Contravento",
        N,
        (4, 0),
        Some("nav.timoneiro"),
        &[(Turn, 5), (Speed, 2)],
    ),
    node(
        "nav.remendo",
        "Velas Remendadas",
        N,
        (4, 1),
        Some("nav.popa"),
        &[(Speed, 3)],
    ),
    node(
        "art.polvora",
        "Pólvora Seca",
        G,
        (0, 1),
        None,
        &[(Damage, 2)],
    ),
    node(
        "art.olho",
        "Olho de Artilheiro",
        G,
        (1, 0),
        Some("art.polvora"),
        &[(Range, 3)],
    ),
    node(
        "art.bala",
        "Bala Calibrada",
        G,
        (1, 2),
        Some("art.polvora"),
        &[(Damage, 3)],
    ),
    node(
        "art.alca",
        "Alça de Mira",
        G,
        (2, 0),
        Some("art.olho"),
        &[(Range, 3)],
    ),
    node(
        "art.culatra",
        "Culatra Reforçada",
        G,
        (2, 1),
        Some("art.bala"),
        &[(Hull, 3)],
    ),
    node(
        "art.carga",
        "Carga Dupla",
        G,
        (2, 2),
        Some("art.bala"),
        &[(Damage, 3)],
    ),
    node(
        "art.longo",
        "Canhão Longo",
        G,
        (3, 0),
        Some("art.alca"),
        &[(Range, 6), (Damage, -4)],
    ),
    node(
        "art.bordada",
        "Bordada Pesada",
        G,
        (3, 2),
        Some("art.carga"),
        &[(Damage, 5), (Range, -4)],
    ),
    node(
        "art.mira",
        "Mira Firme",
        G,
        (4, 0),
        Some("art.longo"),
        &[(Range, 3)],
    ),
    node(
        "art.fina",
        "Pólvora Fina",
        G,
        (4, 2),
        Some("art.bordada"),
        &[(Damage, 2)],
    ),
    node(
        "com.olho",
        "Olho de Mercador",
        T,
        (0, 1),
        None,
        &[(Gather, 5)],
    ),
    node(
        "com.estiva",
        "Estiva Arrumada",
        T,
        (1, 0),
        Some("com.olho"),
        &[(Cargo, 5)],
    ),
    node(
        "com.machado",
        "Machado Afiado",
        T,
        (1, 2),
        Some("com.olho"),
        &[(Gather, 5)],
    ),
    node(
        "com.calafeto",
        "Porão Calafetado",
        T,
        (2, 0),
        Some("com.estiva"),
        &[(Cargo, 5)],
    ),
    node(
        "com.paciente",
        "Coletor Paciente",
        T,
        (2, 2),
        Some("com.machado"),
        &[(Gather, 5)],
    ),
    node(
        "com.fundo",
        "Porão Fundo",
        T,
        (3, 0),
        Some("com.calafeto"),
        &[(Cargo, 20), (Speed, -5)],
    ),
    node(
        "com.ouro",
        "Mãos de Ouro",
        T,
        (3, 2),
        Some("com.paciente"),
        &[(Gather, 20), (Damage, -5)],
    ),
    node(
        "com.rede",
        "Rede de Estiva",
        T,
        (4, 0),
        Some("com.fundo"),
        &[(Cargo, 5)],
    ),
    node(
        "com.faro",
        "Faro de Coletor",
        T,
        (4, 2),
        Some("com.ouro"),
        &[(Gather, 5)],
    ),
];

pub fn find(id: &str) -> Option<&'static TalentNode> {
    TREE.iter().find(|node| node.id == id)
}

/// Pontos que o nível de Renome dá (o nível 1 não dá ponto).
pub fn points_for_level(level: u32) -> u32 {
    level.saturating_sub(1)
}

/// Quantidade de [`RESPEC_ITEM`] para devolver `allocated` pontos.
pub fn respec_cost(allocated: usize) -> u32 {
    RESPEC_PER_POINT * allocated as u32
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AllocateError {
    Unknown,
    AlreadyTaken,
    /// O nó de cima ainda não foi pego.
    Locked,
    NoPoints,
}

impl AllocateError {
    /// Motivo PT-BR para o toast (o client traduz).
    pub fn reason(self) -> &'static str {
        match self {
            AllocateError::Unknown => "Talento desconhecido.",
            AllocateError::AlreadyTaken => "Talento já aprendido.",
            AllocateError::Locked => "Aprenda o talento anterior do ramo primeiro.",
            AllocateError::NoPoints => "Sem pontos: ganhe Renome para subir de nível.",
        }
    }
}

pub fn can_allocate(allocated: &[String], id: &str, points: u32) -> Result<(), AllocateError> {
    let node = find(id).ok_or(AllocateError::Unknown)?;
    if allocated.iter().any(|taken| taken == id) {
        return Err(AllocateError::AlreadyTaken);
    }
    if let Some(parent) = node.requires {
        if !allocated.iter().any(|taken| taken == parent) {
            return Err(AllocateError::Locked);
        }
    }
    if allocated.len() as u32 >= points {
        return Err(AllocateError::NoPoints);
    }
    Ok(())
}

/// Soma dos efeitos aprendidos, em %.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct TalentBonus {
    pub speed: i32,
    pub turn: i32,
    pub hull: i32,
    pub damage: i32,
    pub range: i32,
    pub cargo: i32,
    pub gather: i32,
}

impl TalentBonus {
    /// Ids desconhecidos (nó removido da árvore) são ignorados.
    pub fn of(allocated: &[String]) -> Self {
        let mut bonus = Self::default();
        for (stat, pct) in allocated
            .iter()
            .filter_map(|id| find(id))
            .flat_map(|node| node.effects)
        {
            *bonus.slot(*stat) += pct;
        }
        for stat in [Speed, Turn, Hull, Damage, Range] {
            let slot = bonus.slot(stat);
            *slot = (*slot).min(MAX_COMBAT_PCT);
        }
        bonus
    }

    fn slot(&mut self, stat: Stat) -> &mut i32 {
        match stat {
            Speed => &mut self.speed,
            Turn => &mut self.turn,
            Hull => &mut self.hull,
            Damage => &mut self.damage,
            Range => &mut self.range,
            Cargo => &mut self.cargo,
            Gather => &mut self.gather,
        }
    }

    pub fn apply(&self, base: &ShipStats) -> ShipStats {
        let scale = |value: f32, pct: i32| value * (100 + pct) as f32 / 100.0;
        let scale_u =
            |value: u32, pct: i32| (i64::from(value) * i64::from(100 + pct) / 100).max(0) as u32;
        ShipStats {
            speed: scale(base.speed, self.speed),
            turn_rate: scale(base.turn_rate, self.turn),
            max_hp: scale_u(base.max_hp, self.hull).max(1),
            cargo_capacity: scale_u(base.cargo_capacity, self.cargo),
            weapon_damage: scale_u(base.weapon_damage, self.damage),
            weapon_range: scale(base.weapon_range, self.range),
            reload_factor: base.reload_factor,
        }
    }

    /// Unidades extras de uma coleta de `taken`.
    pub fn gather_extra(&self, taken: u32) -> u32 {
        taken * self.gather.max(0) as u32 / 100
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn ids(list: &[&str]) -> Vec<String> {
        list.iter().map(|id| id.to_string()).collect()
    }

    #[test]
    fn tree_is_well_formed_and_combat_never_passes_the_cap() {
        for node in TREE {
            assert_eq!(TREE.iter().filter(|other| other.id == node.id).count(), 1);
            if let Some(parent) = node.requires {
                let parent = find(parent).expect("pai existe");
                assert_eq!(parent.branch, node.branch, "{}", node.id);
                assert!(parent.row < node.row, "{}", node.id);
            }
            assert!(node.lane <= 2 && node.row <= 4, "{}", node.id);
        }
        for stat in [Speed, Turn, Hull, Damage, Range] {
            let most: i32 = TREE
                .iter()
                .flat_map(|node| node.effects)
                .filter(|(s, pct)| *s == stat && *pct > 0)
                .map(|(_, pct)| pct)
                .sum();
            assert!(most <= MAX_COMBAT_PCT, "{stat:?} soma {most}");
        }
        assert!(TREE.len() >= 30);
    }

    #[test]
    fn allocation_needs_points_and_the_parent() {
        assert_eq!(
            can_allocate(&[], "nav.leme", 0),
            Err(AllocateError::NoPoints)
        );
        assert_eq!(can_allocate(&[], "nav.pano", 5), Err(AllocateError::Locked));
        assert_eq!(can_allocate(&[], "x", 5), Err(AllocateError::Unknown));
        assert_eq!(can_allocate(&[], "nav.leme", 1), Ok(()));
        let taken = ids(&["nav.leme"]);
        assert_eq!(
            can_allocate(&taken, "nav.leme", 5),
            Err(AllocateError::AlreadyTaken)
        );
        assert_eq!(
            can_allocate(&taken, "nav.pano", 1),
            Err(AllocateError::NoPoints)
        );
        assert_eq!(can_allocate(&taken, "nav.pano", 2), Ok(()));
        assert_eq!(points_for_level(1), 0);
        assert_eq!(points_for_level(4), 3);
        assert_eq!(respec_cost(3), 9);
    }

    #[test]
    fn bonus_scales_stats_and_notables_trade() {
        let base = ShipStats {
            speed: 100.0,
            turn_rate: 1.0,
            max_hp: 100,
            cargo_capacity: 100,
            weapon_damage: 20,
            weapon_range: 200.0,
            reload_factor: 1.0,
        };
        assert_eq!(TalentBonus::default().apply(&base), base);
        let bonus = TalentBonus::of(&ids(&[
            "com.olho",
            "com.estiva",
            "com.calafeto",
            "com.fundo",
            "sumiu.da.arvore",
        ]));
        assert_eq!((bonus.cargo, bonus.speed, bonus.gather), (30, -5, 5));
        let stats = bonus.apply(&base);
        assert_eq!(stats.cargo_capacity, 130);
        assert!((stats.speed - 95.0).abs() < 1e-4);
        assert_eq!(bonus.gather_extra(40), 2);
        // Árvore inteira: nenhum atributo de combate passa do teto.
        let all: Vec<String> = TREE.iter().map(|node| node.id.to_string()).collect();
        let full = TalentBonus::of(&all);
        for pct in [full.speed, full.turn, full.hull, full.damage, full.range] {
            assert!(pct <= MAX_COMBAT_PCT);
        }
    }
}
