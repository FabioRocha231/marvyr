use marvyr_shared::ids::ItemInstanceId;
use thiserror::Error;

use crate::definition::ItemDefinition;
use crate::instance::ItemInstance;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Error)]
pub enum SplitError {
    #[error("cannot split {requested} from stack of {current}")]
    InvalidQuantity { current: u32, requested: u32 },
    #[error("item definition is not stackable")]
    NotStackable,
}

/// Quantos itens ainda cabem em um stack baseado na definição.
pub fn remaining_capacity(stack: &ItemInstance, def: &ItemDefinition) -> u32 {
    if def.is_fungible() {
        def.max_stack.saturating_sub(stack.quantity)
    } else {
        0 // não-fungíveis não acumulam
    }
}

/// Tenta fundir `source` em `target` se forem da mesma definição fungível e houver espaço.
/// Retorna `remaining` (itens de `source` que não couberam no target) e o `target` atualizado.
pub fn try_merge(
    target: &mut ItemInstance,
    source: ItemInstance,
    def: &ItemDefinition,
) -> ItemInstance {
    if !def.is_fungible() || target.definition != source.definition {
        return source; // nada funde; devolve source intacta
    }

    let space = remaining_capacity(target, def);
    let moved = source.quantity.min(space);
    target.quantity += moved;
    let leftover = source.quantity - moved;

    ItemInstance {
        id: source.id,
        definition: source.definition,
        quantity: leftover,
        durability: source.durability,
        quality: source.quality,
    }
}

/// Divide um stack em dois; `split_quantity` deve ser > 0 e < quantity.
pub fn split(
    stack: &ItemInstance,
    new_id: ItemInstanceId,
    split_quantity: u32,
    def: &ItemDefinition,
) -> Result<(ItemInstance, ItemInstance), SplitError> {
    if !def.is_fungible() {
        return Err(SplitError::NotStackable);
    }
    if split_quantity == 0 || split_quantity >= stack.quantity {
        return Err(SplitError::InvalidQuantity {
            current: stack.quantity,
            requested: split_quantity,
        });
    }

    let mut new_stack = stack.clone();
    new_stack.id = new_id;
    new_stack.quantity = split_quantity;

    let mut reduced = stack.clone();
    reduced.quantity -= split_quantity;

    Ok((reduced, new_stack))
}

#[cfg(test)]
mod tests {
    use smallvec::smallvec;

    use super::*;
    use crate::definition::{ItemDefinition, ItemKind};
    use crate::instance::ItemInstance;
    use marvyr_shared::ids::{ItemDefinitionId, ItemInstanceId};

    fn def(kind: ItemKind, max_stack: u32) -> ItemDefinition {
        ItemDefinition {
            id: ItemDefinitionId::new(),
            kind,
            equipment: None,
            max_stack,
            base_weight: 1,
            tags: smallvec![],
            display_name: String::new(),
        }
    }

    fn stack(def_id: ItemDefinitionId, quantity: u32) -> ItemInstance {
        ItemInstance::new_resource(ItemInstanceId::new(), def_id, quantity)
    }

    #[test]
    fn try_merge_merges_fully() {
        let def = def(ItemKind::Resource, 10);
        let mut target = stack(def.id, 4);
        let leftover = try_merge(&mut target, stack(def.id, 3), &def);

        assert_eq!(target.quantity, 7);
        assert_eq!(leftover.quantity, 0);
    }

    #[test]
    fn try_merge_returns_leftover_when_exceeding_max_stack() {
        let def = def(ItemKind::Resource, 10);
        let mut target = stack(def.id, 8);
        let leftover = try_merge(&mut target, stack(def.id, 5), &def);

        assert_eq!(target.quantity, 10);
        assert_eq!(leftover.quantity, 3);
    }

    #[test]
    fn try_merge_non_fungible_returns_source_intact() {
        let def = def(ItemKind::Equipment, 1);
        let mut target = stack(def.id, 1);
        let source = stack(def.id, 1);
        let leftover = try_merge(&mut target, source.clone(), &def);

        assert_eq!(leftover, source);
        assert_eq!(target.quantity, 1);
    }

    #[test]
    fn split_rejects_zero() {
        let def = def(ItemKind::Resource, 10);
        let source = stack(def.id, 5);

        assert_eq!(
            split(&source, ItemInstanceId::new(), 0, &def),
            Err(SplitError::InvalidQuantity {
                current: 5,
                requested: 0,
            })
        );
    }

    #[test]
    fn split_rejects_quantity_at_or_above_current() {
        let def = def(ItemKind::Resource, 10);
        let source = stack(def.id, 5);

        for requested in [5, 6] {
            assert!(matches!(
                split(&source, ItemInstanceId::new(), requested, &def),
                Err(SplitError::InvalidQuantity { .. })
            ));
        }
    }

    #[test]
    fn split_splits_in_middle() {
        let def = def(ItemKind::Resource, 10);
        let source = stack(def.id, 5);
        let new_id = ItemInstanceId::new();

        let (reduced, new_stack) = split(&source, new_id, 2, &def).unwrap();

        assert_eq!(reduced.quantity, 3);
        assert_eq!(new_stack.quantity, 2);
        assert_eq!(new_stack.id, new_id);
    }

    #[test]
    fn split_rejects_non_fungible() {
        let def = def(ItemKind::Quest, 1);
        let source = stack(def.id, 1);

        assert_eq!(
            split(&source, ItemInstanceId::new(), 1, &def),
            Err(SplitError::NotStackable)
        );
    }
}
