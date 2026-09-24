//! Ledger reads + the append-only posting primitive.

use crate::contract::common::{CurrencyCode, Money};
use crate::contract::ledger::AccountBalance;
use crate::error::ApiResult;
use crate::util::iso;
use chrono::Utc;
use sqlx::{PgConnection, PgPool};
use uuid::Uuid;

#[derive(sqlx::FromRow)]
struct BalanceRow {
    account_id: Uuid,
    currency: String,
    balance_minor: i64,
}

/// Ledger currency of the settlement wallet's USDC account. It is not a
/// `CurrencyCode`: USDC only appears on the `/settlement` surface, so every
/// other read of the customer's accounts leaves it out. Held in cents, like
/// USD (`settlement::units::cents_to_usdc` converts to base units).
pub const USDC: &str = "USDC";

/// One AccountBalance per currency the customer holds — each the signed sum of
/// that account's ledger_entries. `pending` has no source yet, so it is omitted.
/// The USDC settlement account is excluded (see `USDC`).
pub async fn get_balances(pool: &PgPool, customer_id: Uuid) -> ApiResult<Vec<AccountBalance>> {
    let rows: Vec<BalanceRow> = sqlx::query_as(
        "select a.id as account_id,
                a.currency as currency,
                coalesce(sum(le.amount_minor), 0)::bigint as balance_minor
           from accounts a
           left join ledger_entries le on le.account_id = a.id
          where a.customer_id = $1 and a.currency <> 'USDC'
          group by a.id, a.currency
          order by a.currency",
    )
    .bind(customer_id)
    .fetch_all(pool)
    .await?;

    let as_of = iso(Utc::now());
    rows.into_iter()
        .map(|r| {
            let currency = CurrencyCode::parse(&r.currency)?;
            Ok(AccountBalance {
                account_id: r.account_id.to_string(),
                currency,
                balance: Money::new(r.balance_minor, currency),
                pending: None,
                as_of: as_of.clone(),
            })
        })
        .collect()
}

pub async fn get_or_create_account(
    conn: &mut sqlx::PgConnection,
    customer_id: Uuid,
    currency: CurrencyCode,
) -> ApiResult<Uuid> {
    get_or_create_account_by_code(conn, customer_id, currency.as_str()).await
}

/// `get_or_create_account` for a ledger currency that isn't a `CurrencyCode`
/// (the `USDC` settlement account).
pub async fn get_or_create_account_by_code(
    conn: &mut sqlx::PgConnection,
    customer_id: Uuid,
    currency: &str,
) -> ApiResult<Uuid> {
    if let Some(id) = sqlx::query_scalar::<_, Uuid>(
        "select id from accounts where customer_id = $1 and currency = $2",
    )
    .bind(customer_id)
    .bind(currency)
    .fetch_optional(&mut *conn)
    .await?
    {
        return Ok(id);
    }
    let id = sqlx::query_scalar::<_, Uuid>(
        "insert into accounts (customer_id, currency) values ($1, $2) returning id",
    )
    .bind(customer_id)
    .bind(currency)
    .fetch_one(&mut *conn)
    .await?;
    Ok(id)
}

pub async fn account_balance_minor(
    conn: &mut sqlx::PgConnection,
    account_id: Uuid,
) -> ApiResult<i64> {
    let balance = sqlx::query_scalar::<_, i64>(
        "select coalesce(sum(amount_minor), 0)::bigint from ledger_entries where account_id = $1",
    )
    .bind(account_id)
    .fetch_one(&mut *conn)
    .await?;
    Ok(balance)
}

/// Statuses that no longer count against an exposure limit.
const TERMINAL_STATUSES: &str = "('COMPLETED', 'REJECTED', 'EXPIRED', 'REVERSED')";

/// Sum of a customer's non-terminal transfer amounts in one currency.
/// This is the per-customer side of Business Rule #7's exposure ceiling.
pub async fn customer_open_exposure_minor(
    conn: &mut PgConnection,
    customer_id: Uuid,
    currency: CurrencyCode,
) -> ApiResult<i64> {
    let sum: i64 = sqlx::query_scalar(&format!(
        "select coalesce(sum(send_amount_minor), 0)::bigint
           from transfers
          where customer_id = $1
            and send_currency = $2
            and current_status not in {TERMINAL_STATUSES}"
    ))
    .bind(customer_id)
    .bind(currency.as_str())
    .fetch_one(conn)
    .await?;
    Ok(sum)
}

/// A customer's own exposure ceilings; `None` falls back to the config default.
pub struct CustomerLimits {
    pub max_transfer_amount_minor: Option<i64>,
    pub max_aggregate_exposure_minor: Option<i64>,
}

/// Reads the customer's limits under a row lock. The lock is held until the
/// surrounding transaction ends, so concurrent transfer creates for the same
/// customer serialize and each sees the other's committed exposure.
pub async fn lock_customer_limits(
    conn: &mut PgConnection,
    customer_id: Uuid,
) -> ApiResult<CustomerLimits> {
    let (max_transfer_amount_minor, max_aggregate_exposure_minor): (Option<i64>, Option<i64>) =
        sqlx::query_as(
            "select max_transfer_amount_minor, max_aggregate_exposure_minor
               from customers
              where id = $1
                for update",
        )
        .bind(customer_id)
        .fetch_one(conn)
        .await?;
    Ok(CustomerLimits {
        max_transfer_amount_minor,
        max_aggregate_exposure_minor,
    })
}

/// Sets (or, with `None`, clears) a customer's per-transfer send-amount
/// ceiling — the risk-derived override `CustomerLimits`/`lock_customer_limits`
/// read back. Called on KYB approval and rescreening (Business Rule #1's
/// risk-based limits), never by the transfer path itself.
pub async fn set_transfer_limit(
    conn: &mut sqlx::PgConnection,
    customer_id: Uuid,
    max_transfer_amount_minor: Option<i64>,
) -> ApiResult<()> {
    sqlx::query("update customers set max_transfer_amount_minor = $2 where id = $1")
        .bind(customer_id)
        .bind(max_transfer_amount_minor)
        .execute(conn)
        .await?;
    Ok(())
}

/// Platform-wide non-terminal exposure per currency, for the FX-exposure
/// guardrail. Visible for now; a hard platform-wide cap is a later refinement.
pub async fn platform_open_exposure_by_currency(
    conn: &mut PgConnection,
) -> ApiResult<Vec<(CurrencyCode, i64)>> {
    let rows: Vec<(String, i64)> = sqlx::query_as(&format!(
        "select send_currency, coalesce(sum(send_amount_minor), 0)::bigint
           from transfers
          where current_status not in {TERMINAL_STATUSES}
          group by send_currency"
    ))
    .fetch_all(conn)
    .await?;
    rows.into_iter()
        .map(|(currency, minor)| Ok((CurrencyCode::parse(&currency)?, minor)))
        .collect()
}

pub struct LedgerPosting<'a> {
    pub account_id: Uuid,
    pub transfer_id: Uuid,
    /// Signed minor units: positive = credit, negative = debit.
    pub amount_minor: i64,
    pub currency: CurrencyCode,
    pub description: &'a str,
    pub reversal_of_entry_id: Option<Uuid>,
}

/// Appends one ledger entry for a transfer.
pub async fn post_ledger_entry(
    conn: &mut sqlx::PgConnection,
    posting: LedgerPosting<'_>,
) -> ApiResult<(Uuid, i64)> {
    append_entry(
        conn,
        posting.account_id,
        posting.amount_minor,
        posting.currency.as_str(),
        posting.description,
        EntrySource::Transfer(posting.transfer_id),
        posting.reversal_of_entry_id,
    )
    .await
}

/// One leg of a settlement-wallet trade (`domain::settlement`).
pub struct SettlementPosting<'a> {
    pub account_id: Uuid,
    pub trade_id: Uuid,
    /// Signed minor units: positive = credit, negative = debit.
    pub amount_minor: i64,
    /// A `CurrencyCode` string, or `USDC`.
    pub currency: &'a str,
    pub description: &'a str,
}

pub async fn post_settlement_entry(
    conn: &mut sqlx::PgConnection,
    posting: SettlementPosting<'_>,
) -> ApiResult<(Uuid, i64)> {
    append_entry(
        conn,
        posting.account_id,
        posting.amount_minor,
        posting.currency,
        posting.description,
        EntrySource::SettlementTrade(posting.trade_id),
        None,
    )
    .await
}

enum EntrySource {
    Transfer(Uuid),
    SettlementTrade(Uuid),
}

/// Appends one ledger entry, computing `running_balance_minor` under an account
/// row lock so concurrent postings to the same account serialise.
async fn append_entry(
    conn: &mut sqlx::PgConnection,
    account_id: Uuid,
    amount_minor: i64,
    currency: &str,
    description: &str,
    source: EntrySource,
    reversal_of_entry_id: Option<Uuid>,
) -> ApiResult<(Uuid, i64)> {
    sqlx::query("select id from accounts where id = $1 for update")
        .bind(account_id)
        .execute(&mut *conn)
        .await?;

    let prev = account_balance_minor(&mut *conn, account_id).await?;
    let running = prev + amount_minor;

    let (transfer_id, settlement_trade_id) = match source {
        EntrySource::Transfer(id) => (Some(id), None),
        EntrySource::SettlementTrade(id) => (None, Some(id)),
    };
    let entry_id = sqlx::query_scalar::<_, Uuid>(
        "insert into ledger_entries
           (account_id, transfer_id, settlement_trade_id, amount_minor, currency,
            running_balance_minor, description, reversal_of_entry_id)
         values ($1, $2, $3, $4, $5, $6, $7, $8)
         returning id",
    )
    .bind(account_id)
    .bind(transfer_id)
    .bind(settlement_trade_id)
    .bind(amount_minor)
    .bind(currency)
    .bind(running)
    .bind(description)
    .bind(reversal_of_entry_id)
    .fetch_one(&mut *conn)
    .await?;

    Ok((entry_id, running))
}
