use crate::contract::common::{CurrencyCode, Money};
use crate::contract::quote::FirmQuote;
use crate::contract::transfer::{
    Transfer, TransferFailureCategory, TransferState, TransferStateHistoryEntry, TransferStatus,
};
use crate::error::ApiResult;
use crate::util::{is_uuid, iso};
use chrono::{DateTime, Utc};
use serde_json::{json, Value};
use sqlx::types::Json;
use sqlx::PgPool;
use uuid::Uuid;

#[derive(sqlx::FromRow)]
struct TransferRow {
    id: Uuid,
    reference: String,
    customer_id: Uuid,
    idempotency_key: String,
    recipient_id: Uuid,
    send_currency: String,
    receive_currency: String,
    send_amount_minor: i64,
    receive_amount_minor: i64,
    trade_description: Option<String>,
    quote_snapshot: Json<FirmQuote>,
    current_status: String,
    created_at: DateTime<Utc>,
    updated_at: DateTime<Utc>,
    state_entered_at: Option<DateTime<Utc>>,
    state_payload: Option<Value>,
}

fn str_field(p: &Value, key: &str) -> Option<String> {
    p.get(key).and_then(|v| v.as_str()).map(String::from)
}

pub fn build_state(
    status: TransferStatus,
    entered_at: String,
    payload: Option<Value>,
) -> TransferState {
    let p = payload.unwrap_or_else(|| json!({}));
    match status {
        TransferStatus::Created => TransferState::Created { entered_at },
        TransferStatus::Quoted => TransferState::Quoted { entered_at },
        TransferStatus::Screened => TransferState::Screened {
            entered_at,
            hold: p.get("hold").and_then(|v| v.as_bool()).unwrap_or(false),
            expected_resolution_by: str_field(&p, "expectedResolutionBy"),
        },
        TransferStatus::AwaitingFunds => TransferState::AwaitingFunds {
            entered_at,
            funding_reference: str_field(&p, "fundingReference").unwrap_or_default(),
        },
        TransferStatus::Funded => TransferState::Funded { entered_at },
        TransferStatus::Settling => TransferState::Settling { entered_at },
        TransferStatus::Settled => TransferState::Settled { entered_at },
        TransferStatus::PayingOut => TransferState::PayingOut { entered_at },
        TransferStatus::Completed => TransferState::Completed {
            entered_at,
            payout_reference: str_field(&p, "payoutReference").unwrap_or_default(),
        },
        TransferStatus::Rejected => TransferState::Rejected {
            entered_at,
            failure_category: str_field(&p, "failureCategory")
                .and_then(|s| serde_json::from_value(Value::String(s)).ok())
                .unwrap_or(TransferFailureCategory::PartnerFailure),
            reason_code: str_field(&p, "reasonCode").unwrap_or_else(|| "UNKNOWN".into()),
        },
        TransferStatus::Expired => TransferState::Expired { entered_at },
        TransferStatus::Reversing => TransferState::Reversing {
            entered_at,
            reason: str_field(&p, "reason").unwrap_or_default(),
        },
        TransferStatus::Reversed => TransferState::Reversed {
            entered_at,
            reason: str_field(&p, "reason").unwrap_or_default(),
            reversal_ledger_entry_id: str_field(&p, "reversalLedgerEntryId").unwrap_or_default(),
        },
    }
}

impl TransferRow {
    fn into_contract(self) -> ApiResult<Transfer> {
        let send = CurrencyCode::parse(&self.send_currency)?;
        let receive = CurrencyCode::parse(&self.receive_currency)?;
        let status = TransferStatus::parse(&self.current_status)?;
        let entered_at = iso(self.state_entered_at.unwrap_or(self.created_at));
        Ok(Transfer {
            id: self.id.to_string(),
            reference: self.reference,
            customer_id: self.customer_id.to_string(),
            idempotency_key: self.idempotency_key,
            recipient_id: self.recipient_id.to_string(),
            send_currency: send,
            receive_currency: receive,
            send_amount: Money::new(self.send_amount_minor, send),
            receive_amount: Money::new(self.receive_amount_minor, receive),
            trade_description: self.trade_description,
            quote: self.quote_snapshot.0,
            state: build_state(status, entered_at, self.state_payload),
            created_at: iso(self.created_at),
            updated_at: iso(self.updated_at),
        })
    }
}

const SELECT: &str = "
    select t.*,
           h.entered_at as state_entered_at,
           h.payload    as state_payload
      from transfers t
      left join lateral (
        select entered_at, payload from transfer_state_history
         where transfer_id = t.id order by position desc limit 1
      ) h on true
";

pub async fn find_by_id(pool: &PgPool, id: &str) -> ApiResult<Option<Transfer>> {
    if !is_uuid(id) {
        return Ok(None);
    }
    let row: Option<TransferRow> = sqlx::query_as(&format!("{SELECT} where t.id = $1"))
        .bind(Uuid::parse_str(id).unwrap())
        .fetch_optional(pool)
        .await?;
    row.map(TransferRow::into_contract).transpose()
}

pub async fn find_by_id_uuid(pool: &PgPool, id: Uuid) -> ApiResult<Option<Transfer>> {
    let row: Option<TransferRow> = sqlx::query_as(&format!("{SELECT} where t.id = $1"))
        .bind(id)
        .fetch_optional(pool)
        .await?;
    row.map(TransferRow::into_contract).transpose()
}

pub async fn find_by_idempotency_key(
    pool: &PgPool,
    customer_id: Uuid,
    key: &str,
) -> ApiResult<Option<Transfer>> {
    let row: Option<TransferRow> = sqlx::query_as(&format!(
        "{SELECT} where t.customer_id = $1 and t.idempotency_key = $2"
    ))
    .bind(customer_id)
    .bind(key)
    .fetch_optional(pool)
    .await?;
    row.map(TransferRow::into_contract).transpose()
}

pub async fn list_by_customer(
    pool: &PgPool,
    customer_id: Uuid,
    status: Option<TransferStatus>,
) -> ApiResult<Vec<Transfer>> {
    let rows: Vec<TransferRow> = match status {
        Some(s) => {
            sqlx::query_as(&format!(
                "{SELECT} where t.customer_id = $1 and t.current_status = $2 order by t.created_at desc"
            ))
            .bind(customer_id)
            .bind(s.as_str())
            .fetch_all(pool)
            .await?
        }
        None => {
            sqlx::query_as(&format!(
                "{SELECT} where t.customer_id = $1 order by t.created_at desc"
            ))
            .bind(customer_id)
            .fetch_all(pool)
            .await?
        }
    };
    rows.into_iter().map(TransferRow::into_contract).collect()
}

pub struct OwnerAndStatus {
    pub customer_id: Uuid,
    pub current_status: TransferStatus,
}

pub async fn get_owner_and_status(pool: &PgPool, id: &str) -> ApiResult<Option<OwnerAndStatus>> {
    if !is_uuid(id) {
        return Ok(None);
    }
    let row: Option<(Uuid, String)> =
        sqlx::query_as("select customer_id, current_status from transfers where id = $1")
            .bind(Uuid::parse_str(id).unwrap())
            .fetch_optional(pool)
            .await?;
    match row {
        Some((customer_id, status)) => Ok(Some(OwnerAndStatus {
            customer_id,
            current_status: TransferStatus::parse(&status)?,
        })),
        None => Ok(None),
    }
}

pub async fn get_history(
    pool: &PgPool,
    transfer_id: Uuid,
) -> ApiResult<Vec<TransferStateHistoryEntry>> {
    let rows: Vec<(String, DateTime<Utc>, Option<String>)> = sqlx::query_as(
        "select status, entered_at, note from transfer_state_history
          where transfer_id = $1 order by position",
    )
    .bind(transfer_id)
    .fetch_all(pool)
    .await?;
    rows.into_iter()
        .map(|(status, entered_at, note)| {
            Ok(TransferStateHistoryEntry {
                status: TransferStatus::parse(&status)?,
                entered_at: iso(entered_at),
                note,
            })
        })
        .collect()
}

// ---- writes ----

pub struct InsertTransfer<'a> {
    pub reference: &'a str,
    pub customer_id: Uuid,
    pub idempotency_key: &'a str,
    pub recipient_id: Uuid,
    pub send_currency: CurrencyCode,
    pub receive_currency: CurrencyCode,
    pub send_amount_minor: i64,
    pub receive_amount_minor: i64,
    pub trade_description: Option<&'a str>,
    pub quote_snapshot: &'a FirmQuote,
    pub initial_status: TransferStatus,
}

/// Returns None on a unique-violation (concurrent create with the same key).
pub async fn insert(
    conn: &mut sqlx::PgConnection,
    input: InsertTransfer<'_>,
) -> ApiResult<Option<Uuid>> {
    let result: Result<Uuid, sqlx::Error> = sqlx::query_scalar(
        "insert into transfers
           (reference, customer_id, idempotency_key, recipient_id, send_currency, receive_currency,
            send_amount_minor, receive_amount_minor, trade_description, quote_snapshot, current_status)
         values ($1,$2,$3,$4,$5,$6,$7,$8,$9,$10,$11)
         returning id",
    )
    .bind(input.reference)
    .bind(input.customer_id)
    .bind(input.idempotency_key)
    .bind(input.recipient_id)
    .bind(input.send_currency.as_str())
    .bind(input.receive_currency.as_str())
    .bind(input.send_amount_minor)
    .bind(input.receive_amount_minor)
    .bind(input.trade_description)
    .bind(Json(input.quote_snapshot))
    .bind(input.initial_status.as_str())
    .fetch_one(conn)
    .await;

    match result {
        Ok(id) => Ok(Some(id)),
        Err(sqlx::Error::Database(e)) if e.code().as_deref() == Some("23505") => Ok(None),
        Err(e) => Err(e.into()),
    }
}

pub async fn append_history(
    conn: &mut sqlx::PgConnection,
    transfer_id: Uuid,
    status: TransferStatus,
    payload: Option<&Value>,
    note: Option<&str>,
) -> ApiResult<()> {
    sqlx::query(
        "insert into transfer_state_history (transfer_id, position, status, payload, note)
         values ($1,
                 (select coalesce(max(position), -1) + 1 from transfer_state_history where transfer_id = $1),
                 $2, $3, $4)",
    )
    .bind(transfer_id)
    .bind(status.as_str())
    .bind(payload)
    .bind(note)
    .execute(conn)
    .await?;
    Ok(())
}

pub async fn set_status(
    conn: &mut sqlx::PgConnection,
    transfer_id: Uuid,
    status: TransferStatus,
) -> ApiResult<()> {
    sqlx::query("update transfers set current_status = $2, updated_at = now() where id = $1")
        .bind(transfer_id)
        .bind(status.as_str())
        .execute(conn)
        .await?;
    Ok(())
}
