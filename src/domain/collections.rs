//! Collections: a customer asks to be paid and receives the money into their
//! ledger (issue #79). Transfers only model the customer as payer; here the
//! customer is the payee.
//!
//! A collection is a payment request with a reference (`CL-XXXXXX`) and the
//! instructions the payer follows (`payIn`). The currency picks the partner:
//!
//! * NGN goes through Yellow Card. Each request opens its own receive, whose
//!   bank account only this payer pays into, so it expires with the receive.
//!   Yellow Card's webhook is only a trigger: the receive is fetched back and
//!   credited when its status is `complete`, for the amount it reports.
//! * USD goes through Bridge. The customer has one standing US virtual
//!   account (linked by ops, see `link_bridge_account`). Every processed
//!   deposit is credited; one whose memo quotes an open request's reference
//!   also marks that request paid.
//!
//! Money a partner confirms is always credited, even when it doesn't match
//! the request exactly: it has already arrived.

use crate::audit::{write_audit, AuditEntry};
use crate::contract::common::{CurrencyCode, Money};
use crate::domain::ledger;
use crate::error::{ApiError, ApiResult, ErrorResponse};
use crate::http::{Body, Path, Session};
use crate::partners::bridge::{self, dollars_to_cents, WebhookCheck};
use crate::partners::partner_error;
use crate::partners::yellowcard::{self, major_to_minor, NewReceive};
use crate::state::AppState;
use crate::util::{is_uuid, iso, iso_opt, tagged_reference};
use axum::body::Bytes;
use axum::extract::State;
use axum::http::{HeaderMap, StatusCode};
use axum::routing::{get, post};
use axum::{Json, Router};
use chrono::{DateTime, Duration, Utc};
use regex::Regex;
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use sqlx::types::Json as SqlJson;
use sqlx::{PgConnection, PgPool};
use std::sync::LazyLock;
use utoipa::ToSchema;
use uuid::Uuid;

const YELLOWCARD: &str = "yellowcard";
const BRIDGE: &str = "bridge";
const DEFAULT_TTL_DAYS: i64 = 7;
const MAX_TTL_DAYS: i64 = 90;
const MAX_PAYER_NAME_CHARS: usize = 140;
const MAX_NOTE_CHARS: usize = 280;
const LIST_LIMIT: i64 = 100;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, ToSchema)]
#[serde(rename_all = "SCREAMING_SNAKE_CASE")]
pub enum CollectionStatus {
    Pending,
    Paid,
    /// Still pending when `expiresAt` passed. Never stored; derived on read.
    Expired,
    Cancelled,
}

impl CollectionStatus {
    fn parse(value: &str) -> ApiResult<Self> {
        match value {
            "PENDING" => Ok(CollectionStatus::Pending),
            "PAID" => Ok(CollectionStatus::Paid),
            "EXPIRED" => Ok(CollectionStatus::Expired),
            "CANCELLED" => Ok(CollectionStatus::Cancelled),
            _ => Err(ApiError::server_error()),
        }
    }
}

/// How the payer pays. Which fields are present depends on `method`.
#[derive(Debug, Clone, Default, Serialize, Deserialize, ToSchema)]
#[serde(rename_all = "camelCase")]
pub struct PayIn {
    /// `NG_BANK_TRANSFER` or `US_BANK_TRANSFER`.
    pub method: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub bank_name: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub bank_address: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub account_name: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub account_number: Option<String>,
    /// US ABA routing number.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub routing_number: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub beneficiary_address: Option<String>,
    /// e.g. `ach_push`, `wire`.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub payment_rails: Vec<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub payment_link: Option<String>,
    /// What the payer puts in the transfer memo or narration.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub memo: Option<String>,
}

fn us_pay_in(instructions: &bridge::DepositInstructions, memo: Option<&str>) -> PayIn {
    PayIn {
        method: "US_BANK_TRANSFER".to_string(),
        bank_name: instructions.bank_name.clone(),
        bank_address: instructions.bank_address.clone(),
        account_name: instructions.bank_beneficiary_name.clone(),
        account_number: instructions.bank_account_number.clone(),
        routing_number: instructions.bank_routing_number.clone(),
        beneficiary_address: instructions.bank_beneficiary_address.clone(),
        payment_rails: instructions.payment_rails.clone(),
        payment_link: None,
        memo: memo.map(String::from),
    }
}

/// The inbound payment that settled a collection. Its amount can differ
/// from the request's: partner-confirmed money is credited as received.
#[derive(Debug, Clone, Serialize, ToSchema)]
#[serde(rename_all = "camelCase")]
pub struct CollectionPayment {
    pub amount: Money,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub payer_name: Option<String>,
    pub received_at: String,
}

/// A payment request: what the customer is owed, and how the payer pays.
#[derive(Debug, Clone, Serialize, ToSchema)]
#[serde(rename_all = "camelCase")]
pub struct Collection {
    pub id: String,
    /// e.g. `CL-2H4F9K`.
    pub reference: String,
    pub amount: Money,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub payer_name: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub note: Option<String>,
    pub status: CollectionStatus,
    pub pay_in: PayIn,
    pub expires_at: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub paid_at: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub cancelled_at: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub payment: Option<CollectionPayment>,
    pub created_at: String,
}

/// A standing account the customer can be paid into at any time.
#[derive(Debug, Clone, Serialize, ToSchema)]
#[serde(rename_all = "camelCase")]
pub struct ReceivingAccount {
    pub currency: CurrencyCode,
    pub pay_in: PayIn,
}

#[derive(sqlx::FromRow)]
struct CollectionRow {
    id: Uuid,
    reference: String,
    customer_id: Uuid,
    amount_minor: i64,
    currency: String,
    payer_name: Option<String>,
    note: Option<String>,
    status: String,
    provider: String,
    provider_ref: Option<String>,
    pay_in: SqlJson<PayIn>,
    expires_at: DateTime<Utc>,
    paid_at: Option<DateTime<Utc>>,
    cancelled_at: Option<DateTime<Utc>>,
    created_at: DateTime<Utc>,
    payment_amount_minor: Option<i64>,
    payment_currency: Option<String>,
    payment_payer_name: Option<String>,
    payment_received_at: Option<DateTime<Utc>>,
}

impl CollectionRow {
    fn status(&self) -> ApiResult<CollectionStatus> {
        CollectionStatus::parse(&self.status)
    }

    fn into_collection(self) -> ApiResult<Collection> {
        let currency = CurrencyCode::parse(&self.currency)?;
        let status = self.status()?;
        let payment = match (self.payment_amount_minor, self.payment_received_at) {
            (Some(amount_minor), Some(received_at)) => {
                let paid_in = match self.payment_currency.as_deref() {
                    Some(code) => CurrencyCode::parse(code)?,
                    None => currency,
                };
                Some(CollectionPayment {
                    amount: Money::new(amount_minor, paid_in),
                    payer_name: self.payment_payer_name,
                    received_at: iso(received_at),
                })
            }
            _ => None,
        };
        Ok(Collection {
            id: self.id.to_string(),
            reference: self.reference,
            amount: Money::new(self.amount_minor, currency),
            payer_name: self.payer_name,
            note: self.note,
            status,
            pay_in: self.pay_in.0,
            expires_at: iso(self.expires_at),
            paid_at: iso_opt(self.paid_at),
            cancelled_at: iso_opt(self.cancelled_at),
            payment,
            created_at: iso(self.created_at),
        })
    }
}

/// Selects `CollectionRow`s. The `where` clause is appended by the caller.
/// A pending request past its expiry reads as `EXPIRED`.
const SELECT: &str = "select c.id, c.reference, c.customer_id, c.amount_minor, c.currency,
            c.payer_name, c.note,
            case when c.status = 'PENDING' and c.expires_at <= now()
                 then 'EXPIRED' else c.status end as status,
            c.provider, c.provider_ref, c.pay_in,
            c.expires_at, c.paid_at, c.cancelled_at, c.created_at,
            p.amount_minor as payment_amount_minor,
            p.currency as payment_currency,
            p.payer_name as payment_payer_name,
            p.received_at as payment_received_at
       from collections c
       left join inbound_payments p on p.collection_id = c.id";

async fn find_by_id(conn: &mut PgConnection, id: Uuid) -> ApiResult<Option<CollectionRow>> {
    Ok(sqlx::query_as(&format!("{SELECT} where c.id = $1"))
        .bind(id)
        .fetch_optional(conn)
        .await?)
}

async fn find_by_key(pool: &PgPool, customer_id: Uuid, key: &str) -> ApiResult<Option<Collection>> {
    let row: Option<CollectionRow> = sqlx::query_as(&format!(
        "{SELECT} where c.customer_id = $1 and c.idempotency_key = $2"
    ))
    .bind(customer_id)
    .bind(key)
    .fetch_optional(pool)
    .await?;
    row.map(CollectionRow::into_collection).transpose()
}

/// Trims an optional free-text field, dropping it when blank.
fn optional_text(
    value: Option<String>,
    field: &str,
    max_chars: usize,
) -> ApiResult<Option<String>> {
    let Some(value) = value
        .map(|v| v.trim().to_string())
        .filter(|v| !v.is_empty())
    else {
        return Ok(None);
    };
    if value.chars().count() > max_chars {
        return Err(ApiError::validation(format!(
            "{field} must be at most {max_chars} characters."
        )));
    }
    Ok(Some(value))
}

fn parse_time(raw: Option<&str>) -> Option<DateTime<Utc>> {
    DateTime::parse_from_rfc3339(raw?.trim())
        .ok()
        .map(|t| t.with_timezone(&Utc))
}

struct BridgeAccount {
    instructions: bridge::DepositInstructions,
}

async fn bridge_account(pool: &PgPool, customer_id: Uuid) -> ApiResult<Option<BridgeAccount>> {
    let row: Option<SqlJson<bridge::DepositInstructions>> = sqlx::query_scalar(
        "select deposit_instructions from bridge_virtual_accounts where customer_id = $1",
    )
    .bind(customer_id)
    .fetch_optional(pool)
    .await?;
    Ok(row.map(|instructions| BridgeAccount {
        instructions: instructions.0,
    }))
}

// ---- customer side ----

struct CreateInput {
    idempotency_key: String,
    amount: Money,
    payer_name: Option<String>,
    note: Option<String>,
    expires_at: Option<String>,
}

struct Opened {
    provider: &'static str,
    provider_ref: Option<String>,
    pay_in: PayIn,
    expires_at: DateTime<Utc>,
}

/// Opens a Yellow Card receive for an NGN request.
async fn open_yellowcard(
    state: &AppState,
    session: &Session,
    id: Uuid,
    reference: &str,
    amount_minor: i64,
    note: Option<&str>,
    expires_at: DateTime<Utc>,
) -> ApiResult<Opened> {
    let yc = state
        .partners
        .yellowcard
        .as_ref()
        .ok_or_else(|| ApiError::validation("Receiving NGN isn't available yet."))?;
    let (legal_name, email): (String, Option<String>) = sqlx::query_as(
        "select c.legal_name, u.email
           from customers c join users u on u.id = c.primary_user_id
          where c.id = $1",
    )
    .bind(session.customer_id)
    .fetch_one(&state.pool)
    .await?;
    let reason = match note {
        Some(note) => note.to_string(),
        None => format!("Payment request {reference}"),
    };
    let (receive, _) = yc
        .submit_receive(NewReceive {
            sequence_id: &id.to_string(),
            amount_minor,
            reason: &reason,
            business_name: &legal_name,
            email: email.as_deref(),
        })
        .await?;
    let Some(bank) = receive.bank_info.filter(|b| b.account_number.is_some()) else {
        // The receive exists but the payer has nowhere to pay.
        let _ = yc.cancel_receive(&receive.id).await;
        return Err(partner_error(
            YELLOWCARD,
            "/business/receive",
            format!("receive {} came back without bankInfo", receive.id),
        ));
    };
    // The account only takes money while the receive is open.
    let expires_at = parse_time(receive.expires_at.as_deref())
        .map_or(expires_at, |yc_expiry| yc_expiry.min(expires_at));
    Ok(Opened {
        provider: YELLOWCARD,
        provider_ref: Some(receive.id),
        pay_in: PayIn {
            method: "NG_BANK_TRANSFER".to_string(),
            bank_name: bank.name,
            account_name: bank.account_name,
            account_number: bank.account_number,
            payment_link: bank.payment_link,
            memo: Some(reference.to_string()),
            ..PayIn::default()
        },
        expires_at,
    })
}

async fn create_collection(
    state: &AppState,
    session: &Session,
    input: CreateInput,
) -> ApiResult<Collection> {
    if input.idempotency_key.trim().len() < 8 {
        return Err(ApiError::validation("Provide a stable idempotency key."));
    }
    if let Some(existing) =
        find_by_key(&state.pool, session.customer_id, &input.idempotency_key).await?
    {
        return Ok(existing);
    }
    if input.amount.amount_minor <= 0 {
        return Err(ApiError::validation("Enter an amount greater than zero."));
    }
    let currency = input.amount.currency;
    let payer_name = optional_text(input.payer_name, "Payer name", MAX_PAYER_NAME_CHARS)?;
    let note = optional_text(input.note, "Note", MAX_NOTE_CHARS)?;

    let now = Utc::now();
    let expires_at = match input.expires_at.as_deref().map(str::trim) {
        None | Some("") => now + Duration::days(DEFAULT_TTL_DAYS),
        Some(raw) => parse_time(Some(raw))
            .ok_or_else(|| ApiError::validation("expiresAt must be an ISO-8601 timestamp."))?,
    };
    if expires_at <= now {
        return Err(ApiError::validation("Choose an expiry in the future."));
    }
    if expires_at > now + Duration::days(MAX_TTL_DAYS) {
        return Err(ApiError::validation(format!(
            "A payment request can stay open for at most {MAX_TTL_DAYS} days."
        )));
    }

    let id = Uuid::new_v4();
    let reference = tagged_reference("CL");
    let opened = match currency {
        CurrencyCode::Ngn => {
            open_yellowcard(
                state,
                session,
                id,
                &reference,
                input.amount.amount_minor,
                note.as_deref(),
                expires_at,
            )
            .await?
        }
        CurrencyCode::Usd => {
            let account = bridge_account(&state.pool, session.customer_id)
                .await?
                .ok_or_else(|| {
                    ApiError::validation(
                        "Your USD receiving account isn't set up yet. Contact support to open one.",
                    )
                })?;
            Opened {
                provider: BRIDGE,
                provider_ref: None,
                pay_in: us_pay_in(&account.instructions, Some(&reference)),
                expires_at,
            }
        }
        other => {
            return Err(ApiError::validation(format!(
                "Payments can't be collected in {} yet.",
                other.as_str()
            )))
        }
    };

    let mut tx = state.pool.begin().await?;
    let inserted: Option<Uuid> = sqlx::query_scalar(
        "insert into collections
           (id, reference, customer_id, idempotency_key, amount_minor, currency,
            payer_name, note, provider, provider_ref, pay_in, expires_at, created_by)
         values ($1, $2, $3, $4, $5, $6, $7, $8, $9, $10, $11, $12, $13)
         on conflict (customer_id, idempotency_key) do nothing
         returning id",
    )
    .bind(id)
    .bind(&reference)
    .bind(session.customer_id)
    .bind(&input.idempotency_key)
    .bind(input.amount.amount_minor)
    .bind(currency.as_str())
    .bind(&payer_name)
    .bind(&note)
    .bind(opened.provider)
    .bind(&opened.provider_ref)
    .bind(SqlJson(&opened.pay_in))
    .bind(opened.expires_at)
    .bind(session.user_id)
    .fetch_optional(&mut *tx)
    .await?;
    if inserted.is_none() {
        // A concurrent request with the same key committed first; close the
        // receive this one opened so nobody pays into it.
        tx.rollback().await?;
        if let (Some(yc), Some(receive)) = (&state.partners.yellowcard, &opened.provider_ref) {
            let _ = yc.cancel_receive(receive).await;
        }
        return find_by_key(&state.pool, session.customer_id, &input.idempotency_key)
            .await?
            .ok_or_else(ApiError::server_error);
    }
    let row = find_by_id(&mut tx, id)
        .await?
        .ok_or_else(ApiError::server_error)?;

    write_audit(
        &mut tx,
        AuditEntry {
            actor_id: Some(session.user_id),
            actor_role: Some(session.role_str()),
            action: "collection.create",
            entity_type: "collection",
            entity_id: id.to_string(),
            before: None,
            after: Some(json!({
                "reference": row.reference,
                "amountMinor": row.amount_minor,
                "currency": row.currency,
                "provider": row.provider,
                "providerRef": row.provider_ref,
                "expiresAt": iso(row.expires_at),
            })),
        },
    )
    .await?;
    tx.commit().await?;
    row.into_collection()
}

async fn list_collections(state: &AppState, session: &Session) -> ApiResult<Vec<Collection>> {
    let rows: Vec<CollectionRow> = sqlx::query_as(&format!(
        "{SELECT} where c.customer_id = $1 order by c.created_at desc, c.id limit $2"
    ))
    .bind(session.customer_id)
    .bind(LIST_LIMIT)
    .fetch_all(&state.pool)
    .await?;
    rows.into_iter()
        .map(CollectionRow::into_collection)
        .collect()
}

/// The session customer's collection, or `NOT_FOUND` (also for another
/// customer's id, so ids can't be probed).
async fn owned_row(
    conn: &mut PgConnection,
    session: &Session,
    id: &str,
    lock: bool,
) -> ApiResult<CollectionRow> {
    let not_found = || ApiError::not_found("Payment request not found.");
    if !is_uuid(id) {
        return Err(not_found());
    }
    let id = Uuid::parse_str(id).map_err(|_| not_found())?;
    if lock {
        sqlx::query("select id from collections where id = $1 and customer_id = $2 for update")
            .bind(id)
            .bind(session.customer_id)
            .execute(&mut *conn)
            .await?;
    }
    find_by_id(conn, id)
        .await?
        .filter(|row| row.customer_id == session.customer_id)
        .ok_or_else(not_found)
}

async fn get_collection(state: &AppState, session: &Session, id: &str) -> ApiResult<Collection> {
    let mut conn = state.pool.acquire().await?;
    owned_row(&mut conn, session, id, false)
        .await?
        .into_collection()
}

async fn cancel_collection(state: &AppState, session: &Session, id: &str) -> ApiResult<Collection> {
    let mut tx = state.pool.begin().await?;
    let row = owned_row(&mut tx, session, id, true).await?;
    match row.status()? {
        CollectionStatus::Pending => {}
        CollectionStatus::Cancelled => {
            // Cancelling twice is a no-op.
            tx.rollback().await?;
            return row.into_collection();
        }
        CollectionStatus::Paid => {
            return Err(ApiError::conflict(
                "This payment request has already been paid.",
            ))
        }
        CollectionStatus::Expired => {
            return Err(ApiError::conflict("This payment request has expired."))
        }
    }

    // Close the partner side first, holding the row lock, so a payment can't
    // land between the two. If Yellow Card refuses (the payer may already be
    // paying), the request stays open.
    if row.provider == YELLOWCARD {
        let (Some(yc), Some(receive)) = (&state.partners.yellowcard, &row.provider_ref) else {
            return Err(partner_error(
                YELLOWCARD,
                "/business/receive/cancel",
                "Yellow Card isn't configured",
            ));
        };
        yc.cancel_receive(receive).await?;
    }

    sqlx::query(
        "update collections
            set status = 'CANCELLED', cancelled_at = now(), updated_at = now()
          where id = $1",
    )
    .bind(row.id)
    .execute(&mut *tx)
    .await?;
    write_audit(
        &mut tx,
        AuditEntry {
            actor_id: Some(session.user_id),
            actor_role: Some(session.role_str()),
            action: "collection.cancel",
            entity_type: "collection",
            entity_id: row.id.to_string(),
            before: Some(json!({ "status": "PENDING" })),
            after: Some(json!({ "status": "CANCELLED" })),
        },
    )
    .await?;
    let row = find_by_id(&mut tx, row.id)
        .await?
        .ok_or_else(ApiError::server_error)?;
    tx.commit().await?;
    row.into_collection()
}

async fn receiving_accounts(
    state: &AppState,
    session: &Session,
) -> ApiResult<Vec<ReceivingAccount>> {
    Ok(bridge_account(&state.pool, session.customer_id)
        .await?
        .map(|account| ReceivingAccount {
            currency: CurrencyCode::Usd,
            pay_in: us_pay_in(&account.instructions, None),
        })
        .into_iter()
        .collect())
}

// ---- inbound money ----

/// One payment a partner confirmed.
struct Inbound<'a> {
    customer_id: Uuid,
    /// The request this payment is for, if any.
    collection_id: Option<Uuid>,
    /// Mark the request paid even when it's no longer open. True for a Yellow
    /// Card receive, which only that request's payer can pay into.
    settles_closed: bool,
    provider: &'a str,
    provider_payment_id: &'a str,
    amount_minor: i64,
    currency: CurrencyCode,
    payer_name: Option<String>,
    description: Option<String>,
    received_at: DateTime<Utc>,
    raw: Value,
}

/// Records the payment, credits the customer and, when it settles a request,
/// marks that request paid: one transaction. `false` for a payment already
/// applied (a redelivery), which changes nothing.
async fn apply_inbound(state: &AppState, p: Inbound<'_>) -> ApiResult<bool> {
    let mut tx = state.pool.begin().await?;

    // Lock the request first, so two payments quoting it serialise and only
    // the first settles it.
    let mut settles = None;
    if let Some(collection_id) = p.collection_id {
        sqlx::query("select id from collections where id = $1 for update")
            .bind(collection_id)
            .execute(&mut *tx)
            .await?;
        if let Some(row) = find_by_id(&mut tx, collection_id).await? {
            let open = row.status()? == CollectionStatus::Pending;
            let unpaid = row.payment_received_at.is_none();
            if row.customer_id == p.customer_id && unpaid && (open || p.settles_closed) {
                settles = Some(row);
            }
        }
    }

    let payment_id: Option<Uuid> = sqlx::query_scalar(
        "insert into inbound_payments
           (customer_id, collection_id, provider, provider_payment_id, amount_minor,
            currency, payer_name, description, received_at, raw)
         values ($1, $2, $3, $4, $5, $6, $7, $8, $9, $10)
         on conflict (provider, provider_payment_id) do nothing
         returning id",
    )
    .bind(p.customer_id)
    .bind(settles.as_ref().map(|row| row.id))
    .bind(p.provider)
    .bind(p.provider_payment_id)
    .bind(p.amount_minor)
    .bind(p.currency.as_str())
    .bind(&p.payer_name)
    .bind(&p.description)
    .bind(p.received_at)
    .bind(&p.raw)
    .fetch_optional(&mut *tx)
    .await?;
    let Some(payment_id) = payment_id else {
        tx.rollback().await?;
        return Ok(false);
    };

    let account = ledger::get_or_create_account(&mut tx, p.customer_id, p.currency).await?;
    let (_, balance) = ledger::post_inbound_entry(
        &mut tx,
        ledger::InboundPosting {
            account_id: account,
            payment_id,
            amount_minor: p.amount_minor,
            currency: p.currency,
            description: "Payment received",
        },
    )
    .await?;
    if let Some(row) = &settles {
        sqlx::query(
            "update collections
                set status = 'PAID', paid_at = now(), updated_at = now()
              where id = $1",
        )
        .bind(row.id)
        .execute(&mut *tx)
        .await?;
        if row.amount_minor != p.amount_minor {
            tracing::warn!(
                reference = %row.reference,
                requested = row.amount_minor,
                received = p.amount_minor,
                "collection paid with a different amount"
            );
        }
    }
    write_audit(
        &mut tx,
        AuditEntry {
            actor_id: None,
            actor_role: None,
            action: "collection.payment_received",
            entity_type: "inbound_payment",
            entity_id: payment_id.to_string(),
            before: None,
            after: Some(json!({
                "provider": p.provider,
                "providerPaymentId": p.provider_payment_id,
                "amountMinor": p.amount_minor,
                "currency": p.currency.as_str(),
                "collectionId": settles.as_ref().map(|row| row.id.to_string()),
                "reference": settles.as_ref().map(|row| row.reference.clone()),
                "balanceMinor": balance,
            })),
        },
    )
    .await?;
    tx.commit().await?;
    Ok(true)
}

/// Response to a partner webhook.
#[derive(Debug, Serialize, ToSchema)]
#[serde(rename_all = "camelCase")]
pub struct WebhookAck {
    /// True when the event moved money (now or on an earlier delivery).
    pub applied: bool,
    /// True when this delivery repeated one already applied.
    pub duplicate: bool,
}

const IGNORED: WebhookAck = WebhookAck {
    applied: false,
    duplicate: false,
};

fn route_not_found() -> ApiError {
    ApiError::not_found("That endpoint doesn't exist.")
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct YellowCardEvent {
    id: String,
    #[serde(default)]
    event: Option<String>,
}

async fn receive_yellowcard(
    state: &AppState,
    headers: &HeaderMap,
    body: &[u8],
) -> ApiResult<WebhookAck> {
    let yc = state
        .partners
        .yellowcard
        .as_ref()
        .ok_or_else(route_not_found)?;
    let signature = headers
        .get("x-yc-signature")
        .and_then(|v| v.to_str().ok())
        .unwrap_or_default();
    if !yc.verify_webhook(signature, body) {
        return Err(ApiError::unauthorized("Invalid webhook signature."));
    }
    let event: YellowCardEvent = serde_json::from_slice(body)?;
    if !event
        .event
        .as_deref()
        .is_some_and(yellowcard::is_receive_event)
    {
        return Ok(IGNORED);
    }

    // The event is only a trigger; the receive itself is the record.
    let (receive, raw) = yc.get_receive(&event.id).await?;
    if receive.status != "complete" {
        return Ok(IGNORED);
    }
    let row: Option<(Uuid, Uuid)> = sqlx::query_as(
        "select id, customer_id from collections where provider = $1 and provider_ref = $2",
    )
    .bind(YELLOWCARD)
    .bind(&receive.id)
    .fetch_optional(&state.pool)
    .await?;
    let Some((collection_id, customer_id)) = row else {
        tracing::error!(receive = %receive.id, "completed Yellow Card receive matches no collection");
        return Err(ApiError::not_found(
            "No payment request matches that receive.",
        ));
    };
    if receive.currency.as_deref().is_some_and(|c| c != "NGN") {
        return Err(partner_error(
            YELLOWCARD,
            "/business/receive",
            format!(
                "receive {} is in {:?}, not NGN",
                receive.id, receive.currency
            ),
        ));
    }
    let amount_minor = receive
        .converted_amount
        .and_then(major_to_minor)
        .ok_or_else(|| {
            partner_error(
                YELLOWCARD,
                "/business/receive",
                format!("receive {} has no local amount", receive.id),
            )
        })?;

    let applied = apply_inbound(
        state,
        Inbound {
            customer_id,
            collection_id: Some(collection_id),
            settles_closed: true,
            provider: YELLOWCARD,
            provider_payment_id: &receive.id,
            amount_minor,
            currency: CurrencyCode::Ngn,
            payer_name: receive.source.as_ref().and_then(|s| s.account_name.clone()),
            description: None,
            received_at: parse_time(receive.updated_at.as_deref()).unwrap_or_else(Utc::now),
            raw,
        },
    )
    .await?;
    Ok(WebhookAck {
        applied: true,
        duplicate: !applied,
    })
}

/// A request reference in free text, e.g. a wire memo. Banks often drop the
/// hyphen, so `CL2H4F9K` also matches.
fn find_reference(text: &str) -> Option<String> {
    static RE: LazyLock<Regex> =
        LazyLock::new(|| Regex::new(r"\bCL-?([A-Z0-9]{6})\b").expect("valid regex"));
    RE.captures(&text.to_ascii_uppercase())
        .map(|c| format!("CL-{}", &c[1]))
}

async fn receive_bridge(
    state: &AppState,
    headers: &HeaderMap,
    body: &[u8],
) -> ApiResult<WebhookAck> {
    let key = state
        .partners
        .bridge_webhook_key
        .as_ref()
        .ok_or_else(route_not_found)?;
    let signature = headers
        .get("x-webhook-signature")
        .and_then(|v| v.to_str().ok())
        .unwrap_or_default();
    match bridge::verify_webhook(key, signature, body, Utc::now().timestamp_millis()) {
        WebhookCheck::Valid => {}
        WebhookCheck::Invalid => return Err(ApiError::unauthorized("Invalid webhook signature.")),
        WebhookCheck::Stale => return Err(ApiError::validation("Webhook delivery is too old.")),
    }
    let event: bridge::Event = serde_json::from_slice(body)?;
    if event.event_category != "virtual_account.activity" {
        return Ok(IGNORED);
    }
    let activity: bridge::Activity = serde_json::from_value(event.event_object.clone())?;
    // `payment_processed` is the deposit delivered to our destination;
    // earlier activity (funds_received, in_review...) can still reverse.
    if activity.kind != "payment_processed" {
        return Ok(IGNORED);
    }

    let customer_id: Option<Uuid> = sqlx::query_scalar(
        "select customer_id from bridge_virtual_accounts where virtual_account_id = $1",
    )
    .bind(&activity.virtual_account_id)
    .fetch_optional(&state.pool)
    .await?;
    let Some(customer_id) = customer_id else {
        tracing::error!(
            virtual_account = %activity.virtual_account_id,
            event = %event.event_id,
            "Bridge deposit to an unlinked virtual account"
        );
        return Err(ApiError::not_found("Unknown virtual account."));
    };
    let amount_minor = dollars_to_cents(&activity.amount)
        .filter(|cents| *cents > 0)
        .ok_or_else(|| ApiError::validation("Deposit amount is not a positive dollar amount."))?;

    let description = activity.source.as_ref().and_then(|s| s.description.clone());
    let collection_id = match description.as_deref().and_then(find_reference) {
        Some(reference) => {
            sqlx::query_scalar(
                "select id from collections
                  where reference = $1 and customer_id = $2 and provider = $3",
            )
            .bind(reference)
            .bind(customer_id)
            .bind(BRIDGE)
            .fetch_optional(&state.pool)
            .await?
        }
        None => None,
    };
    // One deposit produces several activity events; its deposit id is stable.
    let payment_id = activity
        .deposit_id
        .clone()
        .unwrap_or_else(|| activity.id.clone());

    let applied = apply_inbound(
        state,
        Inbound {
            customer_id,
            collection_id,
            settles_closed: false,
            provider: BRIDGE,
            provider_payment_id: &payment_id,
            amount_minor,
            currency: CurrencyCode::Usd,
            payer_name: activity.source.as_ref().and_then(|s| s.sender_name.clone()),
            description,
            received_at: parse_time(activity.created_at.as_deref()).unwrap_or_else(Utc::now),
            raw: serde_json::from_slice(body)?,
        },
    )
    .await?;
    Ok(WebhookAck {
        applied: true,
        duplicate: !applied,
    })
}

// ---- ops ----

/// Opens a Bridge USD virtual account for a customer who has passed Bridge's
/// KYB (`bridge_customer_id`) and links it. Run by ops via
/// `cargo run --bin link-bridge`; there is no operator role to guard an HTTP
/// route with yet, and linking someone else's account would route their
/// deposits here.
pub async fn link_bridge_account(
    state: &AppState,
    customer_id: Uuid,
    bridge_customer_id: &str,
) -> ApiResult<ReceivingAccount> {
    let bridge = state
        .partners
        .bridge
        .as_ref()
        .ok_or_else(|| ApiError::validation("Set BRIDGE_API_KEY to link Bridge accounts."))?;
    let bridge_customer_id = bridge_customer_id.trim();
    if bridge_customer_id.is_empty() {
        return Err(ApiError::validation("Provide the Bridge customer id."));
    }
    let exists: Option<Uuid> = sqlx::query_scalar("select id from customers where id = $1")
        .bind(customer_id)
        .fetch_optional(&state.pool)
        .await?;
    if exists.is_none() {
        return Err(ApiError::not_found("Customer not found."));
    }
    if bridge_account(&state.pool, customer_id).await?.is_some() {
        return Err(ApiError::conflict(
            "This customer already has a Bridge virtual account.",
        ));
    }

    let (account, _) = bridge
        .create_virtual_account(bridge_customer_id, &format!("kimana-va-{customer_id}"))
        .await?;
    let mut tx = state.pool.begin().await?;
    sqlx::query(
        "insert into bridge_virtual_accounts
           (customer_id, bridge_customer_id, virtual_account_id, deposit_instructions)
         values ($1, $2, $3, $4)",
    )
    .bind(customer_id)
    .bind(bridge_customer_id)
    .bind(&account.id)
    .bind(SqlJson(&account.source_deposit_instructions))
    .execute(&mut *tx)
    .await?;
    write_audit(
        &mut tx,
        AuditEntry {
            actor_id: None,
            actor_role: Some("operator"),
            action: "collection.bridge_link",
            entity_type: "customer",
            entity_id: customer_id.to_string(),
            before: None,
            after: Some(json!({
                "bridgeCustomerId": bridge_customer_id,
                "virtualAccountId": account.id,
                "status": account.status,
            })),
        },
    )
    .await?;
    tx.commit().await?;
    Ok(ReceivingAccount {
        currency: CurrencyCode::Usd,
        pay_in: us_pay_in(&account.source_deposit_instructions, None),
    })
}

// ---- routes ----

pub fn routes() -> Router<AppState> {
    Router::new()
        .route("/collections", post(create).get(list))
        .route("/collections/{id}", get(get_one))
        .route("/collections/{id}/cancel", post(cancel))
        .route("/receiving-accounts", get(accounts))
        .route("/webhooks/yellowcard", post(yellowcard_webhook))
        .route("/webhooks/bridge", post(bridge_webhook))
}

#[derive(Deserialize, ToSchema)]
#[serde(rename_all = "camelCase")]
pub(crate) struct CreateCollectionBody {
    #[serde(default)]
    idempotency_key: Option<String>,
    /// Amount to collect: `NGN` (Yellow Card) or `USD` (Bridge).
    amount: Money,
    /// Who is expected to pay, for the customer's own reference.
    #[serde(default)]
    payer_name: Option<String>,
    #[serde(default)]
    note: Option<String>,
    /// ISO-8601. Defaults to 7 days from now; at most 90. An NGN request
    /// closes sooner if its Yellow Card account does.
    #[serde(default)]
    expires_at: Option<String>,
}

#[utoipa::path(
    post,
    path = "/collections",
    operation_id = "create_collection",
    tag = "collections",
    request_body = CreateCollectionBody,
    params(
        ("Idempotency-Key" = Option<String>, Header, description = "Takes precedence over `idempotencyKey` in the body"),
    ),
    responses(
        (status = 201, description = "Payment request created with pay-in instructions, or the existing one for a replayed idempotency key", body = Collection),
        (status = 400, description = "Validation error, unsupported currency, or no USD receiving account yet", body = ErrorResponse),
        (status = 401, description = "Not authenticated", body = ErrorResponse),
        (status = 502, description = "Payment partner unavailable (PARTNER_FAILURE)", body = ErrorResponse),
    ),
    security(("cookieAuth" = []))
)]
pub(crate) async fn create(
    State(state): State<AppState>,
    session: Session,
    headers: HeaderMap,
    Body(body): Body<CreateCollectionBody>,
) -> ApiResult<(StatusCode, Json<Collection>)> {
    let idempotency_key = headers
        .get("idempotency-key")
        .and_then(|v| v.to_str().ok())
        .map(String::from)
        .or(body.idempotency_key)
        .unwrap_or_default();
    let collection = create_collection(
        &state,
        &session,
        CreateInput {
            idempotency_key,
            amount: body.amount,
            payer_name: body.payer_name,
            note: body.note,
            expires_at: body.expires_at,
        },
    )
    .await?;
    Ok((StatusCode::CREATED, Json(collection)))
}

#[utoipa::path(
    get,
    path = "/collections",
    operation_id = "list_collections",
    tag = "collections",
    responses(
        (status = 200, description = "Payment requests, newest first (at most 100)", body = [Collection]),
        (status = 401, description = "Not authenticated", body = ErrorResponse),
    ),
    security(("cookieAuth" = []))
)]
pub(crate) async fn list(
    State(state): State<AppState>,
    session: Session,
) -> ApiResult<Json<Vec<Collection>>> {
    Ok(Json(list_collections(&state, &session).await?))
}

#[utoipa::path(
    get,
    path = "/collections/{id}",
    operation_id = "get_collection",
    tag = "collections",
    params(("id" = String, Path, description = "Collection id")),
    responses(
        (status = 200, description = "Payment request", body = Collection),
        (status = 401, description = "Not authenticated", body = ErrorResponse),
        (status = 404, description = "Payment request not found", body = ErrorResponse),
    ),
    security(("cookieAuth" = []))
)]
pub(crate) async fn get_one(
    State(state): State<AppState>,
    session: Session,
    Path(id): Path<String>,
) -> ApiResult<Json<Collection>> {
    Ok(Json(get_collection(&state, &session, &id).await?))
}

#[utoipa::path(
    post,
    path = "/collections/{id}/cancel",
    operation_id = "cancel_collection",
    tag = "collections",
    params(("id" = String, Path, description = "Collection id")),
    responses(
        (status = 200, description = "Payment request cancelled (or already cancelled)", body = Collection),
        (status = 401, description = "Not authenticated", body = ErrorResponse),
        (status = 404, description = "Payment request not found", body = ErrorResponse),
        (status = 409, description = "Already paid or expired", body = ErrorResponse),
        (status = 502, description = "Yellow Card refused or is unavailable; the request stays open", body = ErrorResponse),
    ),
    security(("cookieAuth" = []))
)]
pub(crate) async fn cancel(
    State(state): State<AppState>,
    session: Session,
    Path(id): Path<String>,
) -> ApiResult<Json<Collection>> {
    Ok(Json(cancel_collection(&state, &session, &id).await?))
}

#[utoipa::path(
    get,
    path = "/receiving-accounts",
    operation_id = "receiving_accounts",
    tag = "collections",
    responses(
        (status = 200, description = "Standing accounts the customer can be paid into (USD once linked); every deposit is credited", body = [ReceivingAccount]),
        (status = 401, description = "Not authenticated", body = ErrorResponse),
    ),
    security(("cookieAuth" = []))
)]
pub(crate) async fn accounts(
    State(state): State<AppState>,
    session: Session,
) -> ApiResult<Json<Vec<ReceivingAccount>>> {
    Ok(Json(receiving_accounts(&state, &session).await?))
}

#[utoipa::path(
    post,
    path = "/webhooks/yellowcard",
    operation_id = "yellowcard_webhook",
    tag = "collections",
    request_body(content = Object, content_type = "application/json", description = "Yellow Card webhook event (`id`, `event`, `status`, `sequenceId`...), verified against the raw body"),
    params(
        ("X-YC-Signature" = String, Header, description = "Base64 HMAC-SHA256 of the raw body under the Yellow Card API secret"),
    ),
    responses(
        (status = 200, description = "Receive credited, already credited, or event ignored", body = WebhookAck),
        (status = 401, description = "Invalid signature", body = ErrorResponse),
        (status = 404, description = "Receive matches no collection, or Yellow Card isn't configured", body = ErrorResponse),
        (status = 502, description = "Couldn't fetch the receive from Yellow Card", body = ErrorResponse),
    )
)]
pub(crate) async fn yellowcard_webhook(
    State(state): State<AppState>,
    headers: HeaderMap,
    body: Bytes,
) -> ApiResult<Json<WebhookAck>> {
    Ok(Json(receive_yellowcard(&state, &headers, &body).await?))
}

#[utoipa::path(
    post,
    path = "/webhooks/bridge",
    operation_id = "bridge_webhook",
    tag = "collections",
    request_body(content = Object, content_type = "application/json", description = "Bridge webhook event (`event_id`, `event_category`, `event_object`...), verified against the raw body"),
    params(
        ("X-Webhook-Signature" = String, Header, description = "`t=<unix ms>,v0=<base64 RSA signature>`, verified with BRIDGE_WEBHOOK_PUBLIC_KEY"),
    ),
    responses(
        (status = 200, description = "Deposit credited, already credited, or event ignored", body = WebhookAck),
        (status = 400, description = "Stale delivery or malformed event", body = ErrorResponse),
        (status = 401, description = "Invalid signature", body = ErrorResponse),
        (status = 404, description = "Unlinked virtual account, or Bridge webhooks aren't configured", body = ErrorResponse),
    )
)]
pub(crate) async fn bridge_webhook(
    State(state): State<AppState>,
    headers: HeaderMap,
    body: Bytes,
) -> ApiResult<Json<WebhookAck>> {
    Ok(Json(receive_bridge(&state, &headers, &body).await?))
}

#[cfg(test)]
mod tests {
    use super::find_reference;

    #[test]
    fn finds_references_in_memos() {
        assert_eq!(
            find_reference("INV 114 ref CL-2H4F9K thanks").as_deref(),
            Some("CL-2H4F9K")
        );
        assert_eq!(find_reference("cl2h4f9k").as_deref(), Some("CL-2H4F9K"));
        assert_eq!(find_reference("XCL-2H4F9K"), None);
        assert_eq!(find_reference("CL-2H4F9KZ"), None);
        assert_eq!(find_reference("no reference"), None);
    }
}
