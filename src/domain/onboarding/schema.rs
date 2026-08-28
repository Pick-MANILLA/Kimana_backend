//! Request validation. Field rules mirror the frontend forms. Conditional
//! required-by-role on principals is intentionally not enforced (mock enforces
//! none; frontend enforces director fields client-side).

use crate::contract::onboarding::{BusinessDetails, DirectorOrBeneficialOwner, PrincipalRole};
use crate::error::{ApiError, ApiResult};
use regex::Regex;
use std::sync::OnceLock;

fn rc_re() -> &'static Regex {
    static RE: OnceLock<Regex> = OnceLock::new();
    RE.get_or_init(|| Regex::new(r"(?i)^RC-?\d{4,8}$").unwrap())
}
fn digits11_re() -> &'static Regex {
    static RE: OnceLock<Regex> = OnceLock::new();
    RE.get_or_init(|| Regex::new(r"^\d{11}$").unwrap())
}
fn iso_date_re() -> &'static Regex {
    static RE: OnceLock<Regex> = OnceLock::new();
    RE.get_or_init(|| Regex::new(r"^\d{4}-\d{2}-\d{2}$").unwrap())
}

pub fn validate_business(b: &BusinessDetails) -> ApiResult<()> {
    if b.legal_name.trim().chars().count() < 2 {
        return Err(ApiError::validation("Enter your registered business name."));
    }
    if !rc_re().is_match(b.cac_number.trim()) {
        return Err(ApiError::validation(
            "Enter a valid RC number, e.g. RC-1234567.",
        ));
    }
    if b.trading_address.state.trim().is_empty() {
        return Err(ApiError::validation(
            "Select your primary state of operation.",
        ));
    }
    if b.trading_address.country.trim().len() != 2 || b.country_of_incorporation.trim().len() != 2 {
        return Err(ApiError::validation("Use a 2-letter ISO country code."));
    }
    Ok(())
}

pub fn validate_principals(principals: &[DirectorOrBeneficialOwner]) -> ApiResult<()> {
    for p in principals {
        if p.full_name.trim().chars().count() < 2 {
            return Err(ApiError::validation("Enter the full name."));
        }
        if let Some(bvn) = &p.bvn {
            if !digits11_re().is_match(bvn) {
                return Err(ApiError::validation("BVN must be 11 digits."));
            }
        }
        if let Some(nin) = &p.nin {
            if !digits11_re().is_match(nin) {
                return Err(ApiError::validation("NIN must be 11 digits."));
            }
        }
        if let Some(dob) = &p.date_of_birth {
            if !iso_date_re().is_match(dob) {
                return Err(ApiError::validation(
                    "Date of birth must be an ISO date (YYYY-MM-DD).",
                ));
            }
        }
        if let Some(pct) = p.ownership_percentage {
            if !(0.0..=100.0).contains(&pct) {
                return Err(ApiError::validation("Ownership percentage must be 0–100."));
            }
        }
        // role already validated by deserialization
        let _ = PrincipalRole::as_str(&p.role);
    }
    Ok(())
}
