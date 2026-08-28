//! Stub KYB. Every check passes unless the application data trips a documented
//! trigger — so the rejected path is exercisable:
//!
//! - `cac_lookup` fails if the legal name contains "reject"
//! - `director_identity` fails if any principal BVN is "00000000000"
//! - `sanctions_pep` fails if any principal full name contains "sanction"
//!
//! `adverse_media` and `risk_rating` always pass. Swap this module for a real
//! provider (CAC lookup, NIBSS/BVN, sanctions/PEP, adverse media).

use crate::contract::onboarding::{OnboardingApplication, RejectionDetail};
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

pub struct KybOutcome {
    pub approved: bool,
    pub checks: Vec<CheckResult>,
    pub rejection_reasons: Vec<RejectionDetail>,
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
    let sanctions_fails = app
        .principals
        .iter()
        .any(|p| p.full_name.to_lowercase().contains("sanction"));

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
            passed: !sanctions_fails,
            detail: if sanctions_fails {
                "A principal matched a sanctions / PEP list entry.".into()
            } else {
                "No matches on OFAC SDN, EU Consolidated, or UN sanctions lists.".into()
            },
        },
        CheckResult {
            key: "adverse_media",
            passed: true,
            detail: "No adverse media or enforcement records found.".into(),
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
    if sanctions_fails {
        rejection_reasons.push(RejectionDetail {
            field: "principals[].fullName".into(),
            reason: checks[2].detail.clone(),
        });
    }

    KybOutcome {
        approved: rejection_reasons.is_empty(),
        checks,
        rejection_reasons,
    }
}
