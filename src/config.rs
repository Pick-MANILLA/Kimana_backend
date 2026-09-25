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

    /// Consecutive primary-feed failures before the FX circuit breaker trips
    /// open and starts failing fast instead of hammering a down dependency
    /// (`domain::resilience::CircuitBreaker`, ISSUE-BE-09).
    pub fx_breaker_failure_threshold: u32,
    /// How long the FX breaker stays open before letting one trial call
    /// through to test whether the feed has recovered.
    pub fx_breaker_reset_seconds: u64,
    /// Max age of a cached rate the FX breaker's fallback will still serve
    /// as `cachedProvisional` while the primary feed is down.
    pub fx_cache_max_age_seconds: u64,
    /// Per-call timeout the FX breaker applies to the primary feed fetch. A
    /// hung/slow call (not just an outright error) counts as a failure once
    /// this elapses — without it, a stalled DB/partner connection would just
    /// block until the outer request timeout fires, and the breaker would
    /// never see a failure to react to.
    pub fx_call_timeout_ms: u64,

    /// True when `SETTLEMENT_RPC_URL` is set: confirmed vault events, not the
    /// simulator, move a transfer from SETTLING onwards.
    pub settlement_onchain: bool,
    /// Blocks a vault event must be buried under before the listener applies it.
    pub settlement_confirmations: u64,
    /// Pause between listener polls.
    pub settlement_poll_ms: u64,
    /// First block the listener scans when it has no stored cursor yet
    /// (the vault's deployment block).
    pub settlement_start_block: u64,
    /// Largest block range per `eth_getLogs`; keep it within the RPC's cap.
    pub settlement_log_range: u64,
    /// Shown by `GET /settlement/balance` so the wallet can name the chain.
    /// The network block appears only when both the chain id and
    /// `SETTLEMENT_VAULT_ADDRESS` are set; the others are optional extras.
    pub settlement_chain_id: Option<u64>,
    pub settlement_vault_address: Option<String>,
    pub settlement_usdc_address: Option<String>,
    pub settlement_explorer_url: Option<String>,

    /// Yellow Card (NGN collections). Off unless both the key and the
    /// secret are set; the secret also signs Yellow Card's webhooks.
    pub yellowcard_api_key: Option<String>,
    pub yellowcard_api_secret: Option<String>,
    pub yellowcard_base_url: String,
    /// Pins the NGN bank-transfer deposit channel instead of looking it up.
    pub yellowcard_channel_id: Option<String>,
    /// Sandbox only: the payer account put on each receive. Yellow Card's
    /// sandbox completes receives from `1111111111` and fails `0000000000`.
    pub yellowcard_source_account: Option<String>,

    /// Bridge (USD collections). The API key creates virtual accounts; the
    /// PEM public key of the webhook endpoint verifies deliveries. Either can
    /// be set without the other.
    pub bridge_api_key: Option<String>,
    pub bridge_base_url: String,
    pub bridge_webhook_public_key: Option<String>,
    /// Where Bridge delivers deposits, e.g. `usdc` on `base` to the vault.
    pub bridge_destination_rail: String,
    pub bridge_destination_currency: String,
    pub bridge_destination_address: Option<String>,

    /// True under integration tests: disables FX jitter and other nondeterminism.
    pub is_test: bool,

    /// Whether the session cookie is marked `Secure`. Browsers silently drop
    /// `Secure` cookies over plain `http://localhost`, so local dev needs
    /// `COOKIE_SECURE=false`; production (behind TLS) keeps the default.
    pub cookie_secure: bool,
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

/// Set and non-empty, else `None`.
fn opt(key: &str) -> Option<String> {
    env::var(key).ok().filter(|v| !v.trim().is_empty())
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
            fx_breaker_failure_threshold: num("FX_BREAKER_FAILURE_THRESHOLD", 3),
            fx_breaker_reset_seconds: num("FX_BREAKER_RESET_SECONDS", 10),
            fx_cache_max_age_seconds: num("FX_CACHE_MAX_AGE_SECONDS", 300),
            fx_call_timeout_ms: num("FX_CALL_TIMEOUT_MS", 2_000),
            settlement_onchain: env::var("SETTLEMENT_RPC_URL").is_ok(),
            settlement_confirmations: num("SETTLEMENT_CONFIRMATIONS", 3),
            settlement_poll_ms: num("SETTLEMENT_POLL_MS", 2_000),
            settlement_start_block: num("SETTLEMENT_START_BLOCK", 0),
            settlement_log_range: num("SETTLEMENT_LOG_RANGE", 1_000),
            settlement_chain_id: env::var("SETTLEMENT_CHAIN_ID")
                .ok()
                .and_then(|v| v.parse().ok()),
            settlement_vault_address: env::var("SETTLEMENT_VAULT_ADDRESS").ok(),
            settlement_usdc_address: env::var("SETTLEMENT_USDC_ADDRESS").ok(),
            settlement_explorer_url: env::var("SETTLEMENT_EXPLORER_URL").ok(),
            yellowcard_api_key: opt("YELLOWCARD_API_KEY"),
            yellowcard_api_secret: opt("YELLOWCARD_API_SECRET"),
            yellowcard_base_url: var("YELLOWCARD_BASE_URL", "https://sandbox.api.yellowcard.io"),
            yellowcard_channel_id: opt("YELLOWCARD_CHANNEL_ID"),
            yellowcard_source_account: opt("YELLOWCARD_SOURCE_ACCOUNT"),
            bridge_api_key: opt("BRIDGE_API_KEY"),
            bridge_base_url: var("BRIDGE_BASE_URL", "https://api.sandbox.bridge.xyz/v0"),
            // A PEM in a single-line env var has its newlines escaped.
            bridge_webhook_public_key: opt("BRIDGE_WEBHOOK_PUBLIC_KEY")
                .map(|pem| pem.replace("\\n", "\n")),
            bridge_destination_rail: var("BRIDGE_DESTINATION_RAIL", "base"),
            bridge_destination_currency: var("BRIDGE_DESTINATION_CURRENCY", "usdc"),
            bridge_destination_address: opt("BRIDGE_DESTINATION_ADDRESS")
                .or_else(|| opt("SETTLEMENT_VAULT_ADDRESS")),
            is_test: false,
            cookie_secure: num("COOKIE_SECURE", true),
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
            settlement_onchain: false,
            // Tests point these at local mocks when they need a partner.
            yellowcard_api_key: None,
            yellowcard_api_secret: None,
            yellowcard_channel_id: None,
            yellowcard_source_account: None,
            bridge_api_key: None,
            bridge_webhook_public_key: None,
            bridge_destination_address: None,
            is_test: true,
            cookie_secure: false,
            ..Config::from_env()
        }
    }
}
