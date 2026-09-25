//! Bridge (bridge.xyz): USD virtual accounts.
//!
//! Requests authenticate with the `Api-Key` header. Webhooks carry
//! `X-Webhook-Signature: t=<unix ms>,v0=<base64 signature>`: an RSA PKCS#1
//! v1.5 SHA-256 signature, under the endpoint's key pair, of the SHA-256
//! digest of `<t>.<raw body>`. The digest is hashed again by the signature
//! scheme, so the verifier hashes twice.

use super::{partner_error, HTTP_TIMEOUT};
use crate::error::ApiResult;
use base64::engine::general_purpose::STANDARD as B64;
use base64::Engine;
use rsa::pkcs1::DecodeRsaPublicKey;
use rsa::pkcs8::DecodePublicKey;
use rsa::{Pkcs1v15Sign, RsaPublicKey};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use sha2::{Digest, Sha256};

/// Bridge's advice: refuse deliveries older than about ten minutes.
pub const WEBHOOK_TOLERANCE_MS: i64 = 10 * 60 * 1000;

/// DER `DigestInfo` prefix for SHA-256 (RFC 8017 §9.2). Spelled out because
/// `rsa` 0.9 only derives it from a `digest` 0.10 hasher.
const SHA256_DIGEST_INFO: [u8; 19] = [
    0x30, 0x31, 0x30, 0x0d, 0x06, 0x09, 0x60, 0x86, 0x48, 0x01, 0x65, 0x03, 0x04, 0x02, 0x01, 0x05,
    0x00, 0x04, 0x20,
];

pub fn pkcs1v15_sha256() -> Pkcs1v15Sign {
    Pkcs1v15Sign {
        hash_len: Some(32),
        prefix: Box::new(SHA256_DIGEST_INFO),
    }
}

/// Parses an SPKI (`BEGIN PUBLIC KEY`) or PKCS#1 (`BEGIN RSA PUBLIC KEY`) PEM.
pub fn parse_public_key(pem: &str) -> Result<RsaPublicKey, String> {
    let pem = pem.trim();
    RsaPublicKey::from_public_key_pem(pem)
        .or_else(|_| RsaPublicKey::from_pkcs1_pem(pem))
        .map_err(|err| format!("invalid BRIDGE_WEBHOOK_PUBLIC_KEY: {err}"))
}

/// The bytes Bridge signs: SHA-256 of `<t>.<body>`, hashed once more.
pub fn signed_hash(timestamp: &str, body: &[u8]) -> [u8; 32] {
    let mut inner = Sha256::new();
    inner.update(timestamp.as_bytes());
    inner.update(b".");
    inner.update(body);
    Sha256::digest(inner.finalize()).into()
}

#[derive(Debug, PartialEq, Eq)]
pub enum WebhookCheck {
    Valid,
    Invalid,
    Stale,
}

pub fn verify_webhook(key: &RsaPublicKey, header: &str, body: &[u8], now_ms: i64) -> WebhookCheck {
    let mut timestamp = None;
    let mut signature = None;
    for part in header.split(',') {
        match part.trim().split_once('=') {
            Some(("t", v)) => timestamp = Some(v),
            Some(("v0", v)) => signature = Some(v),
            _ => {}
        }
    }
    let (Some(timestamp), Some(signature)) = (timestamp, signature) else {
        return WebhookCheck::Invalid;
    };
    let Ok(sent_ms) = timestamp.parse::<i64>() else {
        return WebhookCheck::Invalid;
    };
    let Ok(signature) = B64.decode(signature) else {
        return WebhookCheck::Invalid;
    };
    if key
        .verify(pkcs1v15_sha256(), &signed_hash(timestamp, body), &signature)
        .is_err()
    {
        return WebhookCheck::Invalid;
    }
    if (now_ms - sent_ms).abs() > WEBHOOK_TOLERANCE_MS {
        return WebhookCheck::Stale;
    }
    WebhookCheck::Valid
}

#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct DepositInstructions {
    #[serde(default)]
    pub currency: Option<String>,
    #[serde(default)]
    pub payment_rails: Vec<String>,
    #[serde(default)]
    pub bank_name: Option<String>,
    #[serde(default)]
    pub bank_address: Option<String>,
    #[serde(default)]
    pub bank_beneficiary_name: Option<String>,
    #[serde(default)]
    pub bank_beneficiary_address: Option<String>,
    #[serde(default)]
    pub bank_account_number: Option<String>,
    #[serde(default)]
    pub bank_routing_number: Option<String>,
}

#[derive(Debug, Clone, Deserialize)]
pub struct VirtualAccount {
    pub id: String,
    #[serde(default)]
    pub status: Option<String>,
    pub source_deposit_instructions: DepositInstructions,
}

/// A `virtual_account.activity` event object.
#[derive(Debug, Clone, Deserialize)]
pub struct Activity {
    pub id: String,
    #[serde(rename = "type")]
    pub kind: String,
    pub virtual_account_id: String,
    /// Dollars as a decimal string, e.g. `"1970.0"`.
    pub amount: String,
    #[serde(default)]
    pub currency: Option<String>,
    #[serde(default)]
    pub deposit_id: Option<String>,
    #[serde(default)]
    pub created_at: Option<String>,
    #[serde(default)]
    pub source: Option<ActivitySource>,
}

#[derive(Debug, Clone, Deserialize)]
pub struct ActivitySource {
    #[serde(default)]
    pub sender_name: Option<String>,
    #[serde(default)]
    pub description: Option<String>,
}

#[derive(Debug, Clone, Deserialize)]
pub struct Event {
    pub event_id: String,
    pub event_category: String,
    pub event_object: Value,
}

#[derive(Clone)]
pub struct Bridge {
    http: reqwest::Client,
    base_url: String,
    api_key: String,
    destination_rail: String,
    destination_currency: String,
    destination_address: Option<String>,
}

impl Bridge {
    /// `None` unless `BRIDGE_API_KEY` is set.
    pub fn from_config(config: &crate::config::Config) -> Option<Self> {
        Some(Bridge {
            http: reqwest::Client::builder()
                .timeout(HTTP_TIMEOUT)
                .build()
                .expect("reqwest client builds"),
            base_url: config.bridge_base_url.trim_end_matches('/').to_string(),
            api_key: config.bridge_api_key.clone()?,
            destination_rail: config.bridge_destination_rail.clone(),
            destination_currency: config.bridge_destination_currency.clone(),
            destination_address: config.bridge_destination_address.clone(),
        })
    }

    /// Opens a USD virtual account for an existing (KYB-approved) Bridge
    /// customer. `idempotency_key` makes a retried link return the same one.
    pub async fn create_virtual_account(
        &self,
        bridge_customer_id: &str,
        idempotency_key: &str,
    ) -> ApiResult<(VirtualAccount, Value)> {
        let path = format!("/customers/{bridge_customer_id}/virtual_accounts");
        let address = self.destination_address.as_deref().ok_or_else(|| {
            partner_error(
                "bridge",
                &path,
                "set BRIDGE_DESTINATION_ADDRESS (or SETTLEMENT_VAULT_ADDRESS)",
            )
        })?;
        let body = serde_json::json!({
            "source": { "currency": "usd" },
            "destination": {
                "currency": self.destination_currency,
                "payment_rail": self.destination_rail,
                "address": address,
            },
        });
        let response = self
            .http
            .post(format!("{}{path}", self.base_url))
            .header("Api-Key", &self.api_key)
            .header("Idempotency-Key", idempotency_key)
            .json(&body)
            .send()
            .await
            .map_err(|err| partner_error("bridge", &path, err))?;
        let status = response.status();
        let text = response
            .text()
            .await
            .map_err(|err| partner_error("bridge", &path, err))?;
        if !status.is_success() {
            return Err(partner_error(
                "bridge",
                &path,
                format!("HTTP {status}: {text}"),
            ));
        }
        let raw: Value =
            serde_json::from_str(&text).map_err(|err| partner_error("bridge", &path, err))?;
        let account = serde_json::from_value(raw.clone())
            .map_err(|err| partner_error("bridge", &path, err))?;
        Ok((account, raw))
    }
}

/// A non-negative decimal string (`"1970.0"`, `"12.345678"`) to cents,
/// flooring digits past the second so rounding never creates value.
pub fn dollars_to_cents(value: &str) -> Option<i64> {
    let value = value.trim();
    let (whole, frac) = value.split_once('.').unwrap_or((value, ""));
    if whole.is_empty() && frac.is_empty() {
        return None;
    }
    if !whole.bytes().all(|b| b.is_ascii_digit()) || !frac.bytes().all(|b| b.is_ascii_digit()) {
        return None;
    }
    let whole: i64 = if whole.is_empty() {
        0
    } else {
        whole.parse().ok()?
    };
    let cents: i64 = format!("{frac:0<2}")[..2].parse().ok()?;
    whole.checked_mul(100)?.checked_add(cents)
}

#[cfg(test)]
mod tests {
    use super::*;
    use rsa::RsaPrivateKey;

    #[test]
    fn parses_dollar_strings_to_cents() {
        assert_eq!(dollars_to_cents("1970.0"), Some(197_000));
        assert_eq!(dollars_to_cents("123.45"), Some(12_345));
        assert_eq!(dollars_to_cents("12.345678"), Some(1_234));
        assert_eq!(dollars_to_cents("5"), Some(500));
        assert_eq!(dollars_to_cents(".5"), Some(50));
        assert_eq!(dollars_to_cents("-1.00"), None);
        assert_eq!(dollars_to_cents("1e3"), None);
        assert_eq!(dollars_to_cents(""), None);
    }

    fn sign(key: &RsaPrivateKey, timestamp: &str, body: &[u8]) -> String {
        let signature = key
            .sign(pkcs1v15_sha256(), &signed_hash(timestamp, body))
            .unwrap();
        format!("t={timestamp},v0={}", B64.encode(signature))
    }

    #[test]
    fn verifies_signed_deliveries() {
        let key = RsaPrivateKey::new(&mut rand::thread_rng(), 1024).unwrap();
        let public = key.to_public_key();
        let body = br#"{"event_id":"wh_1"}"#;
        let now = 1_750_000_000_000_i64;
        let header = sign(&key, &now.to_string(), body);

        assert_eq!(
            verify_webhook(&public, &header, body, now),
            WebhookCheck::Valid
        );
        assert_eq!(
            verify_webhook(&public, &header, b"{}", now),
            WebhookCheck::Invalid
        );
        assert_eq!(
            verify_webhook(&public, &header, body, now + WEBHOOK_TOLERANCE_MS + 1),
            WebhookCheck::Stale
        );
        // A forged timestamp breaks the signature.
        let forged = header.replace(&now.to_string(), &(now + 1).to_string());
        assert_eq!(
            verify_webhook(&public, &forged, body, now),
            WebhookCheck::Invalid
        );
        assert_eq!(
            verify_webhook(&public, "garbage", body, now),
            WebhookCheck::Invalid
        );
    }
}
