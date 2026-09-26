//! Storage regional do porto (MF-037, PRD §30): uma lista de [`Custody`]
//! sem peso máximo e sem dono no tipo (dono e região são a chave do mapa no
//! servidor). Operações de riqueza guardada — a contraparte exata das
//! operações de porão: retirada é fail-closed, tudo-ou-nada.

use crate::cargo::CargoError;
use crate::location::{Custody, ItemLocation};
use marvyr_shared::ids::ItemDefinitionId;

/// Quantidade total de `item` nas pilhas.
pub fn quantity_of(storage: &[Custody], item: ItemDefinitionId) -> u32 {
    storage
        .iter()
        .filter(|custody| custody.instance.definition == item)
        .map(|custody| custody.instance.quantity)
        .sum()
}

/// Retira EXATAMENTE `quantity` unidades de `item`, regravando a localização
/// pelo destino. Insuficiência é erro e NADA é mutado (§37/§69). Agrega
/// através de pilhas: três pilhas de 10 satisfazem uma retirada de 25.
pub fn take_stacks(
    storage: &mut Vec<Custody>,
    item: ItemDefinitionId,
    quantity: u32,
    destination: ItemLocation,
) -> Result<Vec<Custody>, CargoError> {
    let available = quantity_of(storage, item);
    if available < quantity {
        return Err(CargoError::NotEnoughItems {
            requested: quantity,
        });
    }

    let mut remaining = quantity;
    let mut taken = Vec::new();
    let mut index = 0;
    while index < storage.len() && remaining > 0 {
        if storage[index].instance.definition != item {
            index += 1;
            continue;
        }
        let available_in_stack = storage[index].instance.quantity;
        if available_in_stack <= remaining {
            let custody = storage.remove(index);
            remaining -= available_in_stack;
            taken.push(custody.with_location(destination));
        } else {
            storage[index].instance.quantity -= remaining;
            let mut partial = storage[index].clone();
            partial.instance.quantity = remaining;
            remaining = 0;
            taken.push(partial.with_location(destination));
        }
    }
    Ok(taken)
}

/// Guarda uma custódia no storage, agregando na pilha existente quando o
/// item é fungível e cabe no `max_stack` — pilha nova quando não. Retorna
/// a custódia como ficou armazenada.
pub fn put_stack(storage: &mut Vec<Custody>, custody: Custody, max_stack: u32) -> Custody {
    let incoming = custody.instance.quantity;
    let definition = custody.instance.definition;
    // Soma na MESMA pilha que coube: antes conferia uma e somava na primeira
    // do item, que podia estar cheia e passar de `max_stack`.
    if let Some(existing) = storage.iter_mut().find(|existing| {
        existing.instance.definition == definition
            && existing.instance.quantity + incoming <= max_stack
    }) {
        existing.instance.quantity += incoming;
        return existing.clone();
    }
    storage.push(custody.clone());
    custody
}

#[cfg(test)]
mod tests {
    use crate::instance::ItemInstance;
    use crate::location::{Custody, ItemLocation};
    use marvyr_shared::ids::{ItemDefinitionId, ItemInstanceId, RegionId, ShipInstanceId};

    use super::{put_stack, quantity_of, take_stacks};

    fn stack(item: ItemDefinitionId, quantity: u32, region: RegionId) -> Custody {
        Custody {
            instance: ItemInstance::new_resource(ItemInstanceId::new(), item, quantity),
            location: ItemLocation::PortStorage(region),
        }
    }

    #[test]
    fn quantity_sums_across_stacks() {
        let item = ItemDefinitionId::new();
        let region = RegionId::new();
        let storage = vec![stack(item, 10, region), stack(item, 15, region)];
        assert_eq!(quantity_of(&storage, item), 25);
    }

    #[test]
    fn take_aggregates_across_stacks_and_relocates() {
        let item = ItemDefinitionId::new();
        let region = RegionId::new();
        let ship = ItemLocation::ShipCargo(ShipInstanceId::new());
        let mut storage = vec![stack(item, 10, region), stack(item, 15, region)];

        let taken = take_stacks(&mut storage, item, 12, ship).expect("retirou");

        assert_eq!(taken.len(), 2);
        assert_eq!(taken[0].instance.quantity, 10);
        assert_eq!(taken[1].instance.quantity, 2);
        assert!(taken.iter().all(|custody| custody.location == ship));
        assert_eq!(quantity_of(&storage, item), 13);
    }

    #[test]
    fn take_is_atomic_on_shortage() {
        let item = ItemDefinitionId::new();
        let region = RegionId::new();
        let mut storage = vec![stack(item, 10, region)];

        let error = take_stacks(
            &mut storage,
            item,
            11,
            ItemLocation::ShipCargo(ShipInstanceId::new()),
        )
        .unwrap_err();

        assert!(matches!(
            error,
            crate::cargo::CargoError::NotEnoughItems { requested: 11 }
        ));
        // Nada foi mutado no fracasso.
        assert_eq!(quantity_of(&storage, item), 10);
    }

    #[test]
    fn put_merges_into_fitting_stack_else_opens_another() {
        let item = ItemDefinitionId::new();
        let region = RegionId::new();
        let mut storage = vec![stack(item, 8, region)];

        let merged = put_stack(&mut storage, stack(item, 2, region), 10);
        assert_eq!(merged.instance.quantity, 10);
        assert_eq!(storage.len(), 1, "agregou na pilha existente");
        assert_eq!(quantity_of(&storage, item), 10);

        let overflow = put_stack(&mut storage, stack(item, 5, region), 10);
        assert_eq!(overflow.instance.quantity, 5);
        assert_eq!(storage.len(), 2, "estourou max_stack: pilha nova");
    }

    #[test]
    fn put_skips_a_full_stack_listed_first() {
        let item = ItemDefinitionId::new();
        let region = RegionId::new();
        let mut storage = vec![stack(item, 10, region), stack(item, 8, region)];
        let partial_id = storage[1].instance.id;

        let merged = put_stack(&mut storage, stack(item, 2, region), 10);
        assert_eq!(
            merged.instance.id, partial_id,
            "devolve a pilha que recebeu"
        );
        assert_eq!(storage.len(), 2);
        assert!(storage.iter().all(|s| s.instance.quantity <= 10));
        assert_eq!(quantity_of(&storage, item), 20);
    }
}
