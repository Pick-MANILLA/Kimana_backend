use super::common::{CurrencyCode, Money};
use super::quote::FirmQuote;
use crate::error::ApiError;
use serde::{Deserialize, Serialize};
use utoipa::ToSchema;

#[derive(Debug, Clone, Serialize, ToSchema)]
#[serde(rename_all = "camelCase")]
pub struct Recipient {
    pub id: String,
    pub customer_id: String,
    pub account_name: String,
    pub account_number: String,
    pub bank_code: String,
    pub bank_name: String,
    pub currency: CurrencyCode,
    pub country: String,
    pub validation_status: String,
    pub saved_at: String,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize, ToSchema)]
#[serde(rename_all = "SCREAMING_SNAKE_CASE")]
pub enum TransferStatus {
    Created,
    Quoted,
    Screened,
    AwaitingFunds,
    Funded,
    Settling,
    Settled,
    PayingOut,
    Completed,
    Rejected,
    Expired,
    Reversing,
    Reversed,
}

impl TransferStatus {
    pub fn as_str(&self) -> &'static str {
        match self {
            TransferStatus::Created => "CREATED",
            TransferStatus::Quoted => "QUOTED",
            TransferStatus::Screened => "SCREENED",
            TransferStatus::AwaitingFunds => "AWAITING_FUNDS",
            TransferStatus::Funded => "FUNDED",
            TransferStatus::Settling => "SETTLING",
            TransferStatus::Settled => "SETTLED",
            TransferStatus::PayingOut => "PAYING_OUT",
            TransferStatus::Completed => "COMPLETED",
            TransferStatus::Rejected => "REJECTED",
            TransferStatus::Expired => "EXPIRED",
            TransferStatus::Reversing => "REVERSING",
            TransferStatus::Reversed => "REVERSED",
        }
    }

    pub fn parse(value: &str) -> Result<Self, ApiError> {
        match value {
            "CREATED" => Ok(Self::Created),
            "QUOTED" => Ok(Self::Quoted),
            "SCREENED" => Ok(Self::Screened),
            "AWAITING_FUNDS" => Ok(Self::AwaitingFunds),
            "FUNDED" => Ok(Self::Funded),
            "SETTLING" => Ok(Self::Settling),
            "SETTLED" => Ok(Self::Settled),
            "PAYING_OUT" => Ok(Self::PayingOut),
            "COMPLETED" => Ok(Self::Completed),
            "REJECTED" => Ok(Self::Rejected),
            "EXPIRED" => Ok(Self::Expired),
            "REVERSING" => Ok(Self::Reversing),
            "REVERSED" => Ok(Self::Reversed),
            other => Err(ApiError::validation(format!(
                "Unknown transfer status {other}."
            ))),
        }
    }

    pub fn is_terminal(&self) -> bool {
        matches!(
            self,
            TransferStatus::Completed
                | TransferStatus::Rejected
                | TransferStatus::Expired
                | TransferStatus::Reversed
        )
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, ToSchema)]
#[serde(rename_all = "snake_case")]
pub enum TransferFailureCategory {
    Network,
    Validation,
    ComplianceHold,
    PartnerFailure,
}

/// Discriminated by `status`, matching the frontend's TransferState union.
/// utoipa ignores `rename_all_fields`, so each variant repeats it as
/// `#[schema(rename_all)]` to keep the OpenAPI field names camelCase.
#[derive(Debug, Clone, Serialize, ToSchema)]
#[serde(
    tag = "status",
    rename_all = "SCREAMING_SNAKE_CASE",
    rename_all_fields = "camelCase"
)]
pub enum TransferState {
    #[schema(rename_all = "camelCase")]
    Created { entered_at: String },
    #[schema(rename_all = "camelCase")]
    Quoted { entered_at: String },
    #[schema(rename_all = "camelCase")]
    Screened {
        entered_at: String,
        hold: bool,
        #[serde(skip_serializing_if = "Option::is_none")]
        hold_reason: Option<String>,
        #[serde(skip_serializing_if = "Option::is_none")]
        expected_resolution_by: Option<String>,
    },
    #[schema(rename_all = "camelCase")]
    AwaitingFunds {
        entered_at: String,
        funding_reference: String,
    },
    #[schema(rename_all = "camelCase")]
    Funded { entered_at: String },
    #[schema(rename_all = "camelCase")]
    Settling { entered_at: String },
    #[schema(rename_all = "camelCase")]
    Settled { entered_at: String },
    #[schema(rename_all = "camelCase")]
    PayingOut { entered_at: String },
    #[schema(rename_all = "camelCase")]
    Completed {
        entered_at: String,
        payout_reference: String,
    },
    #[schema(rename_all = "camelCase")]
    Rejected {
        entered_at: String,
        failure_category: TransferFailureCategory,
        reason_code: String,
    },
    #[schema(rename_all = "camelCase")]
    Expired { entered_at: String },
    #[schema(rename_all = "camelCase")]
    Reversing { entered_at: String, reason: String },
    #[schema(rename_all = "camelCase")]
    Reversed {
        entered_at: String,
        reason: String,
        reversal_ledger_entry_id: String,
    },
}

#[derive(Debug, Clone, Serialize, ToSchema)]
#[serde(rename_all = "camelCase")]
pub struct Transfer {
    pub id: String,
    pub reference: String,
    pub customer_id: String,
    pub idempotency_key: String,
    pub recipient_id: String,
    pub send_currency: CurrencyCode,
    pub receive_currency: CurrencyCode,
    pub send_amount: Money,
    pub receive_amount: Money,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub trade_description: Option<String>,
    pub quote: FirmQuote,
    pub state: TransferState,
    pub created_at: String,
    pub updated_at: String,
}

#[derive(Debug, Clone, Serialize, ToSchema)]
#[serde(rename_all = "camelCase")]
pub struct TransferStateHistoryEntry {
    pub status: TransferStatus,
    pub entered_at: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub note: Option<String>,
}

#[derive(Debug, Clone, Serialize, ToSchema)]
#[serde(rename_all = "camelCase")]
pub struct TransferTimeline {
    pub transfer_id: String,
    pub history: Vec<TransferStateHistoryEntry>,
    pub is_terminal: bool,
}
