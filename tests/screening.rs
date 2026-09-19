#![allow(clippy::inconsistent_digit_grouping)]
mod common;

use axum::http::StatusCode;
use common::TestApp;
use serde_json::{json, Value};
use serial_test::file_serial;

async fn save_recipient(app: &TestApp, country: &str) -> Value {
    let (status, recipient) = app
        .post(
            "/recipients",
            json!({
                "accountNumber": "0123456789",
                "bankCode": "058",
                "currency": "USD",
                "country": country,
                "accountName": "Test Beneficiary",
            }),
        )
        .await;
    assert_eq!(status, StatusCode::CREATED);
    recipient
}

async fn quote_for_amount(app: &TestApp, amount_minor: i64) -> Value {
    let (_, body) = app
        .post(
            "/quotes",
            json!({
                "sendCurrency": "USD", "receiveCurrency": "NGN",
                "amount": { "amountMinor": amount_minor, "currency": "USD" }, "amountField": "send"
            }),
        )
        .await;
    body
}

async fn create_transfer(app: &TestApp, recipient_id: &str, amount_minor: i64, key: &str) -> Value {
    let quote = quote_for_amount(app, amount_minor).await;
    let (status, transfer) = app
        .post(
            "/transfers",
            json!({ "idempotencyKey": key, "quoteId": quote["id"], "recipientId": recipient_id }),
        )
        .await;
    assert_eq!(status, StatusCode::CREATED);
    transfer
}

#[tokio::test]
#[file_serial]
async fn watchlisted_country_holds_and_does_not_advance() {
    let app = TestApp::new().await;
    let recipient = save_recipient(&app, "IR").await;
    let transfer = create_transfer(
        &app,
        recipient["id"].as_str().unwrap(),
        100_000,
        "screening-watchlist-key",
    )
    .await;

    assert_eq!(transfer["state"]["status"], "SCREENED");
    assert_eq!(transfer["state"]["hold"], true);
    assert!(transfer["state"]["holdReason"]
        .as_str()
        .unwrap()
        .contains("IR"));

    let (_, timeline) = app
        .get(&format!(
            "/transfers/{}/timeline",
            transfer["id"].as_str().unwrap()
        ))
        .await;
    let statuses: Vec<&str> = timeline["history"]
        .as_array()
        .unwrap()
        .iter()
        .map(|h| h["status"].as_str().unwrap())
        .collect();
    assert_eq!(statuses, ["CREATED", "QUOTED", "SCREENED"]);
}

#[tokio::test]
#[file_serial]
async fn amount_above_threshold_holds() {
    let app = TestApp::new().await;
    let recipient = save_recipient(&app, "NL").await;
    let transfer = create_transfer(
        &app,
        recipient["id"].as_str().unwrap(),
        5_000_000,
        "screening-amount-key",
    )
    .await;

    assert_eq!(transfer["state"]["status"], "SCREENED");
    assert_eq!(transfer["state"]["hold"], true);
}

#[tokio::test]
#[file_serial]
async fn clean_transfer_still_passes_through_unheld() {
    let app = TestApp::new().await;
    let recipient = save_recipient(&app, "NL").await;
    let transfer = create_transfer(
        &app,
        recipient["id"].as_str().unwrap(),
        100_000,
        "screening-clean-key",
    )
    .await;

    assert_eq!(transfer["state"]["status"], "AWAITING_FUNDS");
}

#[tokio::test]
#[file_serial]
async fn ops_clear_decision_advances_a_held_transfer() {
    let app = TestApp::new().await;
    let recipient = save_recipient(&app, "IR").await;
    let transfer = create_transfer(
        &app,
        recipient["id"].as_str().unwrap(),
        100_000,
        "screening-clear-key",
    )
    .await;
    let id = transfer["id"].as_str().unwrap();

    let (status, decided) = app
        .post(
            &format!("/transfers/{id}/screening/decision"),
            json!({ "decision": "clear" }),
        )
        .await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(decided["state"]["status"], "AWAITING_FUNDS");
}

#[tokio::test]
#[file_serial]
async fn ops_reject_decision_rejects_a_held_transfer() {
    let app = TestApp::new().await;
    let recipient = save_recipient(&app, "IR").await;
    let transfer = create_transfer(
        &app,
        recipient["id"].as_str().unwrap(),
        100_000,
        "screening-reject-key",
    )
    .await;
    let id = transfer["id"].as_str().unwrap();

    let (status, decided) = app
        .post(
            &format!("/transfers/{id}/screening/decision"),
            json!({ "decision": "reject", "reason": "Confirmed sanctions match" }),
        )
        .await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(decided["state"]["status"], "REJECTED");
    assert_eq!(decided["state"]["failureCategory"], "compliance_hold");
}

#[tokio::test]
#[file_serial]
async fn screening_decision_on_a_clean_transfer_is_conflict() {
    let app = TestApp::new().await;
    let recipient = save_recipient(&app, "NL").await;
    let transfer = create_transfer(
        &app,
        recipient["id"].as_str().unwrap(),
        100_000,
        "screening-not-held-key",
    )
    .await;
    let id = transfer["id"].as_str().unwrap();

    let (status, body) = app
        .post(
            &format!("/transfers/{id}/screening/decision"),
            json!({ "decision": "clear" }),
        )
        .await;
    assert_eq!(status, StatusCode::CONFLICT);
    assert_eq!(body["code"], "CONFLICT");
}
