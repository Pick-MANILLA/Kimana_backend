//! Stub KYB. Every check passes unless the application data trips a documented
//! trigger — so the rejected path is exercisable:
//!
//! - `cac_lookup` fails if the legal name contains "reject"
//! - `director_identity` fails if any principal BVN is "00000000000"
//! - `sanctions_pep` fails if any principal full name contains "sanction" or "pep"
//! - `adverse_media` fails if the legal name contains "adverse"
//!
//! `risk_rating` isn't a pass/fail gate — see `assess_risk_rating` for the real
//! Low/Medium/High scoring, derived from industry and country of
//! incorporation (with a "highrisk"/"mediumrisk" legal-name marker for
//! testing), which the caller uses to set the approved customer's per-transfer
//! limit.
//!
//! Swap this module for a real provider (CAC lookup, NIBSS/BVN, sanctions/PEP,
//! adverse media, a risk-scoring model) when that integration lands.

use crate::contract::onboarding::{IndustrySector, OnboardingApplication, RejectionDetail};
use std::time::Duration;

pub const CHECK_KEYS: [&str; 5] = [
    "cac_lookup",
    "director_identity",
    "sanctions_pep",
    "adverse_media",
    "risk_rating",
];

pub struct CheckResult {
    pub key: &'static str,
    pub passed: bool,
    pub detail: String,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RiskRating {
    Low,
    Medium,
    High,
}

impl RiskRating {
    pub fn label(&self) -> &'static str {
        match self {
            RiskRating::Low => "Low",
            RiskRating::Medium => "Medium",
            RiskRating::High => "High",
        }
    }

    /// Per-transfer send-amount ceiling this rating imposes on the customer,
    /// overriding `Config::max_transfer_amount_minor`. `None` for Low risk —
    /// those customers stay on the platform default.
    pub fn transaction_limit_minor(&self) -> Option<i64> {
        match self {
            RiskRating::Low => None,
            RiskRating::Medium => Some(5_000_000),
            RiskRating::High => Some(1_000_000),
        }
    }
}

/// Deterministic risk scoring from the application's own data — not a
/// pass/fail gate. Higher-risk sectors and foreign incorporation each add one
/// step of risk; a "highrisk"/"mediumrisk" legal-name marker forces a rating
/// directly, for testing.
pub fn assess_risk_rating(app: &OnboardingApplication) -> RiskRating {
    let Some(business) = app.business.as_ref() else {
        return RiskRating::Low;
    };
    let legal_name = business.legal_name.to_lowercase();
    if legal_name.contains("highrisk") {
        return RiskRating::High;
    }
    if legal_name.contains("mediumrisk") {
        return RiskRating::Medium;
    }

    let high_risk_industry = matches!(
        business.industry,
        IndustrySector::OilGasServices
            | IndustrySector::SolidMinerals
            | IndustrySector::TradingCommodities
    );
    let foreign_incorporated = !business.country_of_incorporation.eq_ignore_ascii_case("NG");

    match (high_risk_industry, foreign_incorporated) {
        (true, true) => RiskRating::High,
        (true, false) | (false, true) => RiskRating::Medium,
        (false, false) => RiskRating::Low,
    }
}

pub struct KybOutcome {
    pub approved: bool,
    pub checks: Vec<CheckResult>,
    pub rejection_reasons: Vec<RejectionDetail>,
    pub risk_rating: RiskRating,
}

pub async fn run_checks(app: &OnboardingApplication, per_check_delay_ms: u64) -> KybOutcome {
    let legal_name = app
        .business
        .as_ref()
        .map(|b| b.legal_name.to_lowercase())
        .unwrap_or_default();

    let cac_fails = legal_name.contains("reject");
    let bvn_fails = app
        .principals
        .iter()
        .any(|p| p.bvn.as_deref() == Some("00000000000"));
    let sanctions_or_pep_fails = app.principals.iter().any(|p| {
        let name = p.full_name.to_lowercase();
        name.contains("sanction") || name.contains("pep")
    });
    let adverse_media_fails = legal_name.contains("adverse");

    let checks = vec![
        CheckResult {
            key: "cac_lookup",
            passed: !cac_fails,
            detail: if cac_fails {
                "RC number could not be verified with the Corporate Affairs Commission.".into()
            } else {
                "RC number verified with the Corporate Affairs Commission.".into()
            },
        },
        CheckResult {
            key: "director_identity",
            passed: !bvn_fails,
            detail: if bvn_fails {
                "A director's BVN did not resolve at NIBSS.".into()
            } else {
                "Director identities cross-referenced with NIBSS.".into()
            },
        },
        CheckResult {
            key: "sanctions_pep",
            passed: !sanctions_or_pep_fails,
            detail: if sanctions_or_pep_fails {
                "A principal matched a sanctions list or PEP screening entry.".into()
            } else {
                "No matches on OFAC SDN, EU Consolidated, UN sanctions lists, or PEP databases."
                    .into()
            },
        },
        CheckResult {
            key: "adverse_media",
            passed: !adverse_media_fails,
            detail: if adverse_media_fails {
                "Adverse media coverage found for the business name.".into()
            } else {
                "No adverse media or enforcement records found.".into()
            },
        },
        CheckResult {
            key: "risk_rating",
            passed: true,
            detail: "Segment, corridor, and volume risk model applied.".into(),
        },
    ];

    if per_check_delay_ms > 0 {
        for _ in 0..checks.len() {
            tokio::time::sleep(Duration::from_millis(per_check_delay_ms)).await;
        }
    }

    let mut rejection_reasons = Vec::new();
    if cac_fails {
        rejection_reasons.push(RejectionDetail {
            field: "business.cacNumber".into(),
            reason: checks[0].detail.clone(),
        });
    }
    if bvn_fails {
        rejection_reasons.push(RejectionDetail {
            field: "principals[].bvn".into(),
            reason: checks[1].detail.clone(),
        });
    }
    if sanctions_or_pep_fails {
        rejection_reasons.push(RejectionDetail {
            field: "principals[].fullName".into(),
            reason: checks[2].detail.clone(),
        });
    }
    if adverse_media_fails {
        rejection_reasons.push(RejectionDetail {
            field: "business.legalName".into(),
            reason: checks[3].detail.clone(),
        });
    }

    KybOutcome {
        approved: rejection_reasons.is_empty(),
        checks,
        rejection_reasons,
        risk_rating: assess_risk_rating(app),
    }
}
