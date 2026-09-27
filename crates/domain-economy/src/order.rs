use chrono::{DateTime, Utc};
use marvyr_shared::ids::{CharacterId, ItemDefinitionId, MarketOrderId, RegionId};
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum OrderStatus {
    #[default]
    Open,
    Filled,
    Cancelled,
    Expired,
}

/// Oferta de escambo: "dou `quantity` de `item` por `ask_quantity` de
/// `ask_item`". Tudo ou nada: quem aceita entrega o pedido inteiro.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct MarketOrder {
    pub id: MarketOrderId,
    pub seller: CharacterId,
    pub item: ItemDefinitionId,
    pub quantity: u32,
    pub ask_item: ItemDefinitionId,
    pub ask_quantity: u32,
    pub region: RegionId,
    pub status: OrderStatus,
    pub created_at: DateTime<Utc>,
    pub expires_at: DateTime<Utc>,
}

impl Default for MarketOrder {
    fn default() -> Self {
        Self {
            id: MarketOrderId::new(),
            seller: CharacterId::new(),
            item: ItemDefinitionId::new(),
            quantity: 0,
            ask_item: ItemDefinitionId::new(),
            ask_quantity: 0,
            region: RegionId::new(),
            status: OrderStatus::Open,
            created_at: Utc::now(),
            expires_at: Utc::now(),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn market_order_defaults_to_open() {
        assert_eq!(MarketOrder::default().status, OrderStatus::Open);
    }
}
