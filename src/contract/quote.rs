use super::common::{CurrencyCode, Money};
use serde::{Deserialize, Serialize};
use utoipa::ToSchema;

/// Whether a rate came straight from the primary feed or is a cached
/// fallback served while that feed is down (`domain::resilience`,
/// ISSUE-BE-09). `#[default]` is `Live` so historical `quote_snapshot` JSONB
/// rows from before this field existed still deserialize correctly.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default, ToSchema)]
#[serde(rename_all = "camelCase")]
pub enum RateSource {
    #[default]
    Live,
    CachedProvisional,
}

#[derive(Debug, Clone, Serialize, ToSchema)]
#[serde(rename_all = "camelCase")]
pub struct IndicativeRate {
    pub send_currency: CurrencyCode,
    pub receive_currency: CurrencyCode,
    pub rate: f64,
    pub change_percent_24h: f64,
    pub as_of: String,
    #[serde(default)]
    pub source: RateSource,
}

#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
#[serde(rename_all = "camelCase")]
pub struct CostBreakdown {
    pub rate: f64,
    pub fee: Money,
    pub send_amount: Money,
    pub receive_amount: Money,
    #[serde(default)]
    pub source: RateSource,
}

#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
#[serde(rename_all = "camelCase")]
pub struct FirmQuote {
    pub id: String,
    pub send_currency: CurrencyCode,
    pub receive_currency: CurrencyCode,
    pub breakdown: CostBreakdown,
    pub issued_at: String,
    pub expires_at: String,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, ToSchema)]
#[serde(rename_all = "lowercase")]
pub enum QuoteAmountField {
    Send,
    Receive,
}
