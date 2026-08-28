//! Indicative FX rates. The stub reads a seeded cache and jitters it on every
//! read (like the frontend mock); a real feed replaces `current_rate`.

use crate::contract::common::CurrencyCode;
use crate::contract::quote::IndicativeRate;
use crate::error::{ApiError, ApiResult};
use crate::state::AppState;
use crate::util::iso;
use axum::extract::{Query, State};
use axum::routing::get;
use axum::{Json, Router};
use chrono::{DateTime, Utc};
use rand::Rng;
use serde::Deserialize;
use sqlx::PgPool;

const JITTER_SPREAD: f64 = 0.004;

pub fn pair_key(send: CurrencyCode, receive: CurrencyCode) -> String {
    format!("{}/{}", send.as_str(), receive.as_str())
}

#[derive(sqlx::FromRow)]
struct FxRow {
    rate: f64,
    change_percent_24h: f64,
    as_of: DateTime<Utc>,
}

pub struct Rate {
    pub rate: f64,
    pub change_percent_24h: f64,
    pub as_of: String,
}

/// Stub provider: seeded cache + jitter, persisting the drift. `jitter=false`
/// (tests) returns the stored value untouched.
pub async fn current_rate(pool: &PgPool, pair: &str, jitter: bool) -> ApiResult<Option<Rate>> {
    let Some(row) = sqlx::query_as::<_, FxRow>(
        "select rate, change_percent_24h, as_of from fx_rates where pair = $1",
    )
    .bind(pair)
    .fetch_optional(pool)
    .await?
    else {
        return Ok(None);
    };

    if !jitter {
        return Ok(Some(Rate {
            rate: row.rate,
            change_percent_24h: row.change_percent_24h,
            as_of: iso(row.as_of),
        }));
    }

    let factor = 1.0 + (rand::thread_rng().gen::<f64>() - 0.5) * JITTER_SPREAD;
    let drifted = (row.rate * factor * 100.0).round() / 100.0;
    let as_of: DateTime<Utc> = sqlx::query_scalar(
        "update fx_rates set rate = $2, as_of = now() where pair = $1 returning as_of",
    )
    .bind(pair)
    .bind(drifted)
    .fetch_one(pool)
    .await?;

    Ok(Some(Rate {
        rate: drifted,
        change_percent_24h: row.change_percent_24h,
        as_of: iso(as_of),
    }))
}

pub async fn get_indicative_rate(
    state: &AppState,
    send: CurrencyCode,
    receive: CurrencyCode,
) -> ApiResult<IndicativeRate> {
    let pair = pair_key(send, receive);
    let rate = current_rate(&state.pool, &pair, !state.config.is_test)
        .await?
        .ok_or_else(|| ApiError::validation(format!("No rate available for {pair}.")))?;
    Ok(IndicativeRate {
        send_currency: send,
        receive_currency: receive,
        rate: rate.rate,
        change_percent_24h: rate.change_percent_24h,
        as_of: rate.as_of,
    })
}

// ---- routes ----

#[derive(Deserialize)]
struct RateQuery {
    send: String,
    receive: String,
}

pub fn routes() -> Router<AppState> {
    Router::new().route("/rates/indicative", get(indicative))
}

async fn indicative(
    State(state): State<AppState>,
    Query(q): Query<RateQuery>,
) -> ApiResult<Json<IndicativeRate>> {
    let send = CurrencyCode::parse(&q.send)?;
    let receive = CurrencyCode::parse(&q.receive)?;
    Ok(Json(get_indicative_rate(&state, send, receive).await?))
}
