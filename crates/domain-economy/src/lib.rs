//! domain-economy: tipos puros de economia (escambo, guilda, contratos,
//! renome). Sem moeda, sem match engine, sem persistência.

pub mod contract;
pub mod guild;
pub mod logbook;
pub mod market;
pub mod order;
pub mod renown;

pub use contract::{
    generate_offers, ActiveContract, Contract, ContractKind, HuntingGround, PortSite,
};
pub use guild::GuildBook;
pub use market::{validate_new_order, MarketError, ORDER_DURATION_SECS};
pub use order::{MarketOrder, OrderStatus};
