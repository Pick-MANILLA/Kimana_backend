#![allow(clippy::inconsistent_digit_grouping)]
mod common;

use axum::http::StatusCode;
use common::TestApp;
use serde_json::json;
use serial_test::file_serial;

#[tokio::test]
#[file_serial]
async fn firm_quote_from_send_amount_derives_receive() {
    let app = TestApp::new().await;
    let (status, body) = app
        .post(
            "/quotes",
            json!({
                "sendCurrency": "USD", "receiveCurrency": "NGN",
                "amount": { "amountMinor": 4_500_000, "currency": "USD" },
                "amountField": "send"
            }),
        )
        .await;
    assert_eq!(status, StatusCode::CREATED);
    assert_eq!(body["breakdown"]["rate"], 1645.2);
    assert_eq!(body["breakdown"]["sendAmount"]["amountMinor"], 4_500_000);
    assert_eq!(
        body["breakdown"]["receiveAmount"]["amountMinor"],
        (4_500_000f64 * 1645.2).round() as i64
    );
    assert_eq!(
        body["breakdown"]["fee"],
        json!({ "amountMinor": 0, "currency": "USD" })
    );
}

#[tokio::test]
#[file_serial]
async fn firm_quote_from_receive_amount_derives_send() {
    let app = TestApp::new().await;
    let (_, body) = app
        .post(
            "/quotes",
            json!({
                "sendCurrency": "USD", "receiveCurrency": "NGN",
                "amount": { "amountMinor": 1_645_200_00i64, "currency": "NGN" },
                "amountField": "receive"
            }),
        )
        .await;
    assert_eq!(
        body["breakdown"]["sendAmount"]["amountMinor"],
        (1_645_200_00f64 / 1645.2).round() as i64
    );
    assert_eq!(
        body["breakdown"]["receiveAmount"]["amountMinor"],
        1_645_200_00i64
    );
}

#[tokio::test]
#[file_serial]
async fn expires_at_is_issued_at_plus_90s() {
    let app = TestApp::new().await;
    let (_, body) = app
        .post(
            "/quotes",
            json!({
                "sendCurrency": "USD", "receiveCurrency": "NGN",
                "amount": { "amountMinor": 100_000, "currency": "USD" }, "amountField": "send"
            }),
        )
        .await;
    let issued = chrono::DateTime::parse_from_rfc3339(body["issuedAt"].as_str().unwrap()).unwrap();
    let expires =
        chrono::DateTime::parse_from_rfc3339(body["expiresAt"].as_str().unwrap()).unwrap();
    assert_eq!((expires - issued).num_seconds(), 90);
}

#[tokio::test]
#[file_serial]
async fn amount_currency_must_match_entered_side() {
    let app = TestApp::new().await;
    let (status, body) = app
        .post(
            "/quotes",
            json!({
                "sendCurrency": "USD", "receiveCurrency": "NGN",
                "amount": { "amountMinor": 100, "currency": "NGN" }, "amountField": "send"
            }),
        )
        .await;
    assert_eq!(status, StatusCode::BAD_REQUEST);
    assert_eq!(body["code"], "VALIDATION");
}

#[tokio::test]
#[file_serial]
async fn unknown_corridor_is_validation() {
    let app = TestApp::new().await;
    let (status, _) = app
        .post(
            "/quotes",
            json!({
                "sendCurrency": "GBP", "receiveCurrency": "EUR",
                "amount": { "amountMinor": 100, "currency": "GBP" }, "amountField": "send"
            }),
        )
        .await;
    assert_eq!(status, StatusCode::BAD_REQUEST);
}

#[tokio::test]
#[file_serial]
async fn same_currency_is_rejected() {
    let app = TestApp::new().await;
    let (status, _) = app
        .post(
            "/quotes",
            json!({
                "sendCurrency": "USD", "receiveCurrency": "USD",
                "amount": { "amountMinor": 100, "currency": "USD" }, "amountField": "send"
            }),
        )
        .await;
    assert_eq!(status, StatusCode::BAD_REQUEST);
}
