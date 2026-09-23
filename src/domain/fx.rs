//! Indicative FX rates, sourced from two independent providers (PRD
//! Functional Requirements §B, ISSUE-03): `PrimaryProvider` reads the seeded
//! `fx_rates` cache and jitters it on every read (like the frontend mock);
//! `SecondaryProvider` does the same against its own `fx_secondary_rates`
//! cache, independently seeded and independently jittered. The customer-
//! facing rate always comes from the primary provider — the secondary exists
//! to compare against and catch divergence, not to be quoted from. A real
//! second feed replaces `SecondaryProvider::get_rate`'s query without
//! touching callers.

use crate::config::Config;
use crate::contract::common::CurrencyCode;
use crate::contract::quote::{IndicativeRate, RateSource};
use crate::domain::resilience::{CallError, CircuitBreaker};
use crate::error::{ApiError, ApiResult, ErrorResponse};
use crate::state::AppState;
use crate::util::iso;
use axum::extract::{Query, State};
use axum::routing::get;
use axum::{Json, Router};
use chrono::{DateTime, Utc};
use rand::Rng;
use serde::Deserialize;
use sqlx::PgPool;
use std::collections::HashMap;
use std::future::Future;
use std::sync::Mutex as StdMutex;
use std::time::{Duration, Instant};

const JITTER_SPREAD: f64 = 0.004;
const SECONDARY_JITTER_SPREAD: f64 = 0.006;

/// A source of FX rates. `get_rate` returns `None` when the provider has no
/// quote for that pair at all (an unknown pair), distinct from the pair
/// simply not diverging.
pub trait FxProvider {
    fn name(&self) -> &'static str;
    fn get_rate(
        &self,
        pool: &PgPool,
        pair: &str,
    ) -> impl Future<Output = ApiResult<Option<f64>>> + Send;
}

pub struct PrimaryProvider;

impl FxProvider for PrimaryProvider {
    fn name(&self) -> &'static str {
        "primary"
    }

    async fn get_rate(&self, pool: &PgPool, pair: &str) -> ApiResult<Option<f64>> {
        let rate = sqlx::query_scalar("select rate from fx_rates where pair = $1")
            .bind(pair)
            .fetch_optional(pool)
            .await?;
        Ok(rate)
    }
}

pub struct SecondaryProvider;

impl FxProvider for SecondaryProvider {
    fn name(&self) -> &'static str {
        "secondary"
    }

    async fn get_rate(&self, pool: &PgPool, pair: &str) -> ApiResult<Option<f64>> {
        let rate = sqlx::query_scalar("select rate from fx_secondary_rates where pair = $1")
            .bind(pair)
            .fetch_optional(pool)
            .await?;
        Ok(rate)
    }
}

/// Nudges the secondary provider's stored rate, mirroring `current_rate`'s
/// jitter on the primary. A no-op if the pair isn't seeded there.
async fn jitter_secondary(pool: &PgPool, pair: &str) -> ApiResult<()> {
    let rate: Option<f64> =
        sqlx::query_scalar("select rate from fx_secondary_rates where pair = $1")
            .bind(pair)
            .fetch_optional(pool)
            .await?;
    let Some(rate) = rate else {
        return Ok(());
    };
    let factor = 1.0 + (rand::thread_rng().gen::<f64>() - 0.5) * SECONDARY_JITTER_SPREAD;
    let drifted = (rate * factor * 100.0).round() / 100.0;
    sqlx::query("update fx_secondary_rates set rate = $2, as_of = now() where pair = $1")
        .bind(pair)
        .bind(drifted)
        .execute(pool)
        .await?;
    Ok(())
}

pub struct DivergenceEvent {
    pub pair: String,
    pub provider_a: String,
    pub provider_b: String,
    pub rate_a: f64,
    pub rate_b: f64,
    pub divergence_percent: f64,
    pub threshold_percent: f64,
}

/// Compares the two providers' current rates for `pair` and, if they diverge
/// beyond `threshold_percent`, records an alert row and logs a structured
/// warning. Returns the divergence, if both providers quote the pair.
pub async fn check_divergence(
    pool: &PgPool,
    pair: &str,
    threshold_percent: f64,
) -> ApiResult<Option<f64>> {
    let rate_a = PrimaryProvider.get_rate(pool, pair).await?;
    let rate_b = SecondaryProvider.get_rate(pool, pair).await?;
    let (Some(rate_a), Some(rate_b)) = (rate_a, rate_b) else {
        return Ok(None);
    };

    let divergence_percent = ((rate_a - rate_b).abs() / rate_a) * 100.0;
    if divergence_percent > threshold_percent {
        sqlx::query(
            "insert into fx_rate_divergence_events
               (pair, provider_a, provider_b, rate_a, rate_b, divergence_percent, threshold_percent)
             values ($1, $2, $3, $4, $5, $6, $7)",
        )
        .bind(pair)
        .bind(PrimaryProvider.name())
        .bind(SecondaryProvider.name())
        .bind(rate_a)
        .bind(rate_b)
        .bind(divergence_percent)
        .bind(threshold_percent)
        .execute(pool)
        .await?;
        tracing::warn!(
            pair,
            rate_a,
            rate_b,
            divergence_percent,
            threshold_percent,
            "FX provider rates diverge beyond threshold"
        );
    }
    Ok(Some(divergence_percent))
}

/// Recent divergence alerts, most recent first — the queryable record the
/// acceptance criteria asks for, until a real alerting pipeline replaces it.
pub async fn recent_divergence_events(
    pool: &PgPool,
    limit: i64,
) -> ApiResult<Vec<DivergenceEvent>> {
    let rows: Vec<(String, String, String, f64, f64, f64, f64)> = sqlx::query_as(
        "select pair, provider_a, provider_b, rate_a, rate_b, divergence_percent, threshold_percent
           from fx_rate_divergence_events
          order by detected_at desc
          limit $1",
    )
    .bind(limit)
    .fetch_all(pool)
    .await?;
    Ok(rows
        .into_iter()
        .map(
            |(
                pair,
                provider_a,
                provider_b,
                rate_a,
                rate_b,
                divergence_percent,
                threshold_percent,
            )| {
                DivergenceEvent {
                    pair,
                    provider_a,
                    provider_b,
                    rate_a,
                    rate_b,
                    divergence_percent,
                    threshold_percent,
                }
            },
        )
        .collect())
}

pub fn pair_key(send: CurrencyCode, receive: CurrencyCode) -> String {
    format!("{}/{}", send.as_str(), receive.as_str())
}

#[derive(sqlx::FromRow)]
struct FxRow {
    rate: f64,
    change_percent_24h: f64,
    as_of: DateTime<Utc>,
}

#[derive(Clone)]
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

/// Outcome of asking `FxResilience` for a pair's rate: whether it came
/// straight from the primary feed, is a cached fallback served because that
/// feed is currently failing, or the pair simply isn't configured at all
/// (not a resilience concern — the feed answered fine, it just has nothing
/// for this pair, so this never touches the circuit breaker or the cache).
enum RateOutcome {
    Live(Rate),
    CachedProvisional(Rate),
    NotFound,
}

struct CachedRate {
    rate: Rate,
    cached_at: Instant,
}

/// Wraps the primary FX feed in a circuit breaker plus a last-known-good
/// rate cache per pair (ISSUE-BE-09): a lone transient failure still gets
/// served (from cache, or by letting the call through since the breaker
/// isn't open yet); sustained failure trips the breaker and callers get a
/// structured `PARTNER_UNAVAILABLE`-style error once the cache goes stale.
pub struct FxResilience {
    breaker: CircuitBreaker,
    cache: StdMutex<HashMap<String, CachedRate>>,
    max_cache_age: Duration,
    call_timeout: Duration,
}

impl FxResilience {
    pub fn new(
        failure_threshold: u32,
        reset_timeout: Duration,
        max_cache_age: Duration,
        call_timeout: Duration,
    ) -> Self {
        Self {
            breaker: CircuitBreaker::new(failure_threshold, reset_timeout),
            cache: StdMutex::new(HashMap::new()),
            max_cache_age,
            call_timeout,
        }
    }

    pub fn from_config(config: &Config) -> Self {
        Self::new(
            config.fx_breaker_failure_threshold,
            Duration::from_secs(config.fx_breaker_reset_seconds),
            Duration::from_secs(config.fx_cache_max_age_seconds),
            Duration::from_millis(config.fx_call_timeout_ms),
        )
    }

    async fn resolve<F, Fut>(&self, pair: &str, fetch: F) -> ApiResult<RateOutcome>
    where
        F: FnOnce() -> Fut,
        Fut: Future<Output = ApiResult<Option<Rate>>>,
    {
        // A hung/slow call must count as a failure on its own schedule, not
        // whenever the outer request timeout eventually fires — otherwise
        // the breaker never gets a chance to react to it at all.
        let timeout = self.call_timeout;
        let timed_fetch = || async move {
            match tokio::time::timeout(timeout, fetch()).await {
                Ok(result) => result,
                Err(_) => Err(ApiError::new(
                    crate::error::ErrorCode::PartnerFailure,
                    format!("fx feed call exceeded {}ms", timeout.as_millis()),
                )),
            }
        };

        match self.breaker.call(timed_fetch).await {
            Ok(Some(rate)) => {
                self.cache.lock().unwrap().insert(
                    pair.to_string(),
                    CachedRate {
                        rate: rate.clone(),
                        cached_at: Instant::now(),
                    },
                );
                Ok(RateOutcome::Live(rate))
            }
            Ok(None) => Ok(RateOutcome::NotFound),
            Err(outcome) => {
                let retry_after = match &outcome {
                    CallError::Open { retry_after } => *retry_after,
                    CallError::Failed(err) => {
                        tracing::warn!(pair, error = %err, "fx primary feed failed; trying cached fallback");
                        Duration::ZERO
                    }
                };
                let cached = self
                    .cache
                    .lock()
                    .unwrap()
                    .get(pair)
                    .map(|c| (c.rate.clone(), c.cached_at.elapsed()));
                match cached {
                    Some((rate, age)) if age <= self.max_cache_age => {
                        Ok(RateOutcome::CachedProvisional(rate))
                    }
                    _ => Err(ApiError::partner_unavailable(pair, retry_after)),
                }
            }
        }
    }
}

pub async fn get_indicative_rate(
    state: &AppState,
    send: CurrencyCode,
    receive: CurrencyCode,
) -> ApiResult<IndicativeRate> {
    let pair = pair_key(send, receive);
    let jitter = !state.config.is_test;
    let outcome = state
        .fx_resilience
        .resolve(&pair, || current_rate(&state.pool, &pair, jitter))
        .await?;

    let (rate, source) = match outcome {
        RateOutcome::NotFound => {
            return Err(ApiError::validation(format!("No rate available for {pair}.")));
        }
        RateOutcome::Live(rate) => (rate, RateSource::Live),
        RateOutcome::CachedProvisional(rate) => (rate, RateSource::CachedProvisional),
    };

    match source {
        RateSource::Live => {
            if !state.config.is_test {
                jitter_secondary(&state.pool, &pair).await?;
            }
            check_divergence(
                &state.pool,
                &pair,
                state.config.fx_divergence_threshold_percent,
            )
            .await?;
        }
        RateSource::CachedProvisional => {
            // The primary DB is the same store the secondary-provider jitter
            // and divergence check would hit too — both would just fail the
            // same way, defeating the fallback. Skip them.
        }
    }

    Ok(IndicativeRate {
        send_currency: send,
        receive_currency: receive,
        rate: rate.rate,
        change_percent_24h: rate.change_percent_24h,
        as_of: rate.as_of,
        source,
    })
}

// ---- routes ----

#[derive(Deserialize, utoipa::IntoParams)]
#[into_params(parameter_in = Query)]
pub(crate) struct RateQuery {
    /// Send currency code, e.g. `USD`
    send: String,
    /// Receive currency code, e.g. `NGN`
    receive: String,
}

pub fn routes() -> Router<AppState> {
    Router::new().route("/rates/indicative", get(indicative))
}

#[utoipa::path(
    get,
    path = "/rates/indicative",
    tag = "fx",
    params(RateQuery),
    responses(
        (status = 200, description = "Indicative rate", body = IndicativeRate),
        (status = 400, description = "Unknown currency or no rate for the pair", body = ErrorResponse),
        (status = 502, description = "Rate feed unavailable (PARTNER_FAILURE)", body = ErrorResponse),
    )
)]
pub(crate) async fn indicative(
    State(state): State<AppState>,
    Query(q): Query<RateQuery>,
) -> ApiResult<Json<IndicativeRate>> {
    let send = CurrencyCode::parse(&q.send)?;
    let receive = CurrencyCode::parse(&q.receive)?;
    Ok(Json(get_indicative_rate(&state, send, receive).await?))
}
