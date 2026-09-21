use crate::error::{ApiError, ApiResult};
use argon2::{Argon2, PasswordHasher, PasswordVerifier};
use chrono::{DateTime, SecondsFormat, Utc};
use rand::Rng;
use sha2::{Digest, Sha256};

/// ISO-8601 with millisecond precision and a `Z` suffix — matches JS `Date.toISOString()`.
pub fn iso(dt: DateTime<Utc>) -> String {
    dt.to_rfc3339_opts(SecondsFormat::Millis, true)
}

pub fn iso_opt(dt: Option<DateTime<Utc>>) -> Option<String> {
    dt.map(iso)
}

const UUID_RE_OK: fn(&str) -> bool = |s| {
    let b = s.as_bytes();
    b.len() == 36
        && b.iter().enumerate().all(|(i, c)| match i {
            8 | 13 | 18 | 23 => *c == b'-',
            _ => c.is_ascii_hexdigit(),
        })
};

/// Guards a lookup key destined for a `uuid` column — a non-UUID string would
/// make Postgres raise `22P02`; callers return "not found" instead.
pub fn is_uuid(value: &str) -> bool {
    UUID_RE_OK(value)
}

// Crockford-ish alphabet, no ambiguous characters.
const REF_ALPHABET: &[u8] = b"ABCDEFGHJKMNPQRSTVWXYZ0123456789";

/// e.g. `KM-2H4F9K`, `FR-XXXXXX`, `PO-XXXXXX`.
pub fn tagged_reference(prefix: &str) -> String {
    let mut rng = rand::thread_rng();
    let body: String = (0..6)
        .map(|_| REF_ALPHABET[rng.gen_range(0..REF_ALPHABET.len())] as char)
        .collect();
    format!("{prefix}-{body}")
}

pub fn generate_account_id(legal_name: &str) -> String {
    let initials: String = legal_name
        .split_whitespace()
        .take(3)
        .filter_map(|w| w.chars().next())
        .map(|c| c.to_ascii_uppercase())
        .collect();
    let initials = if initials.is_empty() {
        "KMA".to_string()
    } else {
        initials
    };
    let serial = rand::thread_rng().gen_range(10000..100000);
    format!("{initials}-{serial}")
}

/// Argon2id with default (OWASP-recommended) parameters, fresh random salt.
pub fn hash_password(password: &str) -> ApiResult<String> {
    Argon2::default()
        .hash_password(password.as_bytes())
        .map(|hash| hash.to_string())
        .map_err(|_| ApiError::server_error())
}

/// Never errors — a malformed stored hash or a mismatch both mean "no".
pub fn verify_password(password: &str, hash: &str) -> bool {
    Argon2::default()
        .verify_password(password.as_bytes(), hash)
        .is_ok()
}

/// A cryptographically random, hex-encoded session token (32 bytes = 256 bits).
pub fn generate_session_token() -> String {
    let mut bytes = [0u8; 32];
    rand::thread_rng().fill(&mut bytes);
    hex::encode(bytes)
}

/// SHA-256 of a session token — what's actually stored/looked up in `sessions`.
pub fn hash_token(token: &str) -> String {
    hex::encode(Sha256::digest(token.as_bytes()))
}
