use super::common::{CurrencyCode, Money};
use serde::Serialize;

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct AccountBalance {
    pub account_id: String,
    pub currency: CurrencyCode,
    pub balance: Money,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub pending: Option<Money>,
    pub as_of: String,
}
