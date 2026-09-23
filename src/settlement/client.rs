//! Operator calls into the SettlementVault.
//!
//! Each call sends one transaction and waits for its receipt, so a returned
//! `Ok` means the state change is mined. Retrying a call with the same `ref`
//! is safe: the vault rejects a second success (`RefAlreadyUsed`,
//! `QuoteAlreadyLocked`), which maps to `CONFLICT`.

use super::bindings::{ISettlementVault::QuoteInput, SettlementVault};
use super::error::SettlementError;
use super::units::{receive_amount_minor, RateE8};
use alloy::network::{EthereumWallet, ReceiptResponse};
use alloy::primitives::{Address, FixedBytes, TxHash, B256, U256};
use alloy::providers::{DynProvider, Provider, ProviderBuilder};
use alloy::transports::http::reqwest::Url;
use chrono::Utc;

/// Where the vault lives. Read from `SETTLEMENT_RPC_URL` and
/// `SETTLEMENT_VAULT_ADDRESS`; settlement is off when either is unset. The
/// operator's signer is passed separately: in production it is the custody
/// provider's MPC wallet, never a key in the environment.
#[derive(Debug, Clone)]
pub struct SettlementConfig {
    pub rpc_url: Url,
    pub vault: Address,
}

impl SettlementConfig {
    pub fn from_env() -> Option<Result<Self, String>> {
        let rpc_url = std::env::var("SETTLEMENT_RPC_URL").ok()?;
        let vault = std::env::var("SETTLEMENT_VAULT_ADDRESS").ok()?;
        Some(Self::parse(&rpc_url, &vault))
    }

    pub fn parse(rpc_url: &str, vault: &str) -> Result<Self, String> {
        Ok(SettlementConfig {
            rpc_url: rpc_url
                .parse()
                .map_err(|e| format!("SETTLEMENT_RPC_URL: {e}"))?,
            vault: vault
                .parse()
                .map_err(|e| format!("SETTLEMENT_VAULT_ADDRESS: {e}"))?,
        })
    }
}

/// The terms the customer accepted, in the units `lockQuote` takes. The
/// counterparty amount is derived here with the vault's own formula, so the
/// backend can never lock a quote the vault would recompute differently.
#[derive(Debug, Clone)]
pub struct LockQuote {
    pub quote_id: B256,
    /// ISO 4217 code, e.g. `*b"NGN"`. Must be registered on the vault.
    pub receive_currency: [u8; 3],
    /// Minor-unit exponent of `receive_currency` (NGN = 2). Must match the
    /// vault's registry, or the lock reverts with `ReceiveAmountMismatch`.
    pub receive_decimals: u8,
    /// Unix seconds.
    pub expires_at: u64,
    pub rate: RateE8,
    /// Net USDC base units settled to the partner.
    pub usdc_amount: U256,
    pub fee_usdc: U256,
}

impl LockQuote {
    pub fn receive_amount_minor(&self) -> Result<U256, SettlementError> {
        Ok(receive_amount_minor(
            self.usdc_amount,
            self.rate,
            self.receive_decimals,
        )?)
    }
}

pub struct SettlementClient {
    vault: SettlementVault::SettlementVaultInstance<DynProvider>,
}

impl SettlementClient {
    pub fn new(config: &SettlementConfig, operator: impl Into<EthereumWallet>) -> Self {
        let provider = ProviderBuilder::new()
            .wallet(operator.into())
            .connect_http(config.rpc_url.clone())
            .erased();
        SettlementClient {
            vault: SettlementVault::new(config.vault, provider),
        }
    }

    pub fn vault(&self) -> &SettlementVault::SettlementVaultInstance<DynProvider> {
        &self.vault
    }

    /// Call when the customer confirms, before `expires_at`.
    pub async fn lock_quote(
        &self,
        transfer_ref: B256,
        quote: &LockQuote,
    ) -> Result<TxHash, SettlementError> {
        let now = u64::try_from(Utc::now().timestamp()).unwrap_or(0);
        if quote.expires_at <= now {
            return Err(SettlementError::QuoteExpiredLocally);
        }
        let input = QuoteInput {
            quoteId: quote.quote_id,
            receiveCurrency: FixedBytes(quote.receive_currency),
            expiresAt: quote.expires_at,
            rate: U256::from(quote.rate.get()),
            usdcAmount: quote.usdc_amount,
            feeUsdc: quote.fee_usdc,
            receiveAmountMinor: quote.receive_amount_minor()?,
        };
        let pending = self.vault.lockQuote(transfer_ref, input).send().await?;
        mined(pending.get_receipt().await?)
    }

    /// `amount` must equal the locked quote's `usdc_amount`.
    pub async fn settle(
        &self,
        transfer_ref: B256,
        partner: Address,
        amount: U256,
    ) -> Result<TxHash, SettlementError> {
        let pending = self
            .vault
            .settle(transfer_ref, partner, amount)
            .send()
            .await?;
        mined(pending.get_receipt().await?)
    }

    /// For a transfer that expired or was rejected after its quote was locked.
    pub async fn cancel_quote(&self, transfer_ref: B256) -> Result<TxHash, SettlementError> {
        let pending = self.vault.cancelQuote(transfer_ref).send().await?;
        mined(pending.get_receipt().await?)
    }

    /// After the partner's `returnSettlement`; `to` must be an on-ramp partner.
    pub async fn refund(&self, transfer_ref: B256, to: Address) -> Result<TxHash, SettlementError> {
        let pending = self.vault.refund(transfer_ref, to).send().await?;
        mined(pending.get_receipt().await?)
    }
}

fn mined(receipt: impl ReceiptResponse) -> Result<TxHash, SettlementError> {
    if receipt.status() {
        Ok(receipt.transaction_hash())
    } else {
        Err(SettlementError::Reverted(receipt.transaction_hash()))
    }
}
