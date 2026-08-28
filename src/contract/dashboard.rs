use super::common::{CurrencyCode, Money};
use super::ledger::AccountBalance;
use serde::Serialize;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum BalanceHighlightTone {
    Success,
    Warning,
    Danger,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct BalanceHighlight {
    pub currency: CurrencyCode,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub secondary_line: Option<String>,
    pub delta_text: String,
    pub delta_tone: BalanceHighlightTone,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct DashboardStats {
    pub volume30d: Money,
    pub transfers_in_progress: i64,
    pub payout_success_rate_percent: f64,
    pub avg_settlement_seconds: i64,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum PendingActionKind {
    ActionRequired,
    InReview,
    Submitted,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct PendingAction {
    pub id: String,
    pub title: String,
    pub subtitle: String,
    pub kind: PendingActionKind,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub transfer_id: Option<String>,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct WorkingCapitalOffer {
    pub max_advance: Money,
    pub basis_description: String,
    pub monthly_rate_percent: f64,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct DashboardOverview {
    pub display_name: String,
    pub business_name: String,
    pub account_id: String,
    pub balances: Vec<AccountBalance>,
    pub balance_highlights: Vec<BalanceHighlight>,
    pub stats: DashboardStats,
    pub pending_actions: Vec<PendingAction>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub working_capital_offer: Option<WorkingCapitalOffer>,
}
