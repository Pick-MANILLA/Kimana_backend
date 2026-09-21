use std::env;

/// Process configuration, resolved from the environment with defaults.
#[derive(Debug, Clone)]
pub struct Config {
    pub host: String,
    pub port: u16,
    pub database_url: String,
    pub storage_dir: String,
    /// Allowed CORS origins. `CORS_ORIGIN` may hold a single origin or a
    /// comma-separated list (e.g. dev + the deployed frontend at once).
    pub cors_origins: Vec<String>,

    /// Simulated per-check latency for the stub KYB provider.
    pub kyb_check_delay_ms: u64,
    /// Firm-quote lifetime. `FirmQuote.expires_at = issued_at + this`.
    pub quote_ttl_seconds: i64,
    /// Delay before a created transfer's simulated funding/settlement/payout
    /// runs. Negative disables auto-advance (tests drive the engine directly).
    pub transfer_auto_advance_ms: i64,
    /// Pause between simulated transfer steps once progression starts.
    pub transfer_step_delay_ms: u64,

    /// Exposure ceiling for a single transfer's send amount (Business Rule #7).
    /// Global for now, per-customer risk-derived limits are ISSUE-02.
    pub max_transfer_amount_minor: i64,
    /// Exposure ceiling for a customer's aggregate non-terminal transfer
    /// amounts in one currency (Business Rule #7). Same caveat as above.
    pub max_aggregate_exposure_minor: i64,

    /// Send amount at or above which compliance screening holds a transfer
    /// for manual review (Business Rule #1), regardless of currency.
    pub compliance_amount_threshold_minor: i64,

    /// Percentage difference between the primary and secondary FX providers'
    /// rates for a pair above which a divergence alert is recorded.
    pub fx_divergence_threshold_percent: f64,

    /// True under integration tests: disables FX jitter and other nondeterminism.
    pub is_test: bool,
}

fn var(key: &str, default: &str) -> String {
    env::var(key).unwrap_or_else(|_| default.to_string())
}

fn num<T: std::str::FromStr>(key: &str, default: T) -> T {
    env::var(key)
        .ok()
        .and_then(|v| v.parse().ok())
        .unwrap_or(default)
}

fn csv(key: &str, default: &str) -> Vec<String> {
    var(key, default)
        .split(',')
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .map(str::to_string)
        .collect()
}

impl Config {
    pub fn from_env() -> Self {
        Config {
            host: var("HOST", "0.0.0.0"),
            port: num("PORT", 4000),
            database_url: var(
                "DATABASE_URL",
                "postgres://kimana:kimana@localhost:5432/kimana",
            ),
            storage_dir: var("STORAGE_DIR", ".storage"),
            cors_origins: csv("CORS_ORIGIN", "http://localhost:3000"),
            kyb_check_delay_ms: num("KYB_CHECK_DELAY_MS", 600),
            quote_ttl_seconds: num("QUOTE_TTL_SECONDS", 90),
            transfer_auto_advance_ms: num("TRANSFER_AUTO_ADVANCE_MS", 2500),
            transfer_step_delay_ms: num("TRANSFER_STEP_DELAY_MS", 700),
            max_transfer_amount_minor: num("MAX_TRANSFER_AMOUNT_MINOR", 10_000_000),
            max_aggregate_exposure_minor: num("MAX_AGGREGATE_EXPOSURE_MINOR", 20_000_000),
            compliance_amount_threshold_minor: num("COMPLIANCE_AMOUNT_THRESHOLD_MINOR", 5_000_000),
            fx_divergence_threshold_percent: num("FX_DIVERGENCE_THRESHOLD_PERCENT", 1.0),
            is_test: false,
        }
    }

    /// Deterministic config for integration tests: no simulated latency, no
    /// background transfer progression.
    pub fn test() -> Self {
        Config {
            kyb_check_delay_ms: 0,
            quote_ttl_seconds: 90,
            transfer_auto_advance_ms: -1,
            transfer_step_delay_ms: 0,
            max_transfer_amount_minor: 10_000_000,
            max_aggregate_exposure_minor: 20_000_000,
            compliance_amount_threshold_minor: 5_000_000,
            fx_divergence_threshold_percent: 1.0,
            is_test: true,
            ..Config::from_env()
        }
    }
}
