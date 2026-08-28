use std::env;

/// Process configuration, resolved from the environment with defaults.
#[derive(Debug, Clone)]
pub struct Config {
    pub host: String,
    pub port: u16,
    pub database_url: String,
    pub storage_dir: String,
    pub cors_origin: String,

    /// Simulated per-check latency for the stub KYB provider.
    pub kyb_check_delay_ms: u64,
    /// Firm-quote lifetime. `FirmQuote.expires_at = issued_at + this`.
    pub quote_ttl_seconds: i64,
    /// Delay before a created transfer's simulated funding/settlement/payout
    /// runs. Negative disables auto-advance (tests drive the engine directly).
    pub transfer_auto_advance_ms: i64,
    /// Pause between simulated transfer steps once progression starts.
    pub transfer_step_delay_ms: u64,

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
            cors_origin: var("CORS_ORIGIN", "http://localhost:5173"),
            kyb_check_delay_ms: num("KYB_CHECK_DELAY_MS", 600),
            quote_ttl_seconds: num("QUOTE_TTL_SECONDS", 90),
            transfer_auto_advance_ms: num("TRANSFER_AUTO_ADVANCE_MS", 2500),
            transfer_step_delay_ms: num("TRANSFER_STEP_DELAY_MS", 700),
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
            is_test: true,
            ..Config::from_env()
        }
    }
}
