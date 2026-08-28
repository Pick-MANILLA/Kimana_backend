//! Firm quotes.

use crate::contract::common::{CurrencyCode, Money};
use crate::contract::quote::{CostBreakdown, FirmQuote, QuoteAmountField};
use crate::domain::fx;
use crate::error::{ApiError, ApiResult};
use crate::http::{Body, Session};
use crate::state::AppState;
use crate::util::{is_uuid, iso};
use axum::extract::State;
use axum::http::StatusCode;
use axum::routing::post;
use axum::{Json, Router};
use chrono::{DateTime, Utc};
use serde::Deserialize;
use sqlx::PgPool;
use uuid::Uuid;

#[derive(sqlx::FromRow)]
struct QuoteRow {
    id: Uuid,
    customer_id: Uuid,
    send_currency: String,
    receive_currency: String,
    rate: f64,
    fee_minor: i64,
    send_amount_minor: i64,
    receive_amount_minor: i64,
    issued_at: DateTime<Utc>,
    expires_at: DateTime<Utc>,
}

pub struct StoredQuote {
    pub id: Uuid,
    pub customer_id: Uuid,
    pub firm_quote: FirmQuote,
    pub expires_at: DateTime<Utc>,
}

impl QuoteRow {
    fn into_stored(self) -> ApiResult<StoredQuote> {
        let send = CurrencyCode::parse(&self.send_currency)?;
        let receive = CurrencyCode::parse(&self.receive_currency)?;
        let firm_quote = FirmQuote {
            id: self.id.to_string(),
            send_currency: send,
            receive_currency: receive,
            breakdown: CostBreakdown {
                rate: self.rate,
                fee: Money::new(self.fee_minor, send),
                send_amount: Money::new(self.send_amount_minor, send),
                receive_amount: Money::new(self.receive_amount_minor, receive),
            },
            issued_at: iso(self.issued_at),
            expires_at: iso(self.expires_at),
        };
        Ok(StoredQuote {
            id: self.id,
            customer_id: self.customer_id,
            firm_quote,
            expires_at: self.expires_at,
        })
    }
}

const COLS: &str = "id, customer_id, send_currency, receive_currency, rate, fee_minor,
                    send_amount_minor, receive_amount_minor, issued_at, expires_at";

pub async fn find_by_id(pool: &PgPool, id: &str) -> ApiResult<Option<StoredQuote>> {
    if !is_uuid(id) {
        return Ok(None);
    }
    let row: Option<QuoteRow> = sqlx::query_as(&format!("select {COLS} from quotes where id = $1"))
        .bind(Uuid::parse_str(id).unwrap())
        .fetch_optional(pool)
        .await?;
    row.map(QuoteRow::into_stored).transpose()
}

// ---- request body ----

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct RequestFirmQuoteBody {
    send_currency: String,
    receive_currency: String,
    amount: Money,
    amount_field: QuoteAmountField,
}

/// Both currencies use minor-unit exponent 2, so the minor-unit ratio equals
/// the quoted major-unit rate — same shortcut the frontend mock takes.
fn derive_amounts(field: QuoteAmountField, amount_minor: i64, rate: f64) -> (i64, i64) {
    match field {
        QuoteAmountField::Send => (amount_minor, (amount_minor as f64 * rate).round() as i64),
        QuoteAmountField::Receive => ((amount_minor as f64 / rate).round() as i64, amount_minor),
    }
}

async fn request_firm_quote(
    state: &AppState,
    session: &Session,
    body: RequestFirmQuoteBody,
) -> ApiResult<FirmQuote> {
    let send = CurrencyCode::parse(body.send_currency.trim())?;
    let receive = CurrencyCode::parse(body.receive_currency.trim())?;
    if send == receive {
        return Err(ApiError::validation(
            "Send and receive currencies must differ.",
        ));
    }
    if body.amount.amount_minor <= 0 {
        return Err(ApiError::validation("Enter an amount greater than zero."));
    }
    let expected = match body.amount_field {
        QuoteAmountField::Send => send,
        QuoteAmountField::Receive => receive,
    };
    if body.amount.currency != expected {
        return Err(ApiError::validation(format!(
            "Amount currency must be {} when entered on the {:?} side.",
            expected.as_str(),
            body.amount_field
        )));
    }

    let indicative = fx::get_indicative_rate(state, send, receive).await?;
    let (send_minor, receive_minor) =
        derive_amounts(body.amount_field, body.amount.amount_minor, indicative.rate);

    let row: QuoteRow = sqlx::query_as(&format!(
        "insert into quotes
           (customer_id, send_currency, receive_currency, rate, fee_minor,
            send_amount_minor, receive_amount_minor, expires_at)
         values ($1, $2, $3, $4, 0, $5, $6, now() + ($7 || ' seconds')::interval)
         returning {COLS}"
    ))
    .bind(session.customer_id)
    .bind(send.as_str())
    .bind(receive.as_str())
    .bind(indicative.rate)
    .bind(send_minor)
    .bind(receive_minor)
    .bind(state.config.quote_ttl_seconds.to_string())
    .fetch_one(&state.pool)
    .await?;

    Ok(row.into_stored()?.firm_quote)
}

// ---- routes ----

pub fn routes() -> Router<AppState> {
    Router::new().route("/quotes", post(create))
}

async fn create(
    State(state): State<AppState>,
    session: Session,
    Body(body): Body<RequestFirmQuoteBody>,
) -> ApiResult<(StatusCode, Json<FirmQuote>)> {
    let quote = request_firm_quote(&state, &session, body).await?;
    Ok((StatusCode::CREATED, Json(quote)))
}
