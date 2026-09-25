//! HTTP clients for the collection partners (issue #79): Yellow Card for NGN
//! and Bridge for USD. Each is `None` in `Partners` until configured.

pub mod bridge;
pub mod yellowcard;

use crate::config::Config;
use crate::error::{ApiError, ErrorCode};
use rsa::RsaPublicKey;
use std::fmt::Display;
use std::time::Duration;

/// Below the 15s request timeout, so a slow partner fails as PARTNER_FAILURE.
pub(crate) const HTTP_TIMEOUT: Duration = Duration::from_secs(10);

/// Logs the partner's detail and returns a generic, retryable error: partner
/// responses can echo account data and never reach the client.
pub(crate) fn partner_error(partner: &str, path: &str, detail: impl Display) -> ApiError {
    tracing::error!(partner, path, detail = %detail, "partner call failed");
    ApiError::new(
        ErrorCode::PartnerFailure,
        "Our payment partner is unavailable. Try again in a moment.",
    )
}

pub struct Partners {
    pub yellowcard: Option<yellowcard::YellowCard>,
    pub bridge: Option<bridge::Bridge>,
    pub bridge_webhook_key: Option<RsaPublicKey>,
}

impl Partners {
    /// Panics on a malformed `BRIDGE_WEBHOOK_PUBLIC_KEY`, so a bad deploy
    /// fails at startup rather than on the first deposit.
    pub fn from_config(config: &Config) -> Self {
        Partners {
            yellowcard: yellowcard::YellowCard::from_config(config),
            bridge: bridge::Bridge::from_config(config),
            bridge_webhook_key: config
                .bridge_webhook_public_key
                .as_deref()
                .map(|pem| bridge::parse_public_key(pem).expect("BRIDGE_WEBHOOK_PUBLIC_KEY")),
        }
    }
}
