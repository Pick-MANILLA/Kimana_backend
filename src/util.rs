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

/// HMAC-SHA256 (RFC 2104). Built on `sha2` directly: the `hmac` crate in the
/// lockfile targets an older `digest` than the `sha2` we depend on.
pub fn hmac_sha256(key: &[u8], message: &[u8]) -> [u8; 32] {
    const BLOCK: usize = 64;
    let mut block_key = [0u8; BLOCK];
    if key.len() > BLOCK {
        block_key[..32].copy_from_slice(&Sha256::digest(key));
    } else {
        block_key[..key.len()].copy_from_slice(key);
    }
    let pad = |byte: u8| block_key.map(|k| k ^ byte);
    let mut inner = Sha256::new();
    inner.update(pad(0x36));
    inner.update(message);
    let mut outer = Sha256::new();
    outer.update(pad(0x5c));
    outer.update(inner.finalize());
    outer.finalize().into()
}

/// Compares two byte strings without an early exit, so the time taken doesn't
/// reveal how much of a signature matched.
pub fn constant_time_eq(a: &[u8], b: &[u8]) -> bool {
    a.len() == b.len() && a.iter().zip(b).fold(0u8, |acc, (x, y)| acc | (x ^ y)) == 0
}

/// Decimal places the fixed-point rate math below scales to. Matches the
/// precision an on-chain FX oracle would publish a rate at, so `apply_rate`/
/// `invert_rate` do the same integer floor-division a settlement contract
/// does instead of `f64::round()`, which rounds `.5` up and so disagrees
/// with floor division on exactly the cases that matter (see ISSUE-BE-02).
pub const RATE_DECIMALS: u32 = 8;
const RATE_SCALE: i128 = 10i128.pow(RATE_DECIMALS);

/// Scales a decimal rate (e.g. `1650.2345`) to a fixed-point integer with
/// `RATE_DECIMALS` places. The only place rate math touches `f64` — everything
/// downstream is integer arithmetic.
fn scale_rate(rate: f64) -> i128 {
    (rate * RATE_SCALE as f64).round() as i128
}

/// `amount_minor * rate`, as integer fixed-point floor division (Rust integer
/// division truncates toward zero, which is floor division for the
/// non-negative amounts money math here deals in).
pub fn apply_rate(amount_minor: i64, rate: f64) -> i64 {
    let scaled = scale_rate(rate);
    ((amount_minor as i128 * scaled) / RATE_SCALE) as i64
}

/// Inverse of `apply_rate`: `amount_minor / rate`.
pub fn invert_rate(amount_minor: i64, rate: f64) -> i64 {
    let scaled = scale_rate(rate);
    ((amount_minor as i128 * RATE_SCALE) / scaled) as i64
}

#[cfg(test)]
mod rate_math_tests {
    use super::*;

    #[test]
    fn floors_instead_of_rounding_half_up() {
        // 1 minor unit * rate 0.5 = 0.5 exactly. f64::round() rounds this up
        // to 1; integer floor division (what on-chain settlement math does)
        // floors it to 0. This is the exact discrepancy ISSUE-BE-02 reports.
        assert_eq!(apply_rate(1, 0.5), 0);
    }

    #[test]
    fn round_trips_on_exact_values() {
        let rate = 1650.25;
        let receive = apply_rate(1_000, rate);
        assert_eq!(receive, 1_650_250);
        assert_eq!(invert_rate(receive, rate), 1_000);
    }

    #[test]
    fn deterministic_across_repeated_calls() {
        let (amount, rate) = (123_456_i64, 1234.5678_f64);
        let first = apply_rate(amount, rate);
        for _ in 0..1_000 {
            assert_eq!(apply_rate(amount, rate), first);
        }
    }
}

#[cfg(test)]
mod hmac_tests {
    use super::*;

    // RFC 4231 test cases 1, 2 and 6 (a key longer than the block).
    #[test]
    fn matches_rfc_4231_vectors() {
        assert_eq!(
            hex::encode(hmac_sha256(&[0x0b; 20], b"Hi There")),
            "b0344c61d8db38535ca8afceaf0bf12b881dc200c9833da726e9376c2e32cff7"
        );
        assert_eq!(
            hex::encode(hmac_sha256(b"Jefe", b"what do ya want for nothing?")),
            "5bdcc146bf60754e6a042426089575c75a003f089d2739839dec58b964ec3843"
        );
        assert_eq!(
            hex::encode(hmac_sha256(
                &[0xaa; 131],
                b"Test Using Larger Than Block-Size Key - Hash Key First"
            )),
            "60e431591ee0b67f0d8a26aacbf5b77f8e0bc6213728c5140546040f0ee37f54"
        );
    }

    #[test]
    fn constant_time_eq_compares_whole_strings() {
        assert!(constant_time_eq(b"abc", b"abc"));
        assert!(!constant_time_eq(b"abc", b"abd"));
        assert!(!constant_time_eq(b"abc", b"ab"));
    }
}
