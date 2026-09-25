//! Regras do escambo entre jogadores. Sem moeda: uma oferta troca um item
//! por outro, tudo ou nada, no porto onde foi listada.

use marvyr_shared::ids::ItemDefinitionId;

/// Janela de vida de uma oferta nova (MF-041).
pub const ORDER_DURATION_SECS: i64 = 300;

/// Erros de mercado — fail-closed (§69: UnknownMarket e afins).
#[derive(Debug, Clone, Copy, PartialEq, Eq, thiserror::Error)]
pub enum MarketError {
    #[error("quantidade precisa ser maior que zero")]
    ZeroQuantity,
    #[error("troque por um item diferente do oferecido")]
    SameItem,
    #[error("faltam itens para a troca: precisa de {needed}, tem {available}")]
    NotEnoughToPay { needed: u32, available: u32 },
    #[error("oferta desconhecida (UnknownOrder)")]
    UnknownOrder,
    #[error("oferta não está aberta (NotOpen)")]
    OrderNotOpen,
    #[error("oferta de outro porto (RegionMismatch) — mercado não é global")]
    RegionMismatch,
    #[error("item não está no armazém deste porto")]
    NotInStorage,
    #[error("armazém vazio ou inexistente neste porto")]
    EmptyStorage,
    #[error("essa oferta não pertence a este personagem")]
    NotOrderOwner,
    #[error("não dá para aceitar a própria oferta")]
    OwnOrder,
}

/// Validação de nova oferta: quantidades positivas e itens diferentes.
pub fn validate_new_order(
    item: ItemDefinitionId,
    quantity: u32,
    ask_item: ItemDefinitionId,
    ask_quantity: u32,
) -> Result<(), MarketError> {
    if quantity == 0 || ask_quantity == 0 {
        return Err(MarketError::ZeroQuantity);
    }
    if item == ask_item {
        return Err(MarketError::SameItem);
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn new_order_needs_positive_quantities_and_two_items() {
        let (a, b) = (ItemDefinitionId::new(), ItemDefinitionId::new());
        assert_eq!(validate_new_order(a, 20, b, 6), Ok(()));
        assert_eq!(
            validate_new_order(a, 0, b, 6),
            Err(MarketError::ZeroQuantity)
        );
        assert_eq!(
            validate_new_order(a, 20, b, 0),
            Err(MarketError::ZeroQuantity)
        );
        assert_eq!(validate_new_order(a, 20, a, 6), Err(MarketError::SameItem));
    }
}
