#![allow(clippy::inconsistent_digit_grouping)]
mod common;

use axum::http::StatusCode;
use common::TestApp;
use kimana_backend::domain::transfers::engine;
use serde_json::json;
use serial_test::file_serial;
use uuid::Uuid;

const RECIPIENT: &str = "00000000-0000-4000-8000-000000000020";
const ACCT_USD: &str = "00000000-0000-4000-8000-000000000011";
const ACCT_NGN: &str = "00000000-0000-4000-8000-000000000010";

async fn create_transfer(app: &TestApp, send_minor: i64) -> (Uuid, i64) {
    let (_, quote) = app
        .post(
            "/quotes",
            json!({
                "sendCurrency": "USD", "receiveCurrency": "NGN",
                "amount": { "amountMinor": send_minor, "currency": "USD" }, "amountField": "send"
            }),
        )
        .await;
    let (_, t) = app
        .post(
            "/transfers",
            json!({
                "idempotencyKey": format!("engine-{send_minor}-key"),
                "quoteId": quote["id"], "recipientId": RECIPIENT
            }),
        )
        .await;
    (
        Uuid::parse_str(t["id"].as_str().unwrap()).unwrap(),
        quote["breakdown"]["receiveAmount"]["amountMinor"]
            .as_i64()
            .unwrap(),
    )
}

async fn balance(app: &TestApp, account: &str) -> i64 {
    app.scalar_i64(&format!(
        "select coalesce(sum(amount_minor),0)::bigint from ledger_entries where account_id = '{account}'"
    ))
    .await
}

#[tokio::test]
#[file_serial]
async fn happy_path_posts_three_ledger_entries() {
    let app = TestApp::new().await;
    let (id, receive_minor) = create_transfer(&app, 5_000_00).await;

    let final_status = engine::advance_to_completion(&app.state, id, 0)
        .await
        .unwrap();
    assert_eq!(final_status.as_str(), "COMPLETED");

    let (_, t) = app.get(&format!("/transfers/{id}")).await;
    assert_eq!(t["state"]["status"], "COMPLETED");
    assert!(t["state"]["payoutReference"]
        .as_str()
        .unwrap()
        .starts_with("PO-"));

    let entries: Vec<(i64, String)> = sqlx::query_as(
        "select amount_minor, description from ledger_entries where transfer_id = $1 order by posted_at, id",
    )
    .bind(id)
    .fetch_all(&app.pool)
    .await
    .unwrap();
    assert_eq!(entries.len(), 3);
    assert_eq!(
        entries.iter().map(|(a, _)| *a).collect::<Vec<_>>(),
        vec![-5_000_00, receive_minor, -receive_minor]
    );
    assert!(entries[0].1.contains("funded"));
    assert!(entries[2].1.contains("paid to Amsterdam Commodities BV"));

    assert_eq!(balance(&app, ACCT_USD).await, 12_450_000 - 5_000_00);
    assert_eq!(balance(&app, ACCT_NGN).await, 4_825_000_000);
}

#[tokio::test]
#[file_serial]
async fn running_balance_tracks_cumulative_total() {
    let app = TestApp::new().await;
    let (id, _) = create_transfer(&app, 3_000_00).await;
    engine::advance_to_completion(&app.state, id, 0)
        .await
        .unwrap();

    let rows: Vec<(Uuid, i64, i64)> = sqlx::query_as(
        "select account_id, amount_minor, running_balance_minor
           from ledger_entries order by account_id, posted_at, id",
    )
    .fetch_all(&app.pool)
    .await
    .unwrap();

    let mut seen: std::collections::HashMap<Uuid, i64> = std::collections::HashMap::new();
    for (acct, amount, running) in rows {
        let next = seen.get(&acct).copied().unwrap_or(0) + amount;
        assert_eq!(running, next);
        seen.insert(acct, next);
    }
}

#[tokio::test]
#[file_serial]
async fn insufficient_balance_rejects_at_funding() {
    let app = TestApp::new().await;
    let (id, _) = create_transfer(&app, 200_000_00).await; // USD balance is 124,500.00
    let final_status = engine::advance_to_completion(&app.state, id, 0)
        .await
        .unwrap();
    assert_eq!(final_status.as_str(), "REJECTED");

    let (_, t) = app.get(&format!("/transfers/{id}")).await;
    assert_eq!(t["state"]["status"], "REJECTED");
    assert_eq!(t["state"]["failureCategory"], "validation");
    assert_eq!(t["state"]["reasonCode"], "INSUFFICIENT_FUNDS");

    let n = app
        .scalar_i64(&format!(
            "select count(*)::bigint from ledger_entries where transfer_id = '{id}'"
        ))
        .await;
    assert_eq!(n, 0);
    assert_eq!(balance(&app, ACCT_USD).await, 12_450_000);
}

#[tokio::test]
#[file_serial]
async fn timeline_after_completion_is_full_history() {
    let app = TestApp::new().await;
    let (id, _) = create_transfer(&app, 1_000_00).await;
    engine::advance_to_completion(&app.state, id, 0)
        .await
        .unwrap();

    let (_, timeline) = app.get(&format!("/transfers/{id}/timeline")).await;
    let statuses: Vec<&str> = timeline["history"]
        .as_array()
        .unwrap()
        .iter()
        .map(|h| h["status"].as_str().unwrap())
        .collect();
    assert_eq!(
        statuses,
        [
            "CREATED",
            "QUOTED",
            "SCREENED",
            "AWAITING_FUNDS",
            "FUNDED",
            "SETTLING",
            "SETTLED",
            "PAYING_OUT",
            "COMPLETED"
        ]
    );
    assert_eq!(timeline["isTerminal"], true);
}

#[tokio::test]
#[file_serial]
async fn reverse_from_completed_makes_customer_whole() {
    let app = TestApp::new().await;
    let (id, _) = create_transfer(&app, 4_000_00).await;
    engine::advance_to_completion(&app.state, id, 0)
        .await
        .unwrap();
    assert_eq!(balance(&app, ACCT_USD).await, 12_450_000 - 4_000_00);

    let reversal_id = engine::reverse_transfer(&app.state, id, "Beneficiary account closed", None)
        .await
        .unwrap();

    let (_, t) = app.get(&format!("/transfers/{id}")).await;
    assert_eq!(t["state"]["status"], "REVERSED");
    assert_eq!(t["state"]["reason"], "Beneficiary account closed");
    assert_eq!(t["state"]["reversalLedgerEntryId"], reversal_id);
    assert_eq!(balance(&app, ACCT_USD).await, 12_450_000);

    let (amount, reversal_of): (i64, Option<Uuid>) = sqlx::query_as(
        "select amount_minor, reversal_of_entry_id from ledger_entries where id = $1",
    )
    .bind(Uuid::parse_str(&reversal_id).unwrap())
    .fetch_one(&app.pool)
    .await
    .unwrap();
    assert_eq!(amount, 4_000_00);
    assert!(reversal_of.is_some());
}

#[tokio::test]
#[file_serial]
async fn illegal_transition_is_conflict() {
    let app = TestApp::new().await;
    let (id, _) = create_transfer(&app, 1_000_00).await; // parked at AWAITING_FUNDS
    let err = engine::reverse_transfer(&app.state, id, "too early", None)
        .await
        .unwrap_err();
    assert_eq!(err.code, kimana_backend::error::ErrorCode::Conflict);
}

#[tokio::test]
#[file_serial]
async fn expire_from_awaiting_funds_has_no_ledger_effect() {
    let app = TestApp::new().await;
    let (id, _) = create_transfer(&app, 1_000_00).await;
    engine::expire_transfer(&app.state, id, None).await.unwrap();

    let (_, t) = app.get(&format!("/transfers/{id}")).await;
    assert_eq!(t["state"]["status"], "EXPIRED");
    let n = app
        .scalar_i64(&format!(
            "select count(*)::bigint from ledger_entries where transfer_id = '{id}'"
        ))
        .await;
    assert_eq!(n, 0);
}

#[tokio::test]
#[file_serial]
async fn completed_transfer_shows_in_dashboard_balances() {
    let app = TestApp::new().await;
    let (id, _) = create_transfer(&app, 10_000_00).await;
    engine::advance_to_completion(&app.state, id, 0)
        .await
        .unwrap();

    let (_, overview) = app.get("/dashboard/overview").await;
    let usd = overview["balances"]
        .as_array()
        .unwrap()
        .iter()
        .find(|b| b["currency"] == "USD")
        .unwrap();
    assert_eq!(usd["balance"]["amountMinor"], 12_450_000 - 10_000_00);
}

#[tokio::test]
#[file_serial]
async fn create_transfer_from_header_idempotency_key() {
    let app = TestApp::new().await;
    let (_, quote) = app
        .post(
            "/quotes",
            json!({
                "sendCurrency": "USD", "receiveCurrency": "NGN",
                "amount": { "amountMinor": 500_000, "currency": "USD" }, "amountField": "send"
            }),
        )
        .await;

    let request = axum::http::Request::builder()
        .method("POST")
        .uri("/transfers")
        .header("content-type", "application/json")
        .header("idempotency-key", "header-key-000001")
        .body(axum::body::Body::from(
            json!({ "quoteId": quote["id"], "recipientId": RECIPIENT }).to_string(),
        ))
        .unwrap();
    let response = tower::ServiceExt::oneshot(app.router_clone(), request)
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::CREATED);
    let bytes = http_body_util::BodyExt::collect(response.into_body())
        .await
        .unwrap()
        .to_bytes();
    let t: serde_json::Value = serde_json::from_slice(&bytes).unwrap();
    assert_eq!(t["idempotencyKey"], "header-key-000001");
}
