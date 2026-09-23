//! Firm quotes.

use crate::contract::common::{CurrencyCode, Money};
use crate::contract::quote::{CostBreakdown, FirmQuote, QuoteAmountField, RateSource};
use crate::domain::fx;
use crate::error::{ApiError, ApiResult, ErrorResponse};
use crate::http::{Body, Session};
use crate::state::AppState;
use crate::util::{apply_rate, invert_rate, is_uuid, iso};
use axum::extract::State;
use axum::http::StatusCode;
use axum::routing::post;
use axum::{Json, Router};
use chrono::{DateTime, Utc};
use serde::Deserialize;
use sqlx::PgPool;
use utoipa::ToSchema;
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
                // The `quotes` table doesn't persist which source produced
                // the rate — only `request_firm_quote`, at creation time,
                // knows that (and sets it directly on the value it returns,
                // not through this reconstruction path).
                source: RateSource::default(),
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

/// Columns every read/write below selects or returns. A macro (not a `const
/// &str`) so `concat!` can splice it into each full query as a compile-time
/// string literal — see ISSUE-BE-06: no runtime `format!` for SQL text, only
/// static strings with `$n` bind parameters.
macro_rules! quote_cols {
    () => {
        "id, customer_id, send_currency, receive_currency, rate, fee_minor,
                    send_amount_minor, receive_amount_minor, issued_at, expires_at"
    };
}

const SELECT_BY_ID: &str = concat!("select ", quote_cols!(), " from quotes where id = $1");
const INSERT_QUOTE: &str = concat!(
    "insert into quotes
       (customer_id, send_currency, receive_currency, rate, fee_minor,
        send_amount_minor, receive_amount_minor, expires_at)
     values ($1, $2, $3, $4, 0, $5, $6, now() + ($7 || ' seconds')::interval)
     returning ",
    quote_cols!()
);

pub async fn find_by_id(pool: &PgPool, id: &str) -> ApiResult<Option<StoredQuote>> {
    if !is_uuid(id) {
        return Ok(None);
    }
    let row: Option<QuoteRow> = sqlx::query_as(SELECT_BY_ID)
        .bind(Uuid::parse_str(id).unwrap())
        .fetch_optional(pool)
        .await?;
    row.map(QuoteRow::into_stored).transpose()
}

/// Atomically claims `quote_id` for `transfer_id`. Returns false when
/// another transfer already claimed it — a concurrent create_transfer race
/// on the same quote_id — in which case the caller must roll back and
/// reject with 409 Conflict rather than leave a transfer that can never
/// settle.
pub async fn try_consume(
    conn: &mut sqlx::PgConnection,
    quote_id: Uuid,
    transfer_id: Uuid,
) -> ApiResult<bool> {
    let rows_affected = sqlx::query(
        "update quotes
            set consumed_by_transfer_id = $2
          where id = $1 and consumed_by_transfer_id is null",
    )
    .bind(quote_id)
    .bind(transfer_id)
    .execute(conn)
    .await?
    .rows_affected();
    Ok(rows_affected > 0)
}

// ---- request body ----

#[derive(Deserialize, ToSchema)]
#[serde(rename_all = "camelCase")]
pub(crate) struct RequestFirmQuoteBody {
    send_currency: String,
    receive_currency: String,
    amount: Money,
    amount_field: QuoteAmountField,
}

/// Both currencies use minor-unit exponent 2, so the minor-unit ratio equals
/// the quoted major-unit rate — same shortcut the frontend mock takes.
///
/// Uses fixed-point integer math (`apply_rate`/`invert_rate`), not
/// `f64::round()` — see ISSUE-BE-02. `.round()` rounds a `.5` fractional
/// product up, which disagrees with the floor division settlement math on
/// the other side of a quote uses, and would fail reconciliation there.
fn derive_amounts(field: QuoteAmountField, amount_minor: i64, rate: f64) -> (i64, i64) {
    match field {
        QuoteAmountField::Send => (amount_minor, apply_rate(amount_minor, rate)),
        QuoteAmountField::Receive => (invert_rate(amount_minor, rate), amount_minor),
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

    let row: QuoteRow = sqlx::query_as(INSERT_QUOTE)
        .bind(session.customer_id)
        .bind(send.as_str())
        .bind(receive.as_str())
        .bind(indicative.rate)
        .bind(send_minor)
        .bind(receive_minor)
        .bind(state.config.quote_ttl_seconds.to_string())
        .fetch_one(&state.pool)
        .await?;

    let mut firm_quote = row.into_stored()?.firm_quote;
    firm_quote.breakdown.source = indicative.source;
    Ok(firm_quote)
}

// ---- routes ----

pub fn routes() -> Router<AppState> {
    Router::new().route("/quotes", post(create))
}

#[utoipa::path(
    post,
    path = "/quotes",
    operation_id = "create_quote",
    tag = "quotes",
    request_body = RequestFirmQuoteBody,
    responses(
        (status = 201, description = "Firm quote, valid until `expiresAt`", body = FirmQuote),
        (status = 400, description = "Validation error or no rate for the pair", body = ErrorResponse),
        (status = 401, description = "Not authenticated", body = ErrorResponse),
        (status = 502, description = "Rate feed unavailable (PARTNER_FAILURE)", body = ErrorResponse),
    ),
    security(("cookieAuth" = []))
)]
pub(crate) async fn create(
    State(state): State<AppState>,
    session: Session,
    Body(body): Body<RequestFirmQuoteBody>,
) -> ApiResult<(StatusCode, Json<FirmQuote>)> {
    let quote = request_firm_quote(&state, &session, body).await?;
    Ok((StatusCode::CREATED, Json(quote)))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn no_float_round_in_money_math() {
        // Same boundary case as util::rate_math_tests::floors_instead_of_rounding_half_up,
        // exercised through derive_amounts directly.
        let (_, receive) = derive_amounts(QuoteAmountField::Send, 1, 0.5);
        assert_eq!(receive, 0, "must floor, not round, a .5 fractional product");
    }

    #[test]
    fn send_and_receive_fields_are_inverses() {
        let rate = 1645.2;
        let (send, receive) = derive_amounts(QuoteAmountField::Send, 4_500_000, rate);
        assert_eq!(send, 4_500_000);
        assert_eq!(receive, 7_403_400_000);

        let (send2, receive2) = derive_amounts(QuoteAmountField::Receive, receive, rate);
        assert_eq!(receive2, receive);
        assert_eq!(send2, send);
    }
}
