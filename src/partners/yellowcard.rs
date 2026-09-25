//! Yellow Card Payments API: NGN collections ("receives").
//!
//! Every request is signed `YcHmacV1 {apiKey}:{signature}`, where the
//! signature is base64 HMAC-SHA256, under the API secret, of
//! `timestamp + path + METHOD (+ base64 SHA-256 of the body for POST/PUT)`.
//! Webhooks carry `X-YC-Signature`: base64 HMAC-SHA256 of the raw body under
//! the same secret.

use super::{partner_error, HTTP_TIMEOUT};
use crate::error::ApiResult;
use crate::util::{constant_time_eq, hmac_sha256, iso};
use base64::engine::general_purpose::STANDARD as B64;
use base64::Engine;
use chrono::Utc;
use reqwest::Method;
use serde::de::DeserializeOwned;
use serde::{Deserialize, Serialize};
use serde_json::Value;
use sha2::{Digest, Sha256};

#[derive(Clone)]
pub struct YellowCard {
    http: reqwest::Client,
    base_url: String,
    api_key: String,
    api_secret: String,
    channel_id: Option<String>,
    source_account: Option<String>,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Channel {
    pub id: String,
    #[serde(default)]
    pub ramp_type: Option<String>,
    #[serde(default)]
    pub channel_type: Option<String>,
    #[serde(default)]
    pub country: Option<String>,
    #[serde(default)]
    pub currency: Option<String>,
    #[serde(default)]
    pub status: Option<String>,
    #[serde(default)]
    pub api_status: Option<String>,
}

#[derive(Deserialize)]
struct Channels {
    channels: Vec<Channel>,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct BankInfo {
    #[serde(default)]
    pub name: Option<String>,
    #[serde(default)]
    pub account_number: Option<String>,
    #[serde(default)]
    pub account_name: Option<String>,
    #[serde(default)]
    pub payment_link: Option<String>,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ReceiveSource {
    #[serde(default)]
    pub account_name: Option<String>,
}

/// The subset of a receive this backend reads. The raw JSON is kept too.
#[derive(Debug, Clone, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Receive {
    pub id: String,
    #[serde(default)]
    pub sequence_id: Option<String>,
    pub status: String,
    #[serde(default)]
    pub currency: Option<String>,
    /// USD value.
    #[serde(default)]
    pub amount: Option<f64>,
    /// Local-currency value (NGN, in naira).
    #[serde(default)]
    pub converted_amount: Option<f64>,
    #[serde(default)]
    pub expires_at: Option<String>,
    #[serde(default)]
    pub updated_at: Option<String>,
    #[serde(default)]
    pub bank_info: Option<BankInfo>,
    #[serde(default)]
    pub source: Option<ReceiveSource>,
    #[serde(default)]
    pub error_code: Option<String>,
}

/// A new NGN receive. `sequence_id` is the collection id.
pub struct NewReceive<'a> {
    pub sequence_id: &'a str,
    pub amount_minor: i64,
    pub reason: &'a str,
    pub business_name: &'a str,
    pub email: Option<&'a str>,
}

impl YellowCard {
    /// `None` unless both the API key and secret are configured.
    pub fn from_config(config: &crate::config::Config) -> Option<Self> {
        Some(YellowCard {
            http: reqwest::Client::builder()
                .timeout(HTTP_TIMEOUT)
                .build()
                .expect("reqwest client builds"),
            base_url: config.yellowcard_base_url.trim_end_matches('/').to_string(),
            api_key: config.yellowcard_api_key.clone()?,
            api_secret: config.yellowcard_api_secret.clone()?,
            channel_id: config.yellowcard_channel_id.clone(),
            source_account: config.yellowcard_source_account.clone(),
        })
    }

    /// Checks a webhook's `X-YC-Signature` against the raw body.
    pub fn verify_webhook(&self, signature: &str, body: &[u8]) -> bool {
        verify_webhook(&self.api_secret, signature, body)
    }

    async fn call<T: DeserializeOwned>(
        &self,
        method: Method,
        path: &str,
        body: Option<&Value>,
    ) -> ApiResult<(T, Value)> {
        let body = body.map(serde_json::to_vec).transpose()?;
        let timestamp = iso(Utc::now());
        let signature = sign(
            &self.api_secret,
            &timestamp,
            path,
            method.as_str(),
            body.as_deref(),
        );
        let mut request = self
            .http
            .request(method, format!("{}{path}", self.base_url))
            .header("X-YC-Timestamp", &timestamp)
            .header(
                "Authorization",
                format!("YcHmacV1 {}:{signature}", self.api_key),
            );
        if let Some(body) = body {
            request = request
                .header("Content-Type", "application/json")
                .body(body);
        }
        let response = request
            .send()
            .await
            .map_err(|err| partner_error("yellowcard", path, err))?;
        let status = response.status();
        let text = response
            .text()
            .await
            .map_err(|err| partner_error("yellowcard", path, err))?;
        if !status.is_success() {
            return Err(partner_error(
                "yellowcard",
                path,
                format!("HTTP {status}: {text}"),
            ));
        }
        let raw: Value =
            serde_json::from_str(&text).map_err(|err| partner_error("yellowcard", path, err))?;
        let parsed = serde_json::from_value(raw.clone())
            .map_err(|err| partner_error("yellowcard", path, err))?;
        Ok((parsed, raw))
    }

    /// The channel NGN bank-transfer receives go through: the configured one,
    /// else the first active `deposit`/`bank` channel for NG.
    pub async fn ngn_bank_channel(&self) -> ApiResult<String> {
        if let Some(id) = &self.channel_id {
            return Ok(id.clone());
        }
        // Filtered here rather than with `?country=`: the signed path is the
        // bare path, and the docs don't say whether a query string joins it.
        let (channels, _): (Channels, _) =
            self.call(Method::GET, "/business/channels", None).await?;
        let active = |s: &Option<String>| s.as_deref().is_none_or(|s| s == "active");
        channels
            .channels
            .into_iter()
            .find(|c| {
                c.country.as_deref() == Some("NG")
                    && c.currency.as_deref().is_none_or(|v| v == "NGN")
                    && c.ramp_type.as_deref() == Some("deposit")
                    && c.channel_type.as_deref() == Some("bank")
                    && active(&c.status)
                    && active(&c.api_status)
            })
            .map(|c| c.id)
            .ok_or_else(|| {
                partner_error(
                    "yellowcard",
                    "/business/channels",
                    "no NGN bank deposit channel",
                )
            })
    }

    /// Submits a force-accepted NGN bank-transfer receive; the response
    /// carries the account the payer transfers into.
    pub async fn submit_receive(&self, new: NewReceive<'_>) -> ApiResult<(Receive, Value)> {
        let channel_id = self.ngn_bank_channel().await?;
        let mut source = serde_json::json!({ "accountType": "bank" });
        if let Some(account) = &self.source_account {
            source["accountNumber"] = account.clone().into();
        }
        let mut recipient = serde_json::json!({
            "name": new.business_name,
            "businessName": new.business_name,
            "country": "NG",
        });
        if let Some(email) = new.email {
            recipient["email"] = email.into();
        }
        let body = serde_json::json!({
            "sequenceId": new.sequence_id,
            "channelId": channel_id,
            "channelType": "bank",
            "currency": "NGN",
            "country": "NG",
            "localAmount": minor_to_major(new.amount_minor),
            "reason": new.reason,
            "customerType": "institution",
            "forceAccept": true,
            "source": source,
            "recipient": recipient,
        });
        self.call(Method::POST, "/business/receive", Some(&body))
            .await
    }

    pub async fn get_receive(&self, id: &str) -> ApiResult<(Receive, Value)> {
        self.call(Method::GET, &format!("/business/receive/{id}"), None)
            .await
    }

    pub async fn cancel_receive(&self, id: &str) -> ApiResult<()> {
        let _: (Value, Value) = self
            .call(
                Method::POST,
                &format!("/business/receive/{id}/cancel"),
                Some(&serde_json::json!({})),
            )
            .await?;
        Ok(())
    }
}

/// Minor units (kobo) to the decimal naira amount Yellow Card takes.
fn minor_to_major(amount_minor: i64) -> f64 {
    amount_minor as f64 / 100.0
}

/// A decimal naira amount from Yellow Card to kobo.
pub fn major_to_minor(amount: f64) -> Option<i64> {
    let minor = (amount * 100.0).round();
    (amount.is_finite() && minor > 0.0 && minor < i64::MAX as f64).then_some(minor as i64)
}

/// The `YcHmacV1` request signature.
pub fn sign(
    secret: &str,
    timestamp: &str,
    path: &str,
    method: &str,
    body: Option<&[u8]>,
) -> String {
    let mut message = format!("{timestamp}{path}{}", method.to_ascii_uppercase());
    if let Some(body) = body {
        message.push_str(&B64.encode(Sha256::digest(body)));
    }
    B64.encode(hmac_sha256(secret.as_bytes(), message.as_bytes()))
}

pub fn verify_webhook(secret: &str, signature: &str, body: &[u8]) -> bool {
    let Ok(given) = B64.decode(signature.trim()) else {
        return false;
    };
    constant_time_eq(&given, &hmac_sha256(secret.as_bytes(), body))
}

/// Receive and legacy collection events; everything else is ignored.
pub fn is_receive_event(event: &str) -> bool {
    let event = event.to_ascii_uppercase();
    event.starts_with("RECEIVE.") || event.starts_with("COLLECTION.")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn request_signature_covers_body_hash_only_when_present() {
        let get = sign(
            "s",
            "2026-01-01T00:00:00.000Z",
            "/business/channels",
            "get",
            None,
        );
        let expected = B64.encode(hmac_sha256(
            b"s",
            b"2026-01-01T00:00:00.000Z/business/channelsGET",
        ));
        assert_eq!(get, expected);

        let body = br#"{"a":1}"#;
        let post = sign(
            "s",
            "2026-01-01T00:00:00.000Z",
            "/business/receive",
            "POST",
            Some(body),
        );
        let message = format!(
            "2026-01-01T00:00:00.000Z/business/receivePOST{}",
            B64.encode(Sha256::digest(body))
        );
        assert_eq!(post, B64.encode(hmac_sha256(b"s", message.as_bytes())));
    }

    #[test]
    fn webhook_signature_round_trips() {
        let body = br#"{"id":"r1","event":"RECEIVE.COMPLETE"}"#;
        let signature = B64.encode(hmac_sha256(b"secret", body));
        assert!(verify_webhook("secret", &signature, body));
        assert!(!verify_webhook("other", &signature, body));
        assert!(!verify_webhook("secret", &signature, b"{}"));
        assert!(!verify_webhook("secret", "not base64!", body));
    }

    #[test]
    fn naira_amounts_convert_exactly() {
        assert_eq!(minor_to_major(5_000_000), 50_000.0);
        assert_eq!(major_to_minor(50_000.0), Some(5_000_000));
        assert_eq!(major_to_minor(1234.56), Some(123_456));
        assert_eq!(major_to_minor(0.0), None);
        assert_eq!(major_to_minor(f64::NAN), None);
    }

    #[test]
    fn recognises_receive_events() {
        assert!(is_receive_event("RECEIVE.COMPLETE"));
        assert!(is_receive_event("collection.complete"));
        assert!(!is_receive_event("PAYMENT.COMPLETE"));
    }
}
