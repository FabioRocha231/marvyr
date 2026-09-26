//! domain-items: tipos puros de item. Sem persistência, sem ECS.

pub mod affix;
pub mod cargo;
pub mod catalog;
pub mod definition;
pub mod gem;
pub mod instance;
pub mod location;
pub mod map_mod;
pub mod orb;
pub mod stack;
pub mod storage;
pub mod synergy;

pub use affix::{aura_level, roll_quality, Affix, AffixKind, AffixTotals, Quality, Rarity};
pub use cargo::{CargoError, CargoHold};
pub use catalog::{CatalogError, ItemCatalog};
pub use definition::{
    EquipmentDefinition, EquipmentSlot, EquipmentStats, ItemDefinition, ItemKind, Tag,
};
pub use gem::{socket_count, GemKind, SocketError};
pub use instance::ItemInstance;
pub use location::{Custody, ItemLocation};
pub use map_mod::{roll_map, MapMod};
pub use orb::{OrbError, OrbKind};
pub use stack::{remaining_capacity, split, try_merge, SplitError};
pub use storage::{put_stack, quantity_of, take_stacks};
pub use synergy::{piece_synergies, set_synergies, Synergy};
