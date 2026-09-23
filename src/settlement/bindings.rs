//! Contract bindings generated from the vendored `abi/SettlementVault.json`.
//!
//! The ABI is a copy of `abi/SettlementVault.json` in Pick-MANILLA/kimana_contract
//! (synced at c8f87be). When the contract's ABI changes, copy the file again and
//! rebuild: the parity tests and the Anvil test catch most drift.

// Generated code: the constructor's `deploy` helper takes the vault's 8 constructor arguments.
#![allow(clippy::too_many_arguments)]

alloy::sol!(
    #[allow(missing_docs)]
    #[sol(rpc)]
    #[derive(Debug)]
    SettlementVault,
    "abi/SettlementVault.json"
);
