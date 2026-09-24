//! Settlement wallet: buy USDC with a local currency, convert it back, read
//! the balance and the trade history (issue #77).
//!
//! This is the one customer surface that names USDC; everything else keeps
//! the settlement asset hidden (`ledger::get_balances` leaves the account
//! out). The SettlementVault holds pooled USDC and only moves it to or from
//! partners, so the customer's USDC is a ledger account and a trade is two
//! ledger legs at the indicative rate. On-chain movement stays with the
//! transfer lifecycle (`crate::settlement`), where partners fund and settle.

use crate::audit::{write_audit, AuditEntry};
use crate::contract::common::{CurrencyCode, Money};
use crate::contract::quote::RateSource;
use crate::domain::{fx, ledger};
use crate::error::{ApiError, ApiResult, ErrorResponse};
use crate::http::{Body, Session};
use crate::state::AppState;
use crate::util::{apply_rate, invert_rate, iso, tagged_reference};
use axum::extract::State;
use axum::http::{HeaderMap, StatusCode};
use axum::routing::{get, post};
use axum::{Json, Router};
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use serde_json::json;
use sqlx::PgPool;
use utoipa::ToSchema;
use uuid::Uuid;

const ASSET: &str = "USDC";
/// The wallet's USDC is held in cents, like USD.
const ASSET_DECIMALS: u8 = 2;
const HISTORY_LIMIT: i64 = 100;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, ToSchema)]
#[serde(rename_all = "SCREAMING_SNAKE_CASE")]
pub enum TradeKind {
    /// Local currency in, USDC out.
    Buy,
    /// USDC in, local currency out.
    Convert,
}

impl TradeKind {
    fn as_str(self) -> &'static str {
        match self {
            TradeKind::Buy => "BUY",
            TradeKind::Convert => "CONVERT",
        }
    }

    fn parse(value: &str) -> ApiResult<Self> {
        match value {
            "BUY" => Ok(TradeKind::Buy),
            "CONVERT" => Ok(TradeKind::Convert),
            _ => Err(ApiError::server_error()),
        }
    }
}

/// One buy or convert: the wallet's "coin transaction" record.
#[derive(Debug, Clone, Serialize, ToSchema)]
#[serde(rename_all = "camelCase")]
pub struct SettlementTrade {
    pub id: String,
    /// e.g. `ST-2H4F9K`
    pub reference: String,
    pub kind: TradeKind,
    /// Always `USDC`.
    pub asset: String,
    /// USDC in cents (1 USDC = 100).
    pub usdc_amount_minor: i64,
    pub local_amount: Money,
    /// Local-currency major units per 1 USDC.
    pub rate: f64,
    pub rate_source: RateSource,
    pub created_at: String,
}

/// The chain the vault lives on, for display. Present only when
/// `SETTLEMENT_VAULT_ADDRESS` and `SETTLEMENT_CHAIN_ID` are both set.
#[derive(Debug, Clone, Serialize, ToSchema)]
#[serde(rename_all = "camelCase")]
pub struct SettlementNetwork {
    pub chain_id: u64,
    pub vault_address: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub asset_address: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub explorer_url: Option<String>,
}

#[derive(Debug, Clone, Serialize, ToSchema)]
#[serde(rename_all = "camelCase")]
pub struct SettlementBalance {
    /// Always `USDC`.
    pub asset: String,
    /// USDC in cents (1 USDC = 100).
    pub amount_minor: i64,
    /// Always 2: `amountMinor` is in cents.
    pub decimals: u8,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub network: Option<SettlementNetwork>,
    pub as_of: String,
}

#[derive(sqlx::FromRow)]
struct TradeRow {
    id: Uuid,
    reference: String,
    kind: String,
    usdc_amount_minor: i64,
    local_currency: String,
    local_amount_minor: i64,
    rate: f64,
    rate_source: String,
    created_at: DateTime<Utc>,
}

impl TradeRow {
    fn into_trade(self) -> ApiResult<SettlementTrade> {
        let currency = CurrencyCode::parse(&self.local_currency)?;
        Ok(SettlementTrade {
            id: self.id.to_string(),
            reference: self.reference,
            kind: TradeKind::parse(&self.kind)?,
            asset: ASSET.to_string(),
            usdc_amount_minor: self.usdc_amount_minor,
            local_amount: Money::new(self.local_amount_minor, currency),
            rate: self.rate,
            rate_source: match self.rate_source.as_str() {
                "cachedProvisional" => RateSource::CachedProvisional,
                _ => RateSource::Live,
            },
            created_at: iso(self.created_at),
        })
    }
}

macro_rules! trade_cols {
    () => {
        "id, reference, kind, usdc_amount_minor, local_currency, local_amount_minor,
                rate, rate_source, created_at"
    };
}

const SELECT_BY_KEY: &str = concat!(
    "select ",
    trade_cols!(),
    " from settlement_trades where customer_id = $1 and idempotency_key = $2"
);
const SELECT_BY_CUSTOMER: &str = concat!(
    "select ",
    trade_cols!(),
    " from settlement_trades where customer_id = $1 order by created_at desc, id limit $2"
);
const INSERT_TRADE: &str = concat!(
    "insert into settlement_trades
       (reference, customer_id, idempotency_key, kind, usdc_amount_minor,
        local_currency, local_amount_minor, rate, rate_source, created_by)
     values ($1, $2, $3, $4, $5, $6, $7, $8, $9, $10)
     on conflict (customer_id, idempotency_key) do nothing
     returning ",
    trade_cols!()
);

async fn find_by_key(
    pool: &PgPool,
    customer_id: Uuid,
    key: &str,
) -> ApiResult<Option<SettlementTrade>> {
    let row: Option<TradeRow> = sqlx::query_as(SELECT_BY_KEY)
        .bind(customer_id)
        .bind(key)
        .fetch_optional(pool)
        .await?;
    row.map(TradeRow::into_trade).transpose()
}

// ---- rates ----

struct WalletRate {
    rate: f64,
    source: RateSource,
}

/// Local-currency units per 1 USDC. USDC is treated as USD 1:1, so this is
/// the indicative `USD/<local>` rate, and exactly 1 for USD itself.
async fn usdc_rate(state: &AppState, local: CurrencyCode) -> ApiResult<WalletRate> {
    if local == CurrencyCode::Usd {
        return Ok(WalletRate {
            rate: 1.0,
            source: RateSource::Live,
        });
    }
    let indicative = fx::get_indicative_rate(state, CurrencyCode::Usd, local)
        .await
        .map_err(|err| match err.code {
            crate::error::ErrorCode::Validation => ApiError::validation(format!(
                "USDC can't be traded against {} yet.",
                local.as_str()
            )),
            _ => err,
        })?;
    Ok(WalletRate {
        rate: indicative.rate,
        source: indicative.source,
    })
}

// ---- trades ----

struct TradeInput {
    kind: TradeKind,
    idempotency_key: String,
    local_currency: CurrencyCode,
    /// Minor units of whichever side the customer entered: local for a buy,
    /// USDC cents for a convert.
    amount_minor: i64,
}

async fn execute_trade(
    state: &AppState,
    session: &Session,
    input: TradeInput,
) -> ApiResult<SettlementTrade> {
    if input.idempotency_key.trim().len() < 8 {
        return Err(ApiError::validation("Provide a stable idempotency key."));
    }
    if let Some(existing) =
        find_by_key(&state.pool, session.customer_id, &input.idempotency_key).await?
    {
        return Ok(existing);
    }
    if input.amount_minor <= 0 {
        return Err(ApiError::validation("Enter an amount greater than zero."));
    }

    let local = input.local_currency;
    let rate = usdc_rate(state, local).await?;
    // Both directions floor, so rounding never creates value.
    let (usdc_minor, local_minor) = match input.kind {
        TradeKind::Buy => (
            invert_rate(input.amount_minor, rate.rate),
            input.amount_minor,
        ),
        TradeKind::Convert => (
            input.amount_minor,
            apply_rate(input.amount_minor, rate.rate),
        ),
    };
    if usdc_minor <= 0 || local_minor <= 0 {
        return Err(ApiError::validation(
            "That amount is too small to convert at the current rate.",
        ));
    }

    let mut tx = state.pool.begin().await?;
    let local_acct = ledger::get_or_create_account(&mut tx, session.customer_id, local).await?;
    let usdc_acct =
        ledger::get_or_create_account_by_code(&mut tx, session.customer_id, ledger::USDC).await?;

    let (debit_acct, debit_minor, debit_label) = match input.kind {
        TradeKind::Buy => (local_acct, local_minor, local.as_str()),
        TradeKind::Convert => (usdc_acct, usdc_minor, ASSET),
    };
    // Row lock first, so a concurrent trade can't spend the same balance.
    sqlx::query("select id from accounts where id = $1 for update")
        .bind(debit_acct)
        .execute(&mut *tx)
        .await?;
    if ledger::account_balance_minor(&mut tx, debit_acct).await? < debit_minor {
        return Err(ApiError::validation(format!(
            "Insufficient {debit_label} balance."
        )));
    }

    let rate_source = match rate.source {
        RateSource::Live => "live",
        RateSource::CachedProvisional => "cachedProvisional",
    };
    let inserted: Option<TradeRow> = sqlx::query_as(INSERT_TRADE)
        .bind(tagged_reference("ST"))
        .bind(session.customer_id)
        .bind(&input.idempotency_key)
        .bind(input.kind.as_str())
        .bind(usdc_minor)
        .bind(local.as_str())
        .bind(local_minor)
        .bind(rate.rate)
        .bind(rate_source)
        .bind(session.user_id)
        .fetch_optional(&mut *tx)
        .await?;
    let Some(row) = inserted else {
        // A concurrent request with the same key committed first.
        tx.rollback().await?;
        return find_by_key(&state.pool, session.customer_id, &input.idempotency_key)
            .await?
            .ok_or_else(ApiError::server_error);
    };
    let trade_id = row.id;

    let (local_signed, usdc_signed, description) = match input.kind {
        TradeKind::Buy => (-local_minor, usdc_minor, "USDC purchase"),
        TradeKind::Convert => (local_minor, -usdc_minor, "USDC conversion"),
    };
    ledger::post_settlement_entry(
        &mut tx,
        ledger::SettlementPosting {
            account_id: local_acct,
            trade_id,
            amount_minor: local_signed,
            currency: local.as_str(),
            description,
        },
    )
    .await?;
    let (_, usdc_balance) = ledger::post_settlement_entry(
        &mut tx,
        ledger::SettlementPosting {
            account_id: usdc_acct,
            trade_id,
            amount_minor: usdc_signed,
            currency: ledger::USDC,
            description,
        },
    )
    .await?;

    write_audit(
        &mut tx,
        AuditEntry {
            actor_id: Some(session.user_id),
            actor_role: Some(session.role_str()),
            action: match input.kind {
                TradeKind::Buy => "settlement.buy",
                TradeKind::Convert => "settlement.convert",
            },
            entity_type: "settlement_trade",
            entity_id: trade_id.to_string(),
            before: None,
            after: Some(json!({
                "reference": row.reference,
                "usdcAmountMinor": usdc_minor,
                "localCurrency": local.as_str(),
                "localAmountMinor": local_minor,
                "rate": rate.rate,
                "rateSource": rate_source,
                "usdcBalanceMinor": usdc_balance,
            })),
        },
    )
    .await?;
    tx.commit().await?;

    row.into_trade()
}

async fn get_balance(state: &AppState, session: &Session) -> ApiResult<SettlementBalance> {
    let amount_minor: i64 = sqlx::query_scalar(
        "select coalesce(sum(le.amount_minor), 0)::bigint
           from accounts a
           join ledger_entries le on le.account_id = a.id
          where a.customer_id = $1 and a.currency = $2",
    )
    .bind(session.customer_id)
    .bind(ledger::USDC)
    .fetch_one(&state.pool)
    .await?;
    Ok(SettlementBalance {
        asset: ASSET.to_string(),
        amount_minor,
        decimals: ASSET_DECIMALS,
        network: network(state),
        as_of: iso(Utc::now()),
    })
}

fn network(state: &AppState) -> Option<SettlementNetwork> {
    let c = &state.config;
    Some(SettlementNetwork {
        chain_id: c.settlement_chain_id?,
        vault_address: c.settlement_vault_address.clone()?,
        asset_address: c.settlement_usdc_address.clone(),
        explorer_url: c.settlement_explorer_url.clone(),
    })
}

async fn list_trades(state: &AppState, session: &Session) -> ApiResult<Vec<SettlementTrade>> {
    let rows: Vec<TradeRow> = sqlx::query_as(SELECT_BY_CUSTOMER)
        .bind(session.customer_id)
        .bind(HISTORY_LIMIT)
        .fetch_all(&state.pool)
        .await?;
    rows.into_iter().map(TradeRow::into_trade).collect()
}

// ---- routes ----

pub fn routes() -> Router<AppState> {
    Router::new()
        .route("/settlement/balance", get(balance))
        .route("/settlement/buy", post(buy))
        .route("/settlement/convert", post(convert))
        .route("/settlement/transactions", get(transactions))
}

/// `Idempotency-Key` header, falling back to the body field.
fn idempotency_key(headers: &HeaderMap, body_key: Option<String>) -> String {
    headers
        .get("idempotency-key")
        .and_then(|v| v.to_str().ok())
        .map(String::from)
        .or(body_key)
        .unwrap_or_default()
}

#[derive(Deserialize, ToSchema)]
#[serde(rename_all = "camelCase")]
pub(crate) struct BuyBody {
    #[serde(default)]
    idempotency_key: Option<String>,
    /// Local-currency amount to spend.
    amount: Money,
}

#[derive(Deserialize, ToSchema)]
#[serde(rename_all = "camelCase")]
pub(crate) struct ConvertBody {
    #[serde(default)]
    idempotency_key: Option<String>,
    /// USDC to convert, in cents (1 USDC = 100).
    usdc_amount_minor: i64,
    /// Local currency to receive, e.g. `NGN`.
    currency: String,
}

#[utoipa::path(
    get,
    path = "/settlement/balance",
    operation_id = "settlement_balance",
    tag = "settlement",
    responses(
        (status = 200, description = "USDC settlement balance", body = SettlementBalance),
        (status = 401, description = "Not authenticated", body = ErrorResponse),
    ),
    security(("cookieAuth" = []))
)]
pub(crate) async fn balance(
    State(state): State<AppState>,
    session: Session,
) -> ApiResult<Json<SettlementBalance>> {
    Ok(Json(get_balance(&state, &session).await?))
}

#[utoipa::path(
    post,
    path = "/settlement/buy",
    operation_id = "settlement_buy",
    tag = "settlement",
    request_body = BuyBody,
    params(
        ("Idempotency-Key" = Option<String>, Header, description = "Takes precedence over `idempotencyKey` in the body"),
    ),
    responses(
        (status = 201, description = "USDC bought, or the existing trade for a replayed idempotency key", body = SettlementTrade),
        (status = 400, description = "Validation error, insufficient balance or unsupported currency", body = ErrorResponse),
        (status = 401, description = "Not authenticated", body = ErrorResponse),
        (status = 502, description = "Rate feed unavailable (PARTNER_FAILURE)", body = ErrorResponse),
    ),
    security(("cookieAuth" = []))
)]
pub(crate) async fn buy(
    State(state): State<AppState>,
    session: Session,
    headers: HeaderMap,
    Body(body): Body<BuyBody>,
) -> ApiResult<(StatusCode, Json<SettlementTrade>)> {
    let trade = execute_trade(
        &state,
        &session,
        TradeInput {
            kind: TradeKind::Buy,
            idempotency_key: idempotency_key(&headers, body.idempotency_key),
            local_currency: body.amount.currency,
            amount_minor: body.amount.amount_minor,
        },
    )
    .await?;
    Ok((StatusCode::CREATED, Json(trade)))
}

#[utoipa::path(
    post,
    path = "/settlement/convert",
    operation_id = "settlement_convert",
    tag = "settlement",
    request_body = ConvertBody,
    params(
        ("Idempotency-Key" = Option<String>, Header, description = "Takes precedence over `idempotencyKey` in the body"),
    ),
    responses(
        (status = 201, description = "USDC converted, or the existing trade for a replayed idempotency key", body = SettlementTrade),
        (status = 400, description = "Validation error, insufficient balance or unsupported currency", body = ErrorResponse),
        (status = 401, description = "Not authenticated", body = ErrorResponse),
        (status = 502, description = "Rate feed unavailable (PARTNER_FAILURE)", body = ErrorResponse),
    ),
    security(("cookieAuth" = []))
)]
pub(crate) async fn convert(
    State(state): State<AppState>,
    session: Session,
    headers: HeaderMap,
    Body(body): Body<ConvertBody>,
) -> ApiResult<(StatusCode, Json<SettlementTrade>)> {
    let trade = execute_trade(
        &state,
        &session,
        TradeInput {
            kind: TradeKind::Convert,
            idempotency_key: idempotency_key(&headers, body.idempotency_key),
            local_currency: CurrencyCode::parse(body.currency.trim())?,
            amount_minor: body.usdc_amount_minor,
        },
    )
    .await?;
    Ok((StatusCode::CREATED, Json(trade)))
}

#[utoipa::path(
    get,
    path = "/settlement/transactions",
    operation_id = "settlement_transactions",
    tag = "settlement",
    responses(
        (status = 200, description = "Buy and convert trades, newest first (at most 100)", body = [SettlementTrade]),
        (status = 401, description = "Not authenticated", body = ErrorResponse),
    ),
    security(("cookieAuth" = []))
)]
pub(crate) async fn transactions(
    State(state): State<AppState>,
    session: Session,
) -> ApiResult<Json<Vec<SettlementTrade>>> {
    Ok(Json(list_trades(&state, &session).await?))
}
