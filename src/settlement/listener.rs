//! Applies confirmed SettlementVault events to backend transfers.
//!
//! - Only blocks at least `confirmations` deep are read, so an ordinary reorg
//!   never reaches the ledger.
//! - Each log is applied in its own transaction together with its
//!   `settlement_events` row, keyed by `(tx_hash, log_index)`: a replayed log
//!   is skipped, so a crash mid-range and a restart are both safe.
//! - `settlement_cursor` holds the last fully processed block and its hash.
//!   A restart resumes after it. If that block's hash changed, the chain
//!   reorganised deeper than `confirmations`: the listener stops and logs for
//!   ops rather than rewrite an append-only ledger.
//!
//! The ops alerting for the same events lives in kimana_contract's `monitor/`.

use super::bindings::SettlementVault::{self, SettlementVaultEvents as Event};
use crate::domain::transfers::engine::{self, OnchainOutcome, OnchainTarget};
use crate::error::ApiError;
use alloy::eips::BlockNumberOrTag;
use alloy::primitives::{Address, B256};
use alloy::providers::{DynProvider, Provider, ProviderBuilder};
use alloy::rpc::types::{Filter, Log};
use alloy::sol_types::{SolEvent, SolEventInterface};
use serde_json::{json, Value};
use sqlx::PgPool;
use std::time::Duration;
use thiserror::Error;
use uuid::Uuid;

#[derive(Debug, Error)]
pub enum ListenerError {
    #[error("block {block} changed hash since it was processed: reorg deeper than the confirmation depth")]
    ReorgBeyondConfirmations { block: u64 },
    #[error("settlement RPC error: {0}")]
    Rpc(String),
    #[error("block {0} not found")]
    MissingBlock(u64),
    #[error("database error: {0}")]
    Db(#[from] sqlx::Error),
    #[error("transfer update failed: {0}")]
    Api(#[from] ApiError),
}

impl<E: std::fmt::Display> From<alloy::transports::RpcError<E>> for ListenerError {
    fn from(err: alloy::transports::RpcError<E>) -> Self {
        ListenerError::Rpc(err.to_string())
    }
}

#[derive(Debug, Clone)]
pub struct ListenerConfig {
    pub vault: Address,
    pub confirmations: u64,
    pub start_block: u64,
    pub poll: Duration,
    /// Largest block range asked of the node in one `eth_getLogs`. Public
    /// RPCs cap it (sepolia.base.org allows 1,000).
    pub max_range: u64,
}

pub struct SettlementListener {
    pool: PgPool,
    provider: DynProvider,
    config: ListenerConfig,
}

impl SettlementListener {
    pub fn new(
        pool: PgPool,
        rpc_url: alloy::transports::http::reqwest::Url,
        config: ListenerConfig,
    ) -> Self {
        let provider = ProviderBuilder::new().connect_http(rpc_url).erased();
        SettlementListener {
            pool,
            provider,
            config,
        }
    }

    /// Polls until an error that needs a human (a deep reorg). Transient RPC
    /// and database errors are logged and retried on the next poll.
    pub async fn run(self) {
        loop {
            match self.run_once().await {
                Ok(_) => {}
                Err(err @ ListenerError::ReorgBeyondConfirmations { .. }) => {
                    tracing::error!(error = %err, "settlement listener stopped");
                    return;
                }
                Err(err) => tracing::warn!(error = %err, "settlement listener poll failed"),
            }
            tokio::time::sleep(self.config.poll).await;
        }
    }

    /// Processes every confirmed block after the cursor. Returns the last
    /// processed block, or `None` when there was nothing new to read.
    pub async fn run_once(&self) -> Result<Option<u64>, ListenerError> {
        let head = self.provider.get_block_number().await?;
        let Some(confirmed) = head.checked_sub(self.config.confirmations) else {
            return Ok(None);
        };

        let from = match self.cursor().await? {
            Some((last, hash)) => {
                if self.block_hash(last).await? != hash {
                    return Err(ListenerError::ReorgBeyondConfirmations { block: last });
                }
                last + 1
            }
            None => self.config.start_block,
        };
        if from > confirmed {
            return Ok(None);
        }

        let mut start = from;
        while start <= confirmed {
            let end = confirmed.min(start + self.config.max_range.max(1) - 1);
            let mut logs = self.provider.get_logs(&self.filter(start, end)).await?;
            logs.sort_by_key(|l| (l.block_number, l.log_index));
            for log in &logs {
                self.handle(log).await?;
            }
            let hash = self.block_hash(end).await?;
            self.save_cursor(end, hash).await?;
            start = end + 1;
        }
        Ok(Some(confirmed))
    }

    fn filter(&self, from: u64, to: u64) -> Filter {
        Filter::new()
            .address(self.config.vault)
            .from_block(from)
            .to_block(to)
            .event_signature(vec![
                SettlementVault::QuoteLocked::SIGNATURE_HASH,
                SettlementVault::QuoteCancelled::SIGNATURE_HASH,
                SettlementVault::SettlementInitiated::SIGNATURE_HASH,
                SettlementVault::SettlementReturned::SIGNATURE_HASH,
                SettlementVault::SettlementRefunded::SIGNATURE_HASH,
                SettlementVault::RateDivergence::SIGNATURE_HASH,
                SettlementVault::ReferenceRateStale::SIGNATURE_HASH,
            ])
    }

    async fn handle(&self, log: &Log) -> Result<(), ListenerError> {
        let Ok(decoded) = Event::decode_log(&log.inner) else {
            return Ok(());
        };
        let (name, settlement_ref, payload) = describe(&decoded.data);
        let (Some(tx_hash), Some(log_index), Some(block_number), Some(block_hash)) = (
            log.transaction_hash,
            log.log_index,
            log.block_number,
            log.block_hash,
        ) else {
            return Err(ListenerError::Rpc("log without block position".into()));
        };

        let seen: bool = sqlx::query_scalar(
            "select exists (select 1 from settlement_events where tx_hash = $1 and log_index = $2)",
        )
        .bind(tx_hash.as_slice())
        .bind(log_index as i64)
        .fetch_one(&self.pool)
        .await?;
        if seen {
            return Ok(());
        }

        let mut tx = self.pool.begin().await?;
        let transfer_id: Option<Uuid> =
            sqlx::query_scalar("select id from transfers where settlement_ref = $1")
                .bind(settlement_ref.as_slice())
                .fetch_optional(&mut *tx)
                .await?;
        let Some(transfer_id) = transfer_id else {
            // Not ours: another backend's transfer, or one created before refs were stored.
            tracing::debug!(event = name, %settlement_ref, "vault event for an unknown ref");
            return Ok(());
        };

        let chain = json!({
            "event": name,
            "txHash": tx_hash.to_string(),
            "blockNumber": block_number,
        });
        let target = match &decoded.data {
            Event::SettlementInitiated(_) => Some(OnchainTarget::Settled),
            Event::SettlementReturned(_) => Some(OnchainTarget::Reversing),
            Event::SettlementRefunded(_) => Some(OnchainTarget::Reversed),
            Event::QuoteCancelled(_) => Some(OnchainTarget::Cancelled {
                quote_expired: self
                    .quote_expired_at(&mut tx, transfer_id, block_number)
                    .await?,
            }),
            _ => None,
        };
        let outcome = match target {
            Some(target) => {
                Some(engine::apply_onchain(&mut tx, transfer_id, target, chain.clone()).await?)
            }
            None => None,
        };

        let mut row = payload;
        row["outcome"] = match &outcome {
            None => json!("recorded"),
            Some(OnchainOutcome::Applied(s)) => json!(format!("applied:{}", s.as_str())),
            Some(OnchainOutcome::AlreadyApplied(s)) => json!(format!("already:{}", s.as_str())),
            Some(OnchainOutcome::Ignored(s)) => {
                tracing::warn!(event = name, %transfer_id, status = s.as_str(), "vault event does not fit the transfer's status");
                json!(format!("ignored:{}", s.as_str()))
            }
        };

        let inserted = sqlx::query(
            "insert into settlement_events
               (vault, tx_hash, log_index, block_number, block_hash, event, settlement_ref, transfer_id, payload)
             values ($1, $2, $3, $4, $5, $6, $7, $8, $9)
             on conflict (tx_hash, log_index) do nothing",
        )
        .bind(self.config.vault.as_slice())
        .bind(tx_hash.as_slice())
        .bind(log_index as i64)
        .bind(block_number as i64)
        .bind(block_hash.as_slice())
        .bind(name)
        .bind(settlement_ref.as_slice())
        .bind(transfer_id)
        .bind(&row)
        .execute(&mut *tx)
        .await?
        .rows_affected();

        if inserted == 0 {
            // A concurrent listener recorded it first: undo this pass.
            tx.rollback().await?;
            return Ok(());
        }

        if outcome.is_none() {
            crate::audit::write_audit(
                &mut tx,
                crate::audit::AuditEntry {
                    actor_id: None,
                    actor_role: None,
                    action: audit_action(name),
                    entity_type: "transfer",
                    entity_id: transfer_id.to_string(),
                    before: None,
                    after: Some(row),
                },
            )
            .await?;
        }
        tx.commit().await?;
        Ok(())
    }

    /// Whether the transfer's locked quote had expired by `block`. False when
    /// no lock was seen, so an unexplained cancel becomes REJECTED.
    async fn quote_expired_at(
        &self,
        conn: &mut sqlx::PgConnection,
        transfer_id: Uuid,
        block: u64,
    ) -> Result<bool, ListenerError> {
        let expires_at: Option<i64> = sqlx::query_scalar(
            "select (payload->>'expiresAt')::bigint from settlement_events
              where transfer_id = $1 and event = 'QuoteLocked'
              order by block_number desc limit 1",
        )
        .bind(transfer_id)
        .fetch_optional(&mut *conn)
        .await?;
        let Some(expires_at) = expires_at else {
            return Ok(false);
        };
        let timestamp = self
            .provider
            .get_block_by_number(BlockNumberOrTag::Number(block))
            .await?
            .ok_or(ListenerError::MissingBlock(block))?
            .header
            .timestamp;
        Ok(timestamp as i64 >= expires_at)
    }

    async fn block_hash(&self, block: u64) -> Result<B256, ListenerError> {
        Ok(self
            .provider
            .get_block_by_number(BlockNumberOrTag::Number(block))
            .await?
            .ok_or(ListenerError::MissingBlock(block))?
            .header
            .hash)
    }

    async fn cursor(&self) -> Result<Option<(u64, B256)>, ListenerError> {
        let row: Option<(i64, Vec<u8>)> = sqlx::query_as(
            "select last_block, last_block_hash from settlement_cursor where vault = $1",
        )
        .bind(self.config.vault.as_slice())
        .fetch_optional(&self.pool)
        .await?;
        Ok(row.map(|(block, hash)| (block as u64, B256::from_slice(&hash))))
    }

    async fn save_cursor(&self, block: u64, hash: B256) -> Result<(), ListenerError> {
        sqlx::query(
            "insert into settlement_cursor (vault, last_block, last_block_hash)
             values ($1, $2, $3)
             on conflict (vault) do update
               set last_block = excluded.last_block,
                   last_block_hash = excluded.last_block_hash,
                   updated_at = now()",
        )
        .bind(self.config.vault.as_slice())
        .bind(block as i64)
        .bind(hash.as_slice())
        .execute(&self.pool)
        .await?;
        Ok(())
    }
}

fn audit_action(event: &str) -> &'static str {
    match event {
        "QuoteLocked" => "transfer.quote_locked",
        "RateDivergence" | "ReferenceRateStale" => "transfer.settlement_alert",
        _ => "transfer.settlement_event",
    }
}

/// Event name, its ref, and the fields worth keeping for audit. Amounts are
/// decimal strings: they are `uint256` and must not pass through `f64`.
fn describe(event: &Event) -> (&'static str, B256, Value) {
    fn ccy(code: &alloy::primitives::FixedBytes<3>) -> String {
        String::from_utf8_lossy(code.as_slice()).into_owned()
    }
    match event {
        Event::QuoteLocked(e) => (
            "QuoteLocked",
            e.r#ref,
            json!({
                "quoteId": e.quoteId.to_string(),
                "receiveCurrency": ccy(&e.receiveCurrency),
                "rate": e.rate.to_string(),
                "usdcAmount": e.usdcAmount.to_string(),
                "feeUsdc": e.feeUsdc.to_string(),
                "receiveAmountMinor": e.receiveAmountMinor.to_string(),
                "expiresAt": e.expiresAt,
            }),
        ),
        Event::QuoteCancelled(e) => (
            "QuoteCancelled",
            e.r#ref,
            json!({ "quoteId": e.quoteId.to_string() }),
        ),
        Event::SettlementInitiated(e) => (
            "SettlementInitiated",
            e.r#ref,
            json!({ "partner": e.partner.to_string(), "amount": e.amount.to_string() }),
        ),
        Event::SettlementReturned(e) => (
            "SettlementReturned",
            e.r#ref,
            json!({ "partner": e.partner.to_string(), "amount": e.amount.to_string() }),
        ),
        Event::SettlementRefunded(e) => (
            "SettlementRefunded",
            e.r#ref,
            json!({ "to": e.to.to_string(), "amount": e.amount.to_string() }),
        ),
        Event::RateDivergence(e) => (
            "RateDivergence",
            e.r#ref,
            json!({
                "currency": ccy(&e.currency),
                "quotedRate": e.quotedRate.to_string(),
                "referenceRate": e.referenceRate.to_string(),
                "deviationBps": e.deviationBps.to_string(),
            }),
        ),
        Event::ReferenceRateStale(e) => (
            "ReferenceRateStale",
            e.r#ref,
            json!({
                "currency": ccy(&e.currency),
                "referenceUpdatedAt": e.referenceUpdatedAt,
            }),
        ),
        // Filtered out by `SettlementListener::filter`.
        _ => ("Other", B256::ZERO, json!({})),
    }
}
