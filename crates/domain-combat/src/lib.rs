//! domain-combat: primitivas puras de combate naval (PRD §18-§26). Sem
//! persistência, sem ECS, sem Bevy — o servidor conecta as peças.

pub mod ammo;
pub mod black_flag;
pub mod destruction;
pub mod elite;
pub mod flask;
pub mod loot;
pub mod naval;
pub mod projectile;
pub mod weapon;

pub use ammo::{sail_points, Ammo};
pub use black_flag::{BlackFlag, FlagRefusal};
pub use destruction::{apply_damage, DamageOutcome};
pub use flask::{FlaskBelt, FlaskKind, FlaskRefusal};
pub use loot::{
    can_loot, is_expired, resolve_ship_destruction, DestructionOutcome, LootPolicy, SurvivorItem,
    WreckChest, WreckPolicy,
};
pub use naval::{
    aim_at, boarding_chance, hit_zone, nearest_in_range, resolve_boarding, rudder_points,
    BoardingOutcome, HitZone,
};
pub use projectile::{Projectile, WeaponParams};
pub use weapon::{BroadsideBattery, BroadsideSide};
