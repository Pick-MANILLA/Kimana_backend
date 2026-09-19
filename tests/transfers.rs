#![allow(clippy::inconsistent_digit_grouping)]
mod common;

use axum::http::StatusCode;
use common::TestApp;
use kimana_backend::ids::DEMO_CUSTOMER_ID;
use serde_json::{json, Value};
use serial_test::file_serial;

const RECIPIENT: &str = "00000000-0000-4000-8000-000000000020";

async fn fresh_quote(app: &TestApp) -> Value {
    quote_for_amount(app, 4_500_000).await
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

#[tokio::test]
#[file_serial]
async fn create_transfer_parks_at_awaiting_funds() {
    let app = TestApp::new().await;
    let quote = fresh_quote(&app).await;
    let (status, t) = app
        .post(
            "/transfers",
            json!({ "idempotencyKey": "idem-key-0001", "quoteId": quote["id"], "recipientId": RECIPIENT }),
        )
        .await;
    assert_eq!(status, StatusCode::CREATED);
    assert!(t["reference"].as_str().unwrap().starts_with("KM-"));
    assert_eq!(t["state"]["status"], "AWAITING_FUNDS");
    assert!(t["state"]["fundingReference"]
        .as_str()
        .unwrap()
        .starts_with("FR-"));
    assert_eq!(t["quote"]["id"], quote["id"]);

    let (_, timeline) = app
        .get(&format!(
            "/transfers/{}/timeline",
            t["id"].as_str().unwrap()
        ))
        .await;
    let statuses: Vec<&str> = timeline["history"]
        .as_array()
        .unwrap()
        .iter()
        .map(|h| h["status"].as_str().unwrap())
        .collect();
    assert_eq!(
        statuses,
        ["CREATED", "QUOTED", "SCREENED", "AWAITING_FUNDS"]
    );
    assert_eq!(timeline["isTerminal"], false);
}

#[tokio::test]
#[file_serial]
async fn replaying_idempotency_key_returns_same_transfer() {
    let app = TestApp::new().await;
    let quote = fresh_quote(&app).await;
    let body = json!({ "idempotencyKey": "idem-key-0002", "quoteId": quote["id"], "recipientId": RECIPIENT });
    let (_, first) = app.post("/transfers", body.clone()).await;
    let (_, second) = app.post("/transfers", body).await;
    assert_eq!(first["id"], second["id"]);

    let n = app
        .scalar_i64(&format!(
            "select count(*)::bigint from transfers where reference = '{}'",
            first["reference"].as_str().unwrap()
        ))
        .await;
    assert_eq!(n, 1);
}

#[tokio::test]
#[file_serial]
async fn expired_quote_is_rate_expired() {
    let app = TestApp::new().await;
    let quote = fresh_quote(&app).await;
    sqlx::query("update quotes set expires_at = now() - interval '1 second' where id = $1")
        .bind(uuid::Uuid::parse_str(quote["id"].as_str().unwrap()).unwrap())
        .execute(&app.pool)
        .await
        .unwrap();
    let (status, body) = app
        .post(
            "/transfers",
            json!({ "idempotencyKey": "idem-key-0003", "quoteId": quote["id"], "recipientId": RECIPIENT }),
        )
        .await;
    assert_eq!(status, StatusCode::CONFLICT);
    assert_eq!(body["code"], "RATE_EXPIRED");
    assert_eq!(body["retryable"], true);
}

#[tokio::test]
#[file_serial]
async fn unknown_quote_or_recipient_is_not_found() {
    let app = TestApp::new().await;
    let quote = fresh_quote(&app).await;
    let (s1, _) = app
        .post(
            "/transfers",
            json!({ "idempotencyKey": "idem-key-0004", "quoteId": "nope", "recipientId": RECIPIENT }),
        )
        .await;
    assert_eq!(s1, StatusCode::NOT_FOUND);
    let (s2, _) = app
        .post(
            "/transfers",
            json!({ "idempotencyKey": "idem-key-0005", "quoteId": quote["id"], "recipientId": "nope" }),
        )
        .await;
    assert_eq!(s2, StatusCode::NOT_FOUND);
}

#[tokio::test]
#[file_serial]
async fn list_transfers_filters_by_status() {
    let app = TestApp::new().await;
    let (_, all) = app.get("/transfers").await;
    assert_eq!(all.as_array().unwrap().len(), 5);

    let (_, completed) = app.get("/transfers?status=COMPLETED").await;
    assert_eq!(completed.as_array().unwrap().len(), 2);
    assert!(completed
        .as_array()
        .unwrap()
        .iter()
        .all(|t| t["state"]["status"] == "COMPLETED"));
}

#[tokio::test]
#[file_serial]
async fn seeded_states_round_trip_their_payload() {
    let app = TestApp::new().await;
    let (_, completed) = app.get("/transfers?status=COMPLETED").await;
    assert!(completed[0]["state"]["payoutReference"]
        .as_str()
        .unwrap()
        .starts_with("PO-"));

    let (_, screened) = app.get("/transfers?status=SCREENED").await;
    assert_eq!(screened[0]["state"]["status"], "SCREENED");
    assert_eq!(screened[0]["state"]["hold"], false);
}

#[tokio::test]
#[file_serial]
async fn timeline_of_unknown_id_is_not_found() {
    let app = TestApp::new().await;
    let (status, _) = app
        .get("/transfers/00000000-0000-4000-8000-0000000000aa")
        .await;
    assert_eq!(status, StatusCode::NOT_FOUND);
}

#[tokio::test]
#[file_serial]
async fn transfer_within_limits_succeeds() {
    let app = TestApp::new().await;
    let quote = fresh_quote(&app).await;
    let (status, t) = app
        .post(
            "/transfers",
            json!({ "idempotencyKey": "idem-key-0006", "quoteId": quote["id"], "recipientId": RECIPIENT }),
        )
        .await;
    assert_eq!(status, StatusCode::CREATED);
    assert_eq!(t["state"]["status"], "AWAITING_FUNDS");
}

#[tokio::test]
#[file_serial]
async fn transfer_over_max_amount_is_rejected() {
    let app = TestApp::new().await;
    let quote = quote_for_amount(&app, 15_000_000).await;
    let (status, body) = app
        .post(
            "/transfers",
            json!({ "idempotencyKey": "idem-key-0007", "quoteId": quote["id"], "recipientId": RECIPIENT }),
        )
        .await;
    assert_eq!(status, StatusCode::BAD_REQUEST);
    assert_eq!(body["code"], "VALIDATION");
}

#[tokio::test]
#[file_serial]
async fn transfer_exceeding_aggregate_exposure_is_rejected() {
    let app = TestApp::new().await;

    // Push the customer's existing (seeded) non-terminal USD exposure close
    // to the aggregate cap, so a small new transfer tips it over while
    // staying well under the per-transfer cap.
    sqlx::query(
        "update transfers set send_amount_minor = 19_500_000
          where reference = 'TXN-8843'",
    )
    .execute(&app.pool)
    .await
    .unwrap();

    let quote = quote_for_amount(&app, 600_000).await;
    let (status, body) = app
        .post(
            "/transfers",
            json!({ "idempotencyKey": "idem-key-0008", "quoteId": quote["id"], "recipientId": RECIPIENT }),
        )
        .await;
    assert_eq!(status, StatusCode::BAD_REQUEST);
    assert_eq!(body["code"], "VALIDATION");
}

#[tokio::test]
#[file_serial]
async fn per_customer_limit_overrides_config_default() {
    let app = TestApp::new().await;
    sqlx::query("update customers set max_transfer_amount_minor = 1_000_000 where id = $1")
        .bind(DEMO_CUSTOMER_ID)
        .execute(&app.pool)
        .await
        .unwrap();

    let quote = fresh_quote(&app).await;
    let (status, body) = app
        .post(
            "/transfers",
            json!({ "idempotencyKey": "idem-key-0009", "quoteId": quote["id"], "recipientId": RECIPIENT }),
        )
        .await;
    assert_eq!(status, StatusCode::BAD_REQUEST);
    assert_eq!(body["code"], "VALIDATION");
}

#[tokio::test]
#[file_serial]
async fn concurrent_creates_cannot_jointly_exceed_aggregate_limit() {
    let app = TestApp::new().await;
    sqlx::query("update transfers set send_amount_minor = 15_000_000 where reference = 'TXN-8843'")
        .execute(&app.pool)
        .await
        .unwrap();

    // Each 3M transfer fits alone (15M + 3M <= 20M) but two together do not.
    let quote_a = quote_for_amount(&app, 3_000_000).await;
    let quote_b = quote_for_amount(&app, 3_000_000).await;
    let (a, b) = tokio::join!(
        app.post(
            "/transfers",
            json!({ "idempotencyKey": "idem-key-0010", "quoteId": quote_a["id"], "recipientId": RECIPIENT }),
        ),
        app.post(
            "/transfers",
            json!({ "idempotencyKey": "idem-key-0011", "quoteId": quote_b["id"], "recipientId": RECIPIENT }),
        ),
    );

    let mut statuses = [a.0, b.0];
    statuses.sort();
    assert_eq!(statuses, [StatusCode::CREATED, StatusCode::BAD_REQUEST]);
}

#[tokio::test]
#[file_serial]
async fn dashboard_stats_come_from_transfers_table() {
    let app = TestApp::new().await;
    let (_, overview) = app.get("/dashboard/overview").await;
    assert_eq!(overview["stats"]["transfersInProgress"], 2);
    assert_eq!(
        overview["stats"]["volume30d"],
        json!({ "amountMinor": 153_500_00i64, "currency": "USD" })
    );
}
