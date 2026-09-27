//! Guilda Mercante por porto: troca recurso por recurso. Pilar 1 intacto: o
//! que ela recebe é destruído (sink) e o que ela paga é sempre recurso bruto
//! — o que aquele porto tem de sobra. Sem moeda: o "valor" da tabela é só a
//! taxa de câmbio (base × multiplicador regional, com saturação).

use std::collections::HashMap;

/// Valor base por nome de exibição do item. Item fora da tabela NÃO é
/// comprável (fail-closed). Novo item = uma linha aqui.
pub const GUILD_BASE_VALUES: &[(&str, u64)] = &[
    ("Madeira", 10),
    ("Minério", 14),
    ("Coral Negro", 60),
    ("Casco Reforçado", 180),
    ("Velas de Corrida", 160),
    ("Canhão de Bronze", 220),
    // MF-059: raros das zonas de alto risco e o tier 2 da Forja Pirata.
    // Tier 2 vale um pouco mais que os insumos pela guilda.
    ("Pérola Abissal", 90),
    ("Essência da Cerração", 110),
    ("Âmbar Abissal", 100),
    ("Casco Negro", 850),
    ("Velas de Cerração", 1050),
    ("Canhões Abissais", 900),
];

/// Multiplicadores de portos sem tabela própria (ex.: um porto pirata novo).
/// Item ausente = 1.0x.
pub const DEFAULT_REGIONAL_MULTIPLIERS: &[(&str, f64)] = &[("Coral Negro", 1.3)];

/// Multiplicadores por porto: especialidade local paga pouco, a do outro
/// porto paga muito — é daqui que nasce a rota. Item ausente = 1.0x.
pub const PORT_REGIONAL_MULTIPLIERS: &[(&str, &[(&str, f64)])] = &[
    (
        "Porto da Serra",
        &[("Madeira", 0.6), ("Minério", 1.6), ("Coral Negro", 2.0)],
    ),
    (
        "Porto da Mina",
        &[("Madeira", 1.6), ("Minério", 0.6), ("Coral Negro", 2.0)],
    ),
];

/// Com o que cada guilda paga: a especialidade local (barata aqui, cara no
/// outro porto) — levar de um porto para o outro é a rota. Porto sem linha
/// própria paga com madeira.
pub const PORT_PAYOUTS: &[(&str, &str)] =
    &[("Porto da Serra", "Madeira"), ("Porto da Mina", "Minério")];
pub const DEFAULT_PAYOUT: &str = "Madeira";

/// Margem da guilda: ela paga só esta fração do valor entregue. Sem margem,
/// ir e voltar entre Serra e Mina multiplicava recurso do nada (a volta
/// completa rendia ~7×); com 0,35 a melhor volta devolve ~87% — trocar
/// serve para conseguir o que falta, não para fabricar recurso.
pub const EXCHANGE_SPREAD: f64 = 0.35;

/// v49: escassez viva — a guilda que não recebe um item há este tempo o
/// marca "em falta" e quem entrega ganha Renome (nunca recurso a mais: a
/// taxa de câmbio não muda, senão a volta entre portos fabricaria recurso).
pub const SCARCE_AFTER_SECS: f64 = 6.0 * 3_600.0;
/// Renome por unidade entregue em falta, e o teto por entrega.
pub const SCARCE_RENOWN_PER_UNIT: u32 = 2;
pub const SCARCE_RENOWN_MAX: u32 = 80;

/// Meia-vida da saturação: vender muito derruba a taxa, que se recupera.
pub const SATURATION_HALF_LIFE_SECS: f64 = 600.0;
/// Unidades recentes que derrubam a taxa à metade.
pub const SATURATION_CAPACITY: f64 = 50.0;

pub fn base_value(item: &str) -> Option<u64> {
    GUILD_BASE_VALUES
        .iter()
        .find(|(name, _)| *name == item)
        .map(|(_, value)| *value)
}

pub fn regional_multiplier(port: &str, item: &str) -> f64 {
    let table = PORT_REGIONAL_MULTIPLIERS
        .iter()
        .find(|(name, _)| *name == port)
        .map(|(_, table)| *table)
        .unwrap_or(DEFAULT_REGIONAL_MULTIPLIERS);
    table
        .iter()
        .find(|(name, _)| *name == item)
        .map(|(_, multiplier)| *multiplier)
        .unwrap_or(1.0)
}

/// Valor sem saturação (base × regional) — a taxa de câmbio do porto.
pub fn guild_value(port: &str, item: &str) -> Option<f64> {
    base_value(item).map(|base| base as f64 * regional_multiplier(port, item))
}

/// O recurso com que a guilda de `port` paga.
pub fn payout(port: &str) -> &'static str {
    PORT_PAYOUTS
        .iter()
        .find(|(name, _)| *name == port)
        .map(|(_, item)| *item)
        .unwrap_or(DEFAULT_PAYOUT)
}

/// Converte um valor (recompensa de contrato) no recurso principal que o
/// porto paga: (item, quantidade), nunca menos de 1.
pub fn paid_in(port: &str, value: f64) -> (&'static str, u32) {
    let item = payout(port);
    let rate = guild_value(port, item).expect("pagamento vem da tabela da guilda");
    (item, ((value / rate).ceil() as u32).max(1))
}

/// Valor de UMA unidade com `recent_units` já vendidas recentemente.
fn unit_value_at(value: f64, recent_units: f64) -> f64 {
    value / (1.0 + recent_units / SATURATION_CAPACITY)
}

/// Unidades vendidas recentemente numa (porto, item), com instante da
/// última atualização para o decaimento.
#[derive(Debug, Clone, Copy, Default, PartialEq)]
struct Demand {
    units: f64,
    at_secs: f64,
}

impl Demand {
    fn units_at(&self, now_secs: f64) -> f64 {
        let elapsed = (now_secs - self.at_secs).max(0.0);
        self.units * 0.5_f64.powf(elapsed / SATURATION_HALF_LIFE_SECS)
    }
}

/// Livro da guilda: saturação por (porto, item). Tempo em segundos
/// monotônicos do servidor.
#[derive(Debug, Clone, Default)]
pub struct GuildBook {
    demand: HashMap<(String, String), Demand>,
}

impl GuildBook {
    fn recent(&self, port: &str, item: &str, now_secs: f64) -> f64 {
        self.demand
            .get(&(port.to_owned(), item.to_owned()))
            .map(|demand| demand.units_at(now_secs))
            .unwrap_or(0.0)
    }

    /// Quanto do [`payout`] do porto a guilda paga por `quantity` de
    /// `give`: cada unidade entregue satura a seguinte. `None` = a guilda
    /// não aceita `give` aqui (fora da tabela, ou é o que ela mesma paga);
    /// `Some(0)` = é pouco demais para valer uma unidade.
    pub fn exchange_quote(
        &self,
        port: &str,
        give: &str,
        quantity: u32,
        now_secs: f64,
    ) -> Option<u32> {
        let receive = payout(port);
        if give == receive {
            return None;
        }
        let value = guild_value(port, give)?;
        let rate = guild_value(port, receive)?;
        let recent = self.recent(port, give, now_secs);
        let delivered: f64 = (0..quantity)
            .map(|sold| unit_value_at(value, recent + f64::from(sold)))
            .sum();
        Some((delivered * EXCHANGE_SPREAD / rate).floor() as u32)
    }

    /// v49: a guilda de `port` não recebe `item` há [`SCARCE_AFTER_SECS`]
    /// (contando do boot: restart não deixa tudo em falta de uma vez).
    pub fn is_scarce(&self, port: &str, item: &str, now_secs: f64) -> bool {
        if payout(port) == item || guild_value(port, item).is_none() {
            return false;
        }
        let last = self
            .demand
            .get(&(port.to_owned(), item.to_owned()))
            .map_or(0.0, |demand| demand.at_secs);
        now_secs - last >= SCARCE_AFTER_SECS
    }

    pub fn record_sale(&mut self, port: &str, item: &str, quantity: u32, now_secs: f64) {
        let units = self.recent(port, item, now_secs) + f64::from(quantity);
        self.demand.insert(
            (port.to_owned(), item.to_owned()),
            Demand {
                units,
                at_secs: now_secs,
            },
        );
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn unknown_item_is_not_accepted() {
        let book = GuildBook::default();
        assert_eq!(
            book.exchange_quote("Porto da Serra", "Pedra Magica", 3, 0.0),
            None
        );
    }

    #[test]
    fn guild_pays_only_with_the_local_surplus() {
        let book = GuildBook::default();
        assert_eq!(payout("Porto da Serra"), "Madeira");
        assert_eq!(payout("Porto da Mina"), "Minério");
        assert_eq!(payout("Porto Livre"), DEFAULT_PAYOUT);
        // A Serra não troca madeira por madeira.
        assert_eq!(
            book.exchange_quote("Porto da Serra", "Madeira", 10, 0.0),
            None
        );
    }

    #[test]
    fn exchange_gets_what_is_missing_but_never_multiplies() {
        let book = GuildBook::default();
        // Serra: minério vale 22,4, madeira 6 → 10 minério viram madeira.
        let timber = book
            .exchange_quote("Porto da Serra", "Minério", 10, 0.0)
            .unwrap();
        assert!(timber >= 10, "a troca rende: {timber}");
        // Ida e volta em qualquer par de portos nunca devolve mais do que saiu.
        let ports = ["Porto da Serra", "Porto da Mina", "Porto Livre"];
        for a in ports {
            for b in ports {
                for x in ["Minério", "Madeira", "Coral Negro"] {
                    let Some(there) = book.exchange_quote(a, x, 100, 0.0) else {
                        continue;
                    };
                    if payout(b) != x {
                        continue;
                    }
                    let Some(back) = book.exchange_quote(b, payout(a), there, 0.0) else {
                        continue;
                    };
                    assert!(back < 100, "{a}→{b} {x}: 100 virou {back}");
                }
            }
        }
    }

    #[test]
    fn an_item_nobody_brought_for_hours_is_scarce_until_delivered() {
        let mut book = GuildBook::default();
        let port = "Porto da Mina";
        assert!(
            !book.is_scarce(port, "Madeira", 60.0),
            "logo após o boot, nada falta"
        );
        assert!(book.is_scarce(port, "Madeira", SCARCE_AFTER_SECS));
        assert!(
            !book.is_scarce(port, "Minério", SCARCE_AFTER_SECS),
            "o que ela paga não falta"
        );
        book.record_sale(port, "Madeira", 5, SCARCE_AFTER_SECS);
        assert!(!book.is_scarce(port, "Madeira", SCARCE_AFTER_SECS + 60.0));
    }

    #[test]
    fn unknown_port_falls_back_to_default_table() {
        assert_eq!(
            regional_multiplier("Porto do Coral Negro", "Coral Negro"),
            1.3
        );
        assert_eq!(regional_multiplier("Porto do Coral Negro", "Madeira"), 1.0);
    }

    #[test]
    fn selling_saturates_and_recovers_over_time() {
        let mut book = GuildBook::default();
        let fresh = book
            .exchange_quote("Porto da Mina", "Coral Negro", 10, 0.0)
            .unwrap();
        book.record_sale("Porto da Mina", "Coral Negro", 50, 0.0);
        let saturated = book
            .exchange_quote("Porto da Mina", "Coral Negro", 10, 0.0)
            .unwrap();
        assert!(saturated < fresh * 6 / 10, "{saturated} vs {fresh}");
        let later = book
            .exchange_quote(
                "Porto da Mina",
                "Coral Negro",
                10,
                SATURATION_HALF_LIFE_SECS * 10.0,
            )
            .unwrap();
        assert!(later > saturated);
        // Outro porto não é afetado.
        assert_eq!(
            book.exchange_quote("Porto da Serra", "Coral Negro", 10, 0.0),
            GuildBook::default().exchange_quote("Porto da Serra", "Coral Negro", 10, 0.0)
        );
    }

    #[test]
    fn paid_in_uses_the_port_surplus_and_rounds_up() {
        assert_eq!(paid_in("Porto da Serra", 60.0), ("Madeira", 10));
        assert_eq!(paid_in("Porto da Mina", 1.0), ("Minério", 1));
    }
}
