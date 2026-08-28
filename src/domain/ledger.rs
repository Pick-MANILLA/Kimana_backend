//! Ledger reads + the append-only posting primitive.

use crate::contract::common::{CurrencyCode, Money};
use crate::contract::ledger::AccountBalance;
use crate::error::ApiResult;
use crate::util::iso;
use chrono::Utc;
use sqlx::PgPool;
use uuid::Uuid;

#[derive(sqlx::FromRow)]
struct BalanceRow {
    account_id: Uuid,
    currency: String,
    balance_minor: i64,
}

/// One AccountBalance per currency the customer holds — each the signed sum of
/// that account's ledger_entries. `pending` has no source yet, so it is omitted.
pub async fn get_balances(pool: &PgPool, customer_id: Uuid) -> ApiResult<Vec<AccountBalance>> {
    let rows: Vec<BalanceRow> = sqlx::query_as(
        "select a.id as account_id,
                a.currency as currency,
                coalesce(sum(le.amount_minor), 0)::bigint as balance_minor
           from accounts a
           left join ledger_entries le on le.account_id = a.id
          where a.customer_id = $1
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
    if let Some(id) = sqlx::query_scalar::<_, Uuid>(
        "select id from accounts where customer_id = $1 and currency = $2",
    )
    .bind(customer_id)
    .bind(currency.as_str())
    .fetch_optional(&mut *conn)
    .await?
    {
        return Ok(id);
    }
    let id = sqlx::query_scalar::<_, Uuid>(
        "insert into accounts (customer_id, currency) values ($1, $2) returning id",
    )
    .bind(customer_id)
    .bind(currency.as_str())
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

pub struct LedgerPosting<'a> {
    pub account_id: Uuid,
    pub transfer_id: Uuid,
    /// Signed minor units: positive = credit, negative = debit.
    pub amount_minor: i64,
    pub currency: CurrencyCode,
    pub description: &'a str,
    pub reversal_of_entry_id: Option<Uuid>,
}

/// Appends one ledger entry, computing `running_balance_minor` under an account
/// row lock so concurrent postings to the same account serialise.
pub async fn post_ledger_entry(
    conn: &mut sqlx::PgConnection,
    posting: LedgerPosting<'_>,
) -> ApiResult<(Uuid, i64)> {
    sqlx::query("select id from accounts where id = $1 for update")
        .bind(posting.account_id)
        .execute(&mut *conn)
        .await?;

    let prev = account_balance_minor(&mut *conn, posting.account_id).await?;
    let running = prev + posting.amount_minor;

    let entry_id = sqlx::query_scalar::<_, Uuid>(
        "insert into ledger_entries
           (account_id, transfer_id, amount_minor, currency, running_balance_minor, description, reversal_of_entry_id)
         values ($1, $2, $3, $4, $5, $6, $7)
         returning id",
    )
    .bind(posting.account_id)
    .bind(posting.transfer_id)
    .bind(posting.amount_minor)
    .bind(posting.currency.as_str())
    .bind(running)
    .bind(posting.description)
    .bind(posting.reversal_of_entry_id)
    .fetch_one(&mut *conn)
    .await?;

    Ok((entry_id, running))
}
