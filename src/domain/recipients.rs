//! Recipients: list, name-resolution (stub), save.

use crate::audit::{write_audit, AuditEntry};
use crate::contract::common::CurrencyCode;
use crate::contract::transfer::Recipient;
use crate::error::{ApiError, ApiResult};
use crate::http::{Body, Session};
use crate::state::AppState;
use crate::util::{is_uuid, iso};
use axum::extract::State;
use axum::http::StatusCode;
use axum::routing::{get, post};
use axum::{Json, Router};
use chrono::{DateTime, Utc};
use serde::Deserialize;
use serde_json::json;
use sqlx::PgPool;
use uuid::Uuid;

const DEFAULT_BANK_NAME: &str = "Partner Bank";

#[derive(sqlx::FromRow)]
struct RecipientRow {
    id: Uuid,
    customer_id: Uuid,
    account_name: String,
    account_number: String,
    bank_code: String,
    bank_name: String,
    currency: String,
    country: String,
    validation_status: String,
    saved_at: DateTime<Utc>,
}

impl RecipientRow {
    fn into_contract(self) -> ApiResult<Recipient> {
        Ok(Recipient {
            id: self.id.to_string(),
            customer_id: self.customer_id.to_string(),
            account_name: self.account_name,
            account_number: self.account_number,
            bank_code: self.bank_code,
            bank_name: self.bank_name,
            currency: CurrencyCode::parse(&self.currency)?,
            country: self.country,
            validation_status: self.validation_status,
            saved_at: iso(self.saved_at),
        })
    }
}

const COLS: &str = "id, customer_id, account_name, account_number, bank_code,
                    bank_name, currency, country, validation_status, saved_at";

pub async fn list_by_customer(pool: &PgPool, customer_id: Uuid) -> ApiResult<Vec<Recipient>> {
    let rows: Vec<RecipientRow> = sqlx::query_as(&format!(
        "select {COLS} from recipients where customer_id = $1 order by saved_at desc"
    ))
    .bind(customer_id)
    .fetch_all(pool)
    .await?;
    rows.into_iter().map(RecipientRow::into_contract).collect()
}

pub async fn find_by_id(
    pool: &PgPool,
    id: &str,
    customer_id: Uuid,
) -> ApiResult<Option<Recipient>> {
    if !is_uuid(id) {
        return Ok(None);
    }
    let row: Option<RecipientRow> = sqlx::query_as(&format!(
        "select {COLS} from recipients where id = $1 and customer_id = $2"
    ))
    .bind(Uuid::parse_str(id).unwrap())
    .bind(customer_id)
    .fetch_optional(pool)
    .await?;
    row.map(RecipientRow::into_contract).transpose()
}

// ---- request bodies ----

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct NewRecipientBody {
    account_number: String,
    bank_code: String,
    currency: String,
    country: String,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct SaveRecipientBody {
    account_number: String,
    bank_code: String,
    currency: String,
    country: String,
    account_name: String,
}

fn validate_new(b: &NewRecipientBody) -> ApiResult<CurrencyCode> {
    if b.account_number.trim().len() < 4 {
        return Err(ApiError::validation(
            "Enter the destination account number.",
        ));
    }
    if b.bank_code.trim().is_empty() {
        return Err(ApiError::validation("Select the destination bank."));
    }
    if b.country.trim().len() != 2 {
        return Err(ApiError::validation("Use a 2-letter ISO country code."));
    }
    CurrencyCode::parse(b.currency.trim())
}

/// Deterministic stand-in for the payout partner's name-lookup API.
fn resolve_account_name(account_number: &str) -> String {
    let tail: String = account_number
        .chars()
        .rev()
        .take(4)
        .collect::<Vec<_>>()
        .into_iter()
        .rev()
        .collect();
    format!("Verified Beneficiary ({tail})")
}

// ---- routes ----

pub fn routes() -> Router<AppState> {
    Router::new()
        .route("/recipients", get(list).post(save))
        .route("/recipients/validate", post(validate))
}

async fn list(State(state): State<AppState>, session: Session) -> ApiResult<Json<Vec<Recipient>>> {
    Ok(Json(
        list_by_customer(&state.pool, session.customer_id).await?,
    ))
}

async fn validate(
    _session: Session,
    Body(body): Body<NewRecipientBody>,
) -> ApiResult<Json<serde_json::Value>> {
    validate_new(&body)?;
    Ok(Json(
        json!({ "accountName": resolve_account_name(&body.account_number) }),
    ))
}

async fn save(
    State(state): State<AppState>,
    session: Session,
    Body(body): Body<SaveRecipientBody>,
) -> ApiResult<(StatusCode, Json<Recipient>)> {
    let currency = validate_new(&NewRecipientBody {
        account_number: body.account_number.clone(),
        bank_code: body.bank_code.clone(),
        currency: body.currency.clone(),
        country: body.country.clone(),
    })?;
    if body.account_name.trim().is_empty() {
        return Err(ApiError::validation("Confirm the account holder name."));
    }

    let mut tx = state.pool.begin().await?;
    let row: RecipientRow = sqlx::query_as(&format!(
        "insert into recipients
           (customer_id, account_name, account_number, bank_code, bank_name, currency, country, validation_status)
         values ($1, $2, $3, $4, $5, $6, $7, 'valid')
         returning {COLS}"
    ))
    .bind(session.customer_id)
    .bind(body.account_name.trim())
    .bind(body.account_number.trim())
    .bind(body.bank_code.trim())
    .bind(DEFAULT_BANK_NAME)
    .bind(currency.as_str())
    .bind(body.country.trim())
    .fetch_one(&mut *tx)
    .await?;

    let recipient = row.into_contract()?;
    write_audit(
        &mut tx,
        AuditEntry {
            actor_id: Some(session.user_id),
            actor_role: Some(session.role_str()),
            action: "recipient.saved",
            entity_type: "recipient",
            entity_id: recipient.id.clone(),
            before: None,
            after: Some(serde_json::to_value(&recipient)?),
        },
    )
    .await?;
    tx.commit().await?;

    Ok((StatusCode::CREATED, Json(recipient)))
}
