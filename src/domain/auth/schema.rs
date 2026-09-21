//! Request validation for register.

use crate::error::{ApiError, ApiResult};
use regex::Regex;
use std::sync::OnceLock;

fn email_re() -> &'static Regex {
    static RE: OnceLock<Regex> = OnceLock::new();
    RE.get_or_init(|| Regex::new(r"^[^\s@]+@[^\s@]+\.[^\s@]+$").unwrap())
}

pub fn validate_register(
    email: &str,
    password: &str,
    display_name: &str,
    legal_name: &str,
) -> ApiResult<()> {
    if !email_re().is_match(email.trim()) {
        return Err(ApiError::validation("Enter a valid email address."));
    }
    if password.len() < 8 {
        return Err(ApiError::validation(
            "Password must be at least 8 characters.",
        ));
    }
    if display_name.trim().chars().count() < 2 {
        return Err(ApiError::validation("Enter your full name."));
    }
    if legal_name.trim().chars().count() < 2 {
        return Err(ApiError::validation("Enter your registered business name."));
    }
    Ok(())
}
