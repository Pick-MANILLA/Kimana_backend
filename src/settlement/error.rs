//! Settlement failures, and how each one reaches the API.
//!
//! A revert is decoded against the vault's own error set, so a handler can
//! tell an expired quote (the customer must re-quote) from a paused vault
//! (retry later) from a quote the backend built wrongly (our bug). Decoded
//! errors are logged in full but never returned to the caller (CWE-209).

use super::bindings::SettlementVault::SettlementVaultErrors as VaultError;
use super::units::UnitsError;
use crate::error::{ApiError, ErrorCode};
use alloy::primitives::TxHash;
use thiserror::Error;

#[derive(Debug, Error)]
pub enum SettlementError {
    /// The vault rejected the call with one of its custom errors.
    #[error("vault reverted: {0:?}")]
    Vault(VaultError),
    /// Mined, but reverted without revert data we could read.
    #[error("transaction {0} reverted")]
    Reverted(TxHash),
    /// `lockQuote` would revert with `QuoteExpired`; not sent.
    #[error("quote expired before it could be locked")]
    QuoteExpiredLocally,
    #[error(transparent)]
    Units(#[from] UnitsError),
    /// RPC, transport or ABI failure: nothing is known about the outcome.
    #[error("settlement RPC error: {0}")]
    Rpc(String),
}

impl From<alloy::contract::Error> for SettlementError {
    fn from(err: alloy::contract::Error) -> Self {
        match err.as_decoded_interface_error::<VaultError>() {
            Some(decoded) => SettlementError::Vault(decoded),
            None => SettlementError::Rpc(err.to_string()),
        }
    }
}

impl From<alloy::providers::PendingTransactionError> for SettlementError {
    fn from(err: alloy::providers::PendingTransactionError) -> Self {
        SettlementError::Rpc(err.to_string())
    }
}

impl From<SettlementError> for ApiError {
    fn from(err: SettlementError) -> Self {
        match &err {
            SettlementError::QuoteExpiredLocally => {
                return ApiError::rate_expired("This quote has expired. Request a new quote.")
            }
            SettlementError::Vault(vault) => {
                if let Some(api) = map_vault_error(vault) {
                    tracing::warn!(error = ?vault, "settlement vault rejected the call");
                    return api;
                }
            }
            SettlementError::Rpc(_) => {
                tracing::warn!(error = %err, "settlement RPC failure");
                return ApiError::new(
                    ErrorCode::PartnerFailure,
                    "Settlement is temporarily unavailable. Try again in a moment.",
                );
            }
            SettlementError::Reverted(_) | SettlementError::Units(_) => {}
        }
        // Everything else means the backend sent something the vault's rules
        // forbid: a malformed quote, a wrong key or role, a config gap.
        tracing::error!(error = ?err, "settlement call failed on a backend fault");
        ApiError::server_error()
    }
}

/// `None` for errors that are a backend fault; the caller turns those into
/// a logged `SERVER_ERROR`.
fn map_vault_error(err: &VaultError) -> Option<ApiError> {
    use VaultError as E;
    let api = match err {
        E::QuoteExpired(_) | E::QuoteLockTooOld(_) => {
            ApiError::rate_expired("This quote has expired. Request a new quote.")
        }
        E::QuoteAlreadyUsed(_)
        | E::QuoteAlreadyLocked(_)
        | E::RefAlreadyUsed(_)
        | E::QuoteIsCancelled(_)
        | E::QuoteNotLocked(_)
        | E::InvalidStatus(_)
        | E::NotFunded(_)
        | E::AlreadyFunded(_) => {
            ApiError::conflict("This transfer is not in a state that allows that action.")
        }
        // The quoting provider is far from the independent reference rate.
        // Retrying the same quote cannot succeed; ops is paged by the monitor.
        E::RateDivergenceTooHigh(_) => ApiError::new(
            ErrorCode::PartnerFailure,
            "We can't honour this rate right now. Request a new quote.",
        )
        .retryable(false),
        E::EnforcedPause(_) => ApiError::new(
            ErrorCode::PartnerFailure,
            "Settlement is paused. Try again later.",
        ),
        E::ExceedsDailyLimit(_) | E::ExceedsPerSettlementLimit(_) => ApiError::compliance_hold(
            "This transfer exceeds the current settlement limit and needs review.",
        ),
        _ => return None,
    };
    Some(api)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::settlement::bindings::SettlementVault;
    use alloy::primitives::{B256, U256};

    fn api(err: VaultError) -> ApiError {
        SettlementError::Vault(err).into()
    }

    #[test]
    fn expired_quotes_ask_for_a_new_quote() {
        let err = api(VaultError::QuoteExpired(SettlementVault::QuoteExpired {
            r#ref: B256::ZERO,
            expiresAt: 1,
        }));
        assert_eq!(err.code, ErrorCode::RateExpired);
        assert_eq!(
            ApiError::from(SettlementError::QuoteExpiredLocally).code,
            ErrorCode::RateExpired
        );
    }

    #[test]
    fn reuse_is_a_conflict() {
        let err = api(VaultError::QuoteAlreadyUsed(
            SettlementVault::QuoteAlreadyUsed {
                quoteId: B256::ZERO,
            },
        ));
        assert_eq!(err.code, ErrorCode::Conflict);
    }

    #[test]
    fn divergence_is_not_retryable() {
        let err = api(VaultError::RateDivergenceTooHigh(
            SettlementVault::RateDivergenceTooHigh {
                r#ref: B256::ZERO,
                deviationBps: U256::from(600u64),
                maxBps: U256::from(500u64),
            },
        ));
        assert_eq!(err.code, ErrorCode::PartnerFailure);
        assert!(!err.retryable);
    }

    #[test]
    fn pause_is_retryable() {
        let err = api(VaultError::EnforcedPause(SettlementVault::EnforcedPause {}));
        assert_eq!(err.code, ErrorCode::PartnerFailure);
        assert!(err.retryable);
    }

    #[test]
    fn a_malformed_quote_is_our_bug() {
        let err = api(VaultError::ReceiveAmountMismatch(
            SettlementVault::ReceiveAmountMismatch {
                provided: U256::from(2u64),
                expected: U256::from(1u64),
            },
        ));
        assert_eq!(err.code, ErrorCode::ServerError);
        assert!(!err.message.contains("ReceiveAmountMismatch"));
    }

    #[test]
    fn revert_data_decodes_to_the_vault_error() {
        use alloy::sol_types::SolInterface;
        // `QuoteExpired(bytes32,uint64)`, selector 0xcb089b44 in abi/SettlementVault.errors.json.
        let encoded = VaultError::QuoteExpired(SettlementVault::QuoteExpired {
            r#ref: B256::repeat_byte(0xab),
            expiresAt: 42,
        })
        .abi_encode();
        assert_eq!(&encoded[..4], &[0xcb, 0x08, 0x9b, 0x44]);
        assert!(matches!(
            VaultError::abi_decode(&encoded),
            Ok(VaultError::QuoteExpired(_))
        ));
    }
}
