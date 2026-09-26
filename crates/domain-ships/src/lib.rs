//! domain-ships: tipos puros de navio, stats derivados e movimento naval.

pub mod components;
pub mod cosmetics;
pub mod crew;
pub mod definition;
pub mod loadout;
pub mod motion;
pub mod presence;
pub mod sailing;
pub mod stats;
pub mod talents;
pub mod weather;

pub use components::{EquippedComponent, EquippedComponents};
pub use cosmetics::{
    cosmetic_by_code, cosmetic_code, Cosmetic, CosmeticError, CosmeticSlot, ShipCosmetics,
    COSMETICS,
};
pub use crew::{
    casualties, crew_capacity, reload_multiplier, repair_step, rudder_turn_multiplier, RepairStep,
    CREW_WAGE, CREW_WAGE_ITEM, REPAIR_COMBAT_LOCK_SECS, RUDDER_HP_MAX, SKELETON_CREW,
};
pub use definition::{ShipDefinition, ShipKind, SlotSpec};
pub use loadout::{can_equip, LoadoutError, ShipLoadout};
pub use marvyr_domain_items::EquipmentSlot;
pub use motion::{step_motion, MotionInput, MotionTuning, ShipMotion};
pub use presence::{dock, undock, DockError, DockPolicy, VesselPresence};
pub use sailing::{sail_speed_multiplier, Wind, SAIL_HP_MAX};
pub use stats::{compute_ship_stats, rescale_hp, ShipStats, StatsError};
pub use weather::{Storm, Weather, WeatherBounds};
