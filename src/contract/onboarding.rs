use super::common::Money;
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum OnboardingStatus {
    Draft,
    Submitted,
    InReview,
    Approved,
    Rejected,
}

impl OnboardingStatus {
    pub fn as_str(&self) -> &'static str {
        match self {
            OnboardingStatus::Draft => "draft",
            OnboardingStatus::Submitted => "submitted",
            OnboardingStatus::InReview => "in_review",
            OnboardingStatus::Approved => "approved",
            OnboardingStatus::Rejected => "rejected",
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Address {
    #[serde(skip_serializing_if = "Option::is_none", default)]
    pub line1: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none", default)]
    pub line2: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none", default)]
    pub city: Option<String>,
    pub state: String,
    #[serde(skip_serializing_if = "Option::is_none", default)]
    pub postal_code: Option<String>,
    pub country: String,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum BusinessType {
    SoleProprietorship,
    LimitedLiabilityCompany,
    Partnership,
    PublicLimitedCompany,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum IndustrySector {
    AgricultureAgroExport,
    TextilesApparel,
    SolidMinerals,
    Manufacturing,
    OilGasServices,
    Technology,
    TradingCommodities,
    Other,
}

impl IndustrySector {
    /// Lifted from Kimana_frontend/src/api/mock/onboardingApi.ts.
    pub fn segment_label(&self) -> &'static str {
        match self {
            IndustrySector::AgricultureAgroExport => "Agro Exporter",
            IndustrySector::TextilesApparel => "Textiles Exporter",
            IndustrySector::SolidMinerals => "Solid Minerals Exporter",
            IndustrySector::Manufacturing => "Manufacturing Exporter",
            IndustrySector::OilGasServices => "Oil & Gas Services",
            IndustrySector::Technology => "Technology Exporter",
            IndustrySector::TradingCommodities => "Commodities Trader",
            IndustrySector::Other => "Trading Business",
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct BusinessDetails {
    pub legal_name: String,
    #[serde(skip_serializing_if = "Option::is_none", default)]
    pub trading_name: Option<String>,
    pub cac_number: String,
    pub business_type: BusinessType,
    pub industry: IndustrySector,
    pub trading_address: Address,
    pub country_of_incorporation: String,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum PrincipalRole {
    Director,
    BeneficialOwner,
    Both,
}

impl PrincipalRole {
    pub fn as_str(&self) -> &'static str {
        match self {
            PrincipalRole::Director => "director",
            PrincipalRole::BeneficialOwner => "beneficial_owner",
            PrincipalRole::Both => "both",
        }
    }
    pub fn parse(value: &str) -> Option<Self> {
        match value {
            "director" => Some(PrincipalRole::Director),
            "beneficial_owner" => Some(PrincipalRole::BeneficialOwner),
            "both" => Some(PrincipalRole::Both),
            _ => None,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct DirectorOrBeneficialOwner {
    #[serde(skip_serializing_if = "Option::is_none", default)]
    pub id: Option<String>,
    pub full_name: String,
    pub role: PrincipalRole,
    #[serde(skip_serializing_if = "Option::is_none", default)]
    pub ownership_percentage: Option<f64>,
    #[serde(skip_serializing_if = "Option::is_none", default)]
    pub date_of_birth: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none", default)]
    pub bvn: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none", default)]
    pub nin: Option<String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum OnboardingDocumentType {
    CacCertificate,
    Memart,
    ProofOfAddress,
    DirectorsId,
    BoardResolution,
}

impl OnboardingDocumentType {
    pub fn parse(value: &str) -> Option<Self> {
        match value {
            "cac_certificate" => Some(Self::CacCertificate),
            "memart" => Some(Self::Memart),
            "proof_of_address" => Some(Self::ProofOfAddress),
            "directors_id" => Some(Self::DirectorsId),
            "board_resolution" => Some(Self::BoardResolution),
            _ => None,
        }
    }
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::CacCertificate => "cac_certificate",
            Self::Memart => "memart",
            Self::ProofOfAddress => "proof_of_address",
            Self::DirectorsId => "directors_id",
            Self::BoardResolution => "board_resolution",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum DocumentUploadStatus {
    Pending,
    Uploading,
    Uploaded,
    Failed,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct UploadedDocument {
    pub id: String,
    pub r#type: String,
    pub file_name: String,
    pub mime_type: String,
    pub size_bytes: i64,
    pub status: String,
    pub upload_progress_percent: i32,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub uploaded_at: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub error_message: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct RejectionDetail {
    pub field: String,
    pub reason: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ApprovedAccountSummary {
    pub account_id: String,
    pub risk_rating_label: String,
    pub segment: String,
    pub corridor: String,
    pub monthly_limit: Money,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct OnboardingApplication {
    pub id: String,
    pub customer_id: String,
    pub status: String,
    pub business: Option<BusinessDetails>,
    pub principals: Vec<DirectorOrBeneficialOwner>,
    pub documents: Vec<UploadedDocument>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub rejection_reasons: Option<Vec<RejectionDetail>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub approved_summary: Option<ApprovedAccountSummary>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub submitted_at: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub reviewed_at: Option<String>,
}
