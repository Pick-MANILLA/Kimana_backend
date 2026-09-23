//! On-chain settlement through the SettlementVault (Pick-MANILLA/kimana_contract).
//!
//! - `units`: helpers that must match the contract's libraries exactly.
//! - `client`: operator calls (`lock_quote`, `settle`, `cancel_quote`, `refund`).
//! - `error`: vault reverts decoded and mapped to `ApiError`.

pub mod bindings;
pub mod client;
pub mod error;
pub mod units;

pub use client::{LockQuote, SettlementClient, SettlementConfig};
pub use error::SettlementError;
