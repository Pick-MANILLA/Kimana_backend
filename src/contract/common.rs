use crate::error::ApiError;
use serde::{Deserialize, Serialize};

/// The only representation of money in the codebase: signed integer minor units
/// plus a currency code. Never a float, never a bare number.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Money {
    pub amount_minor: i64,
    pub currency: CurrencyCode,
}

impl Money {
    pub fn new(amount_minor: i64, currency: CurrencyCode) -> Self {
        Money {
            amount_minor,
            currency,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "UPPERCASE")]
pub enum CurrencyCode {
    Ngn,
    Kes,
    Ghs,
    Zar,
    Xof,
    Xaf,
    Egp,
    Usd,
    Eur,
    Gbp,
}

impl CurrencyCode {
    pub fn as_str(&self) -> &'static str {
        match self {
            CurrencyCode::Ngn => "NGN",
            CurrencyCode::Kes => "KES",
            CurrencyCode::Ghs => "GHS",
            CurrencyCode::Zar => "ZAR",
            CurrencyCode::Xof => "XOF",
            CurrencyCode::Xaf => "XAF",
            CurrencyCode::Egp => "EGP",
            CurrencyCode::Usd => "USD",
            CurrencyCode::Eur => "EUR",
            CurrencyCode::Gbp => "GBP",
        }
    }

    pub fn parse(value: &str) -> Result<Self, ApiError> {
        match value {
            "NGN" => Ok(CurrencyCode::Ngn),
            "KES" => Ok(CurrencyCode::Kes),
            "GHS" => Ok(CurrencyCode::Ghs),
            "ZAR" => Ok(CurrencyCode::Zar),
            "XOF" => Ok(CurrencyCode::Xof),
            "XAF" => Ok(CurrencyCode::Xaf),
            "EGP" => Ok(CurrencyCode::Egp),
            "USD" => Ok(CurrencyCode::Usd),
            "EUR" => Ok(CurrencyCode::Eur),
            "GBP" => Ok(CurrencyCode::Gbp),
            other => Err(ApiError::validation(format!("Unknown currency {other}."))),
        }
    }
}
