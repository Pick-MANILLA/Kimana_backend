use super::common::{CurrencyCode, Money};
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct IndicativeRate {
    pub send_currency: CurrencyCode,
    pub receive_currency: CurrencyCode,
    pub rate: f64,
    pub change_percent_24h: f64,
    pub as_of: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CostBreakdown {
    pub rate: f64,
    pub fee: Money,
    pub send_amount: Money,
    pub receive_amount: Money,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct FirmQuote {
    pub id: String,
    pub send_currency: CurrencyCode,
    pub receive_currency: CurrencyCode,
    pub breakdown: CostBreakdown,
    pub issued_at: String,
    pub expires_at: String,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum QuoteAmountField {
    Send,
    Receive,
}
