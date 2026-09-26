//! Loadout do navio (MF-039): o que está INSTALADO nos slots. Cada slot
//! guarda uma custódia inteira (instância + localização `Equipped`), então
//! equipar/desequipar move a MESMA instância — nada é destruído num swap —
//! e o full loot enxerga o equipamento instalado sem bridges.

use std::collections::HashMap;

use marvyr_domain_items::{Custody, ItemDefinition, ItemLocation};
use marvyr_shared::ids::{ItemDefinitionId, ShipInstanceId};
use thiserror::Error;

use crate::components::{EquippedComponent, EquippedComponents};
use crate::definition::ShipDefinition;

/// Um equipamento equipado por slot. Um item por slot (a definição do casco
/// determina QUAIS slots existem; múltiplos slots do mesmo kind são decisão
/// de conteúdo futura, não de tipo).
#[derive(Debug, Clone, Default, PartialEq)]
pub struct ShipLoadout {
    slots: HashMap<crate::EquipmentSlot, Custody>,
}

impl ShipLoadout {
    pub fn new() -> Self {
        Self::default()
    }

    /// Instala a custódia no slot; devolve o que estava lá (swap NUNCA
    /// destrói: o chamador realoca o antigo de volta ao storage).
    pub fn equip(
        &mut self,
        ship: ShipInstanceId,
        custody: Custody,
        slot: crate::EquipmentSlot,
    ) -> Option<Custody> {
        let relocated = Custody {
            instance: custody.instance,
            location: ItemLocation::Equipped { ship, slot },
        };
        self.slots.insert(slot, relocated)
    }

    /// Remove o slot; devolve a custódia ao chamador (vai de volta ao
    /// storage — a instância continua viva).
    pub fn unequip(&mut self, slot: crate::EquipmentSlot) -> Option<Custody> {
        self.slots.remove(&slot)
    }

    /// Peça instalada para encaixar/tirar gema (a custódia não muda de lugar).
    pub fn get_mut(&mut self, slot: crate::EquipmentSlot) -> Option<&mut Custody> {
        self.slots.get_mut(&slot)
    }

    pub fn get(&self, slot: crate::EquipmentSlot) -> Option<&Custody> {
        self.slots.get(&slot)
    }

    /// Todas as custódias instaladas (full loot e persistência leem daqui).
    pub fn items(&self) -> impl Iterator<Item = &Custody> {
        self.slots.values()
    }

    /// Visão para `compute_ship_stats`: definição por slot.
    pub fn components(&self) -> EquippedComponents {
        let mut equipped = EquippedComponents::default();
        for (slot, custody) in &self.slots {
            let component = EquippedComponent {
                slot: *slot,
                item_definition: custody.instance.definition,
                affixes: custody.instance.stat_mods(),
            };
            match slot {
                crate::EquipmentSlot::Hull => equipped.hull.push(component),
                crate::EquipmentSlot::Sail => equipped.sail.push(component),
                crate::EquipmentSlot::Weapon => equipped.weapon.push(component),
                crate::EquipmentSlot::Aux => equipped.aux.push(component),
            }
        }
        equipped
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Error)]
pub enum LoadoutError {
    #[error("item {0:?} não é equipamento (recurso não equipa)")]
    NotEquipment(ItemDefinitionId),
    #[error("item {0:?} desconhecido no catálogo")]
    UnknownItem(ItemDefinitionId),
    #[error("o casco não aceita equipamento no slot {slot:?}")]
    SlotNotAccepted { slot: crate::EquipmentSlot },
}

/// Validação fail-closed do equipar (MF-038/039): o item precisa ser
/// equipamento conhecido, e o casco precisa TER o slot que o item ocupa.
/// Puro — o servidor executa o swap só depois desta aprovação.
pub fn can_equip(
    ship: &ShipDefinition,
    item: &ItemDefinition,
) -> Result<crate::EquipmentSlot, LoadoutError> {
    let equipment = item
        .equipment
        .as_ref()
        .ok_or(LoadoutError::NotEquipment(item.id))?;
    if !ship.slots.iter().any(|spec| spec.kind == equipment.slot) {
        return Err(LoadoutError::SlotNotAccepted {
            slot: equipment.slot,
        });
    }
    Ok(equipment.slot)
}

#[cfg(test)]
mod tests {
    use marvyr_domain_items::{
        Custody, EquipmentDefinition, EquipmentSlot, EquipmentStats, ItemDefinition, ItemInstance,
        ItemKind, ItemLocation,
    };
    use marvyr_shared::ids::{
        ItemDefinitionId, ItemInstanceId, RegionId, ShipDefinitionId, ShipInstanceId,
    };

    use super::{can_equip, ShipLoadout};
    use crate::definition::ShipDefinition;

    fn ship_def(slots: &[EquipmentSlot]) -> ShipDefinition {
        ShipDefinition {
            id: ShipDefinitionId::new(),
            kind: crate::ShipKind::SmallMerchant,
            display_name: String::new(),
            slots: slots
                .iter()
                .map(|kind| crate::SlotSpec {
                    kind: *kind,
                    accepts_tag: None,
                })
                .collect(),
            cargo_capacity: 100,
            base_speed: 30.0,
            base_turn_rate: 1.0,
            base_hp: 100,
            base_weapon_damage: 20,
            base_weapon_range: 50.0,
        }
    }

    fn equipment_item(slot: EquipmentSlot) -> ItemDefinition {
        ItemDefinition {
            id: ItemDefinitionId::new(),
            kind: ItemKind::Equipment,
            equipment: Some(EquipmentDefinition {
                slot,
                stats: EquipmentStats::default(),
            }),
            max_stack: 1,
            base_weight: 5,
            tags: Default::default(),
            display_name: String::from("equipamento de teste"),
        }
    }

    fn custody_of(definition: ItemDefinition) -> Custody {
        Custody {
            instance: ItemInstance::new_equipment(ItemInstanceId::new(), definition.id, 100),
            location: ItemLocation::PortStorage(RegionId::new()),
        }
    }

    /// Matriz fail-closed do MF-038/039.
    #[test]
    fn equip_validation_fails_closed() {
        let merchant = ship_def(&[EquipmentSlot::Hull, EquipmentSlot::Sail]);

        // Resource não equipa.
        let wood = ItemDefinition {
            id: ItemDefinitionId::new(),
            kind: ItemKind::Resource,
            equipment: None,
            max_stack: 100,
            base_weight: 2,
            tags: Default::default(),
            display_name: String::from("Madeira"),
        };
        assert_eq!(
            can_equip(&merchant, &wood),
            Err(super::LoadoutError::NotEquipment(wood.id))
        );

        // Item desconhecido não tem slot válido (Equipment sem definição).
        // O catálogo barra; na borda do domínio, None ⇒ NotEquipment.
        let mut ghost = equipment_item(EquipmentSlot::Weapon);
        ghost.equipment = None;
        assert!(matches!(
            can_equip(&merchant, &ghost),
            Err(super::LoadoutError::NotEquipment(_))
        ));

        // Hull em navio sem slot Hull = erro.
        let no_hull = ship_def(&[EquipmentSlot::Sail, EquipmentSlot::Weapon]);
        let hull = equipment_item(EquipmentSlot::Hull);
        assert_eq!(
            can_equip(&no_hull, &hull),
            Err(super::LoadoutError::SlotNotAccepted {
                slot: EquipmentSlot::Hull
            })
        );

        // Caminho feliz: devolve o slot do equipamento.
        let sail = equipment_item(EquipmentSlot::Sail);
        assert_eq!(can_equip(&merchant, &sail), Ok(EquipmentSlot::Sail));
    }

    #[test]
    fn swap_keeps_the_old_instance_alive() {
        let ship = ShipInstanceId::new();
        let mut loadout = ShipLoadout::new();
        let old = equipment_item(EquipmentSlot::Sail);
        let new = equipment_item(EquipmentSlot::Sail);
        let old_id = old.id;
        let new_id = new.id;

        let displaced = loadout.equip(ship, custody_of(old), EquipmentSlot::Sail);
        assert!(displaced.is_none(), "slot vazio: nada desalojado");

        // Swap: o antigo SAI inteiro (o chamador o devolve ao storage).
        let displaced = loadout
            .equip(ship, custody_of(new), EquipmentSlot::Sail)
            .expect("antigo sai");
        assert_eq!(displaced.instance.definition, old_id);
        assert_eq!(
            displaced.location,
            ItemLocation::Equipped {
                ship,
                slot: EquipmentSlot::Sail
            },
            "localização acompanha o contêiner até o chamador realocar"
        );

        // O novo está instalado; a instância do antigo continua viva para
        // voltar ao storage — nada foi destruído no swap.
        assert_eq!(
            loadout
                .get(EquipmentSlot::Sail)
                .unwrap()
                .instance
                .definition,
            new_id
        );
        let removed = loadout.unequip(EquipmentSlot::Sail).expect("desequipou");
        assert_eq!(removed.instance.definition, new_id);
        assert!(loadout.unequip(EquipmentSlot::Sail).is_none());
    }

    #[test]
    fn components_feed_stats_with_one_entry_per_slot() {
        let ship = ShipInstanceId::new();
        let mut loadout = ShipLoadout::new();
        loadout.equip(
            ship,
            custody_of(equipment_item(EquipmentSlot::Sail)),
            EquipmentSlot::Sail,
        );
        loadout.equip(
            ship,
            custody_of(equipment_item(EquipmentSlot::Weapon)),
            EquipmentSlot::Weapon,
        );

        let components = loadout.components();
        assert_eq!(components.sail.len(), 1);
        assert_eq!(components.weapon.len(), 1);
        assert!(components.hull.is_empty());
    }
}
