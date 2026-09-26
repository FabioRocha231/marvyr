//! domain-items: tipos puros de item. Sem persistência, sem ECS.

pub mod affix;
pub mod cargo;
pub mod catalog;
pub mod definition;
pub mod instance;
pub mod location;
pub mod stack;
pub mod storage;

pub use affix::{roll_quality, Affix, AffixKind, AffixTotals, Quality, Rarity};
pub use cargo::{CargoError, CargoHold};
pub use catalog::{CatalogError, ItemCatalog};
pub use definition::{
    EquipmentDefinition, EquipmentSlot, EquipmentStats, ItemDefinition, ItemKind, Tag,
};
pub use instance::ItemInstance;
pub use location::{Custody, ItemLocation};
pub use stack::{remaining_capacity, split, try_merge, SplitError};
pub use storage::{put_stack, quantity_of, take_stacks};
