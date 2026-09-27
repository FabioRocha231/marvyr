//! Quadro de Contratos por porto: gerador puro e determinístico (seed) e o
//! progresso de um contrato ativo. A recompensa é recurso bruto — o que o
//! porto que paga tem de sobra (pilar 1) — e os itens entregues são
//! consumidos (sink).

use marvyr_shared::ItemInstanceId;

use crate::guild::{guild_value, paid_in, EXCHANGE_SPREAD, GUILD_BASE_VALUES};

pub const OFFERS_PER_PORT: usize = 3;
pub const DELIVERY_DURATION_SECS: f64 = 15.0 * 60.0;
pub const HUNT_DURATION_SECS: f64 = 10.0 * 60.0;
/// Valor (na tabela da guilda) de cada abate de uma Caçada.
pub const HUNT_VALUE_PER_KILL: u64 = 150;
/// Valor-alvo de um lote de entrega: define N pela base do item.
const DELIVERY_LOT_VALUE: u64 = 250;
/// Valor de bônus por unidade de distância entre os portos.
const DISTANCE_BONUS_PER_UNIT: f64 = 0.1;
/// A Entrega paga o que a guilda do destino pagaria pela carga (com a
/// margem dela) vezes este prêmio, mais a distância. Sem a margem, a volta
/// contrato → guilda rendia ~4,5x o que saiu (fabricava recurso).
const DELIVERY_PREMIUM: f64 = 2.0;

#[derive(Debug, Clone, PartialEq)]
pub enum ContractKind {
    /// Leve `quantity` x `item` de `from` para `to` (no porão, não storage).
    Delivery {
        item: String,
        quantity: u32,
        from: String,
        to: String,
    },
    /// Afunde `kills` navios NPC hostis dentro da zona `zone` (MV-067).
    Hunt { kills: u32, zone: String },
}

#[derive(Debug, Clone, PartialEq)]
pub struct Contract {
    pub id: u32,
    pub kind: ContractKind,
    /// Recurso pago e quantidade (no armazém do porto que paga: o do quadro
    /// na Caçada, o de destino na Entrega).
    pub reward_item: &'static str,
    pub reward_quantity: u32,
    pub duration_secs: f64,
}

impl Contract {
    /// Texto de exibição (ASCII fora dos nomes de item/porto).
    pub fn title(&self) -> String {
        match &self.kind {
            ContractKind::Delivery {
                item,
                quantity,
                from,
                to,
            } => format!("Entrega: {quantity}x {item} de {from} para {to}"),
            ContractKind::Hunt { kills, zone } => {
                format!("Caçada: afunde {kills} navio(s) hostil(is) em {zone}")
            }
        }
    }

    pub fn target(&self) -> u32 {
        match self.kind {
            ContractKind::Delivery { quantity, .. } => quantity,
            ContractKind::Hunt { kills, .. } => kills,
        }
    }
}

/// Zona com NPC hostil onde uma Caçada pode mandar o capitão. `lawless`
/// paga mais: é onde ele pode perder o navio para outro jogador.
#[derive(Debug, Clone, Copy)]
pub struct HuntingGround<'a> {
    pub name: &'a str,
    pub lawless: bool,
}

/// Bônus da Caçada em zona sem lei (×1.6 por abate).
const LAWLESS_HUNT_BONUS: f64 = 1.6;

/// Um porto para o gerador: nome e posição (bônus de distância).
#[derive(Debug, Clone, Copy)]
pub struct PortSite<'a> {
    pub name: &'a str,
    pub x: f32,
    pub y: f32,
}

/// xorshift64 — determinístico, sem dependência.
fn next(state: &mut u64) -> u64 {
    *state ^= *state << 13;
    *state ^= *state >> 7;
    *state ^= *state << 17;
    *state
}

/// Gera as ofertas do porto `here`. Mesmo seed → mesmas ofertas (testes).
/// `first_id` é o primeiro id livre; ids crescem de 1 em 1. A primeira
/// oferta é sempre Caçada (quando há onde caçar): todo quadro tem luta.
pub fn generate_offers(
    here: &PortSite,
    ports: &[PortSite],
    grounds: &[HuntingGround],
    seed: u64,
    first_id: u32,
) -> Vec<Contract> {
    let mut state = seed | 1;
    let destinations: Vec<&PortSite> = ports.iter().filter(|port| port.name != here.name).collect();
    // ponytail: sem destino nem zona de caça o porto fica sem quadro.
    (0..OFFERS_PER_PORT)
        .filter_map(|index| {
            let id = first_id + index as u32;
            let roll = next(&mut state);
            let wants_hunt = index == 0 || destinations.is_empty() || roll.is_multiple_of(3);
            if wants_hunt && !grounds.is_empty() {
                let ground = grounds[(next(&mut state) % grounds.len() as u64) as usize];
                let kills = 2 + (next(&mut state) % 3) as u32;
                let per_kill = if ground.lawless {
                    HUNT_VALUE_PER_KILL as f64 * LAWLESS_HUNT_BONUS
                } else {
                    HUNT_VALUE_PER_KILL as f64
                };
                let (reward_item, reward_quantity) =
                    paid_in(here.name, per_kill * f64::from(kills));
                return Some(Contract {
                    id,
                    kind: ContractKind::Hunt {
                        kills,
                        zone: ground.name.to_owned(),
                    },
                    reward_item,
                    reward_quantity,
                    duration_secs: HUNT_DURATION_SECS,
                });
            }
            if destinations.is_empty() {
                return None;
            }
            let to = destinations[(next(&mut state) % destinations.len() as u64) as usize];
            let (item, base) =
                GUILD_BASE_VALUES[(next(&mut state) % GUILD_BASE_VALUES.len() as u64) as usize];
            let quantity = (DELIVERY_LOT_VALUE / base).clamp(1, 30) as u32;
            let value = guild_value(to.name, item).expect("item vem da tabela da guilda");
            let distance = ((to.x - here.x).hypot(to.y - here.y)) as f64;
            let (reward_item, reward_quantity) = paid_in(
                to.name,
                value * f64::from(quantity) * EXCHANGE_SPREAD * DELIVERY_PREMIUM
                    + distance * DISTANCE_BONUS_PER_UNIT,
            );
            Some(Contract {
                id,
                kind: ContractKind::Delivery {
                    item: item.to_owned(),
                    quantity,
                    from: here.name.to_owned(),
                    to: to.name.to_owned(),
                },
                reward_item,
                reward_quantity,
                duration_secs: DELIVERY_DURATION_SECS,
            })
        })
        .collect()
}

/// Contrato aceito por um jogador (1 por vez).
#[derive(Debug, Clone, PartialEq)]
pub struct ActiveContract {
    pub contract: Contract,
    pub deadline_secs: f64,
    /// Abates contados (Caça).
    pub kills: u32,
    /// Entrega: pilhas do item que estavam no porão no aceite, com a
    /// quantidade de então. Só elas contam na chegada — comprar ou coletar
    /// no destino não cumpre o contrato.
    pub consignment: Vec<(ItemInstanceId, u32)>,
}

impl ActiveContract {
    /// `hold` = pilhas do item da entrega no porão agora. Entrega sem a
    /// carga completa a bordo é recusada (`None`); Caça ignora o porão.
    pub fn accept(
        contract: Contract,
        now_secs: f64,
        hold: &[(ItemInstanceId, u32)],
    ) -> Option<Self> {
        if let ContractKind::Delivery { quantity, .. } = contract.kind {
            if hold.iter().map(|(_, qty)| qty).sum::<u32>() < quantity {
                return None;
            }
        }
        let consignment = match contract.kind {
            ContractKind::Delivery { .. } => hold.to_vec(),
            ContractKind::Hunt { .. } => Vec::new(),
        };
        Some(Self {
            deadline_secs: now_secs + contract.duration_secs,
            contract,
            kills: 0,
            consignment,
        })
    }

    pub fn remaining_secs(&self, now_secs: f64) -> f64 {
        (self.deadline_secs - now_secs).max(0.0)
    }

    pub fn expired(&self, now_secs: f64) -> bool {
        now_secs >= self.deadline_secs
    }

    /// Conta um abate feito na zona `sunk_in`; só vale o da zona da Caçada.
    /// `true` quando a Caçada fica completa.
    pub fn record_kill(&mut self, sunk_in: Option<&str>) -> bool {
        match &self.contract.kind {
            ContractKind::Hunt { kills, zone } if sunk_in == Some(zone.as_str()) => {
                self.kills += 1;
                self.kills >= *kills
            }
            _ => false,
        }
    }

    /// Consignação só encolhe: pilha que desceu (depositada, vendida,
    /// saqueada) não volta a contar se for reabastecida depois — senão dava
    /// para zarpar com 1 e completar no destino numa pilha de mesmo id.
    pub fn observe_hold(&mut self, hold: &[(ItemInstanceId, u32)]) {
        for (id, counted) in &mut self.consignment {
            let now = hold
                .iter()
                .find(|(held, _)| held == id)
                .map_or(0, |(_, qty)| *qty);
            *counted = (*counted).min(now);
        }
    }

    /// Entrega pronta: atracado no destino com ≥N da carga consignada no
    /// porão. Pilha que cresceu depois do aceite conta só até o que tinha.
    pub fn delivery_ready(&self, docked_port: &str, hold: &[(ItemInstanceId, u32)]) -> bool {
        let ContractKind::Delivery { quantity, to, .. } = &self.contract.kind else {
            return false;
        };
        let carried: u32 = self
            .consignment
            .iter()
            .map(|(id, at_accept)| {
                hold.iter()
                    .find(|(held, _)| held == id)
                    .map_or(0, |(_, now)| (*now).min(*at_accept))
            })
            .sum();
        docked_port == to && carried >= *quantity
    }

    pub fn progress(&self) -> u32 {
        match self.contract.kind {
            ContractKind::Hunt { .. } => self.kills,
            ContractKind::Delivery { .. } => 0,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn ports() -> Vec<PortSite<'static>> {
        vec![
            PortSite {
                name: "Porto da Serra",
                x: -600.0,
                y: 0.0,
            },
            PortSite {
                name: "Porto da Mina",
                x: 600.0,
                y: 0.0,
            },
        ]
    }

    const GROUNDS: [HuntingGround<'static>; 2] = [
        HuntingGround {
            name: "Corredor do Alvorecer",
            lawless: false,
        },
        HuntingGround {
            name: "Mar Negro",
            lawless: true,
        },
    ];

    #[test]
    fn every_board_leads_with_a_zone_hunt_paying_more_where_lawless() {
        let ports = ports();
        for seed in 0..50 {
            let offers = generate_offers(&ports[0], &ports, &GROUNDS, seed, 0);
            let ContractKind::Hunt { kills, zone } = &offers[0].kind else {
                panic!("primeira oferta deveria ser Caçada");
            };
            // Serra paga em Madeira (6 cada): 150 por abate = 25; sem lei, 40.
            assert_eq!(offers[0].reward_item, "Madeira");
            let per_kill = offers[0].reward_quantity / *kills;
            let expected = if zone == "Mar Negro" { 40 } else { 25 };
            assert_eq!(per_kill, expected, "{zone}");
        }
        // Sem zona de caça, o quadro é só de Entregas.
        let offers = generate_offers(&ports[0], &ports, &[], 3, 0);
        assert!(offers
            .iter()
            .all(|c| matches!(c.kind, ContractKind::Delivery { .. })));
    }

    #[test]
    fn generator_is_deterministic_and_well_formed() {
        let ports = ports();
        let a = generate_offers(&ports[0], &ports, &GROUNDS, 42, 10);
        let b = generate_offers(&ports[0], &ports, &GROUNDS, 42, 10);
        assert_eq!(a, b);
        assert_eq!(a.len(), OFFERS_PER_PORT);
        assert_eq!(a.iter().map(|c| c.id).collect::<Vec<_>>(), vec![10, 11, 12]);
        for contract in &a {
            assert!(contract.reward_quantity > 0);
            if let ContractKind::Delivery { from, to, .. } = &contract.kind {
                assert_eq!(from, "Porto da Serra");
                assert_eq!(to, "Porto da Mina");
            }
        }
        // Seeds diferentes variam o quadro (em algum ponto de 20 tentativas).
        assert!((0..20).any(|seed| generate_offers(&ports[0], &ports, &GROUNDS, seed, 10) != a));
    }

    #[test]
    fn delivery_reward_uses_destination_value_and_distance() {
        let ports = ports();
        let delivery = (0..200)
            .flat_map(|seed| generate_offers(&ports[0], &ports, &GROUNDS, seed, 0))
            .find(|c| matches!(&c.kind, ContractKind::Delivery { item, .. } if item == "Madeira"))
            .expect("alguma seed gera entrega de Madeira");
        // 25 Madeira × 16 (Mina paga 1.6x) × 0,35 (margem) × 2 + 1200 × 0.1
        // = 400 de valor, pago no minério da Mina (8,4 cada) = 48.
        assert_eq!(delivery.target(), 25);
        assert_eq!(delivery.reward_item, "Minério");
        assert_eq!(delivery.reward_quantity, 48);
    }

    #[test]
    fn single_port_only_offers_hunts() {
        let ports = ports();
        let offers = generate_offers(&ports[0], &ports[..1], &GROUNDS, 7, 0);
        assert!(offers
            .iter()
            .all(|c| matches!(c.kind, ContractKind::Hunt { .. })));
    }

    #[test]
    fn active_contract_expires_counts_kills_and_checks_delivery() {
        let hunt = Contract {
            id: 1,
            kind: ContractKind::Hunt {
                kills: 2,
                zone: String::from("Mar Negro"),
            },
            reward_item: "Madeira",
            reward_quantity: 50,
            duration_secs: 60.0,
        };
        let mut active = ActiveContract::accept(hunt, 100.0, &[]).unwrap();
        assert_eq!(active.remaining_secs(130.0), 30.0);
        assert!(!active.expired(159.0));
        assert!(active.expired(160.0));
        // Abate fora da zona da Caçada não conta.
        assert!(!active.record_kill(Some("Baía da Serra")));
        assert!(!active.record_kill(None));
        assert_eq!(active.progress(), 0);
        assert!(!active.record_kill(Some("Mar Negro")));
        assert!(active.record_kill(Some("Mar Negro")));

        let (a, b, local) = (
            ItemInstanceId::new(),
            ItemInstanceId::new(),
            ItemInstanceId::new(),
        );
        let madeira = Contract {
            id: 2,
            kind: ContractKind::Delivery {
                item: String::from("Madeira"),
                quantity: 10,
                from: String::from("Porto da Serra"),
                to: String::from("Porto da Mina"),
            },
            reward_item: "Minério",
            reward_quantity: 10,
            duration_secs: 60.0,
        };
        // Sem a carga a bordo não dá para aceitar.
        assert!(ActiveContract::accept(madeira.clone(), 0.0, &[(a, 9)]).is_none());
        let delivery = ActiveContract::accept(madeira, 0.0, &[(a, 6), (b, 4)]).unwrap();
        assert!(delivery.delivery_ready("Porto da Mina", &[(a, 6), (b, 4)]));
        assert!(!delivery.delivery_ready("Porto da Mina", &[(a, 6), (b, 3)]));
        assert!(!delivery.delivery_ready("Porto da Serra", &[(a, 6), (b, 4)]));
        // Comprado/coletado no destino não conta; pilha que cresceu, só até o aceite.
        assert!(!delivery.delivery_ready("Porto da Mina", &[(a, 6), (local, 50)]));
        assert!(!delivery.delivery_ready("Porto da Mina", &[(a, 50), (b, 3)]));
        // Zarpou com a pilha reduzida e reabasteceu no destino: não conta.
        let mut delivery = delivery;
        delivery.observe_hold(&[(a, 1), (b, 4)]);
        assert!(!delivery.delivery_ready("Porto da Mina", &[(a, 6), (b, 4)]));
    }
}
