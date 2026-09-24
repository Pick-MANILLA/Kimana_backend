#![allow(clippy::inconsistent_digit_grouping)]
mod common;

use axum::http::StatusCode;
use common::TestApp;
use serde_json::{json, Value};
use serial_test::file_serial;

// Seeded: USD/NGN 1645.2, NGN balance 4_825_000_000, USD balance 12_450_000.

async fn usdc_balance(app: &TestApp) -> i64 {
    let (status, body) = app.get("/settlement/balance").await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(body["asset"], "USDC");
    assert_eq!(body["decimals"], 2);
    body["amountMinor"].as_i64().unwrap()
}

async fn buy(app: &TestApp, key: &str, amount_minor: i64, currency: &str) -> (StatusCode, Value) {
    app.post(
        "/settlement/buy",
        json!({
            "idempotencyKey": key,
            "amount": { "amountMinor": amount_minor, "currency": currency },
        }),
    )
    .await
}

async fn convert(app: &TestApp, key: &str, usdc_minor: i64, currency: &str) -> (StatusCode, Value) {
    app.post(
        "/settlement/convert",
        json!({ "idempotencyKey": key, "usdcAmountMinor": usdc_minor, "currency": currency }),
    )
    .await
}

async fn local_balance(app: &TestApp, currency: &str) -> i64 {
    sqlx::query_scalar(
        "select coalesce(sum(le.amount_minor), 0)::bigint
           from accounts a join ledger_entries le on le.account_id = a.id
          where a.currency = $1",
    )
    .bind(currency)
    .fetch_one(&app.pool)
    .await
    .unwrap()
}

#[tokio::test]
#[file_serial]
async fn balance_starts_at_zero() {
    let app = TestApp::new().await;
    assert_eq!(usdc_balance(&app).await, 0);
}

#[tokio::test]
#[file_serial]
async fn buy_debits_local_and_credits_usdc() {
    let app = TestApp::new().await;
    let ngn_before = local_balance(&app, "NGN").await;

    // 164_520_000 kobo = ₦1,645,200 = 1,000 USDC at 1645.2.
    let (status, body) = buy(&app, "buy-key-0001", 164_520_000, "NGN").await;
    assert_eq!(status, StatusCode::CREATED, "{body}");
    assert_eq!(body["kind"], "BUY");
    assert_eq!(body["asset"], "USDC");
    assert_eq!(body["usdcAmountMinor"], 100_000);
    assert_eq!(
        body["localAmount"],
        json!({ "amountMinor": 164_520_000, "currency": "NGN" })
    );
    assert_eq!(body["rate"], 1645.2);
    assert_eq!(body["rateSource"], "live");
    assert!(body["reference"].as_str().unwrap().starts_with("ST-"));

    assert_eq!(usdc_balance(&app).await, 100_000);
    assert_eq!(local_balance(&app, "NGN").await, ngn_before - 164_520_000);
    assert_eq!(
        app.scalar_i64("select count(*) from audit_log where action = 'settlement.buy'")
            .await,
        1
    );
}

#[tokio::test]
#[file_serial]
async fn convert_debits_usdc_and_credits_local() {
    let app = TestApp::new().await;
    buy(&app, "buy-key-0002", 164_520_000, "NGN").await;
    let ngn_before = local_balance(&app, "NGN").await;

    let (status, body) = convert(&app, "conv-key-0001", 40_000, "NGN").await;
    assert_eq!(status, StatusCode::CREATED, "{body}");
    assert_eq!(body["kind"], "CONVERT");
    assert_eq!(body["usdcAmountMinor"], 40_000);
    assert_eq!(body["localAmount"]["amountMinor"], 65_808_000);

    assert_eq!(usdc_balance(&app).await, 60_000);
    assert_eq!(local_balance(&app, "NGN").await, ngn_before + 65_808_000);
    assert_eq!(
        app.scalar_i64("select count(*) from audit_log where action = 'settlement.convert'")
            .await,
        1
    );
}

#[tokio::test]
#[file_serial]
async fn usd_trades_one_to_one() {
    let app = TestApp::new().await;
    let (status, body) = buy(&app, "buy-key-usd1", 12_345, "USD").await;
    assert_eq!(status, StatusCode::CREATED, "{body}");
    assert_eq!(body["usdcAmountMinor"], 12_345);
    assert_eq!(body["rate"], 1.0);
}

#[tokio::test]
#[file_serial]
async fn replayed_key_returns_the_same_trade_once() {
    let app = TestApp::new().await;
    let (_, first) = buy(&app, "buy-key-same", 1_645_200, "NGN").await;
    let (status, second) = buy(&app, "buy-key-same", 1_645_200, "NGN").await;
    assert_eq!(status, StatusCode::CREATED);
    assert_eq!(first["id"], second["id"]);
    assert_eq!(usdc_balance(&app).await, 1_000);
    assert_eq!(
        app.scalar_i64("select count(*) from settlement_trades")
            .await,
        1
    );
}

#[tokio::test]
#[file_serial]
async fn convert_without_usdc_is_rejected() {
    let app = TestApp::new().await;
    let (status, body) = convert(&app, "conv-key-poor", 1, "NGN").await;
    assert_eq!(status, StatusCode::BAD_REQUEST);
    assert_eq!(body["code"], "VALIDATION");
    assert_eq!(body["message"], "Insufficient USDC balance.");
    assert_eq!(
        app.scalar_i64("select count(*) from settlement_trades")
            .await,
        0
    );
}

#[tokio::test]
#[file_serial]
async fn buy_beyond_local_balance_is_rejected() {
    let app = TestApp::new().await;
    let (status, body) = buy(&app, "buy-key-poor", 12_450_001, "USD").await;
    assert_eq!(status, StatusCode::BAD_REQUEST);
    assert_eq!(body["message"], "Insufficient USD balance.");
}

#[tokio::test]
#[file_serial]
async fn rejects_bad_input() {
    let app = TestApp::new().await;

    let (status, body) = buy(&app, "short", 100, "NGN").await;
    assert_eq!(status, StatusCode::BAD_REQUEST);
    assert_eq!(body["message"], "Provide a stable idempotency key.");

    let (status, _) = buy(&app, "buy-key-zero", 0, "NGN").await;
    assert_eq!(status, StatusCode::BAD_REQUEST);

    // Less than one USDC cent's worth of NGN.
    let (status, body) = buy(&app, "buy-key-dust", 1_000, "NGN").await;
    assert_eq!(status, StatusCode::BAD_REQUEST);
    assert_eq!(
        body["message"],
        "That amount is too small to convert at the current rate."
    );

    // No USD/KES rate is seeded.
    let (status, body) = convert(&app, "conv-key-kes1", 100, "KES").await;
    assert_eq!(status, StatusCode::BAD_REQUEST);
    assert_eq!(body["message"], "USDC can't be traded against KES yet.");

    let (status, _) = convert(&app, "conv-key-bad1", 100, "BTC").await;
    assert_eq!(status, StatusCode::BAD_REQUEST);
}

#[tokio::test]
#[file_serial]
async fn transactions_list_newest_first() {
    let app = TestApp::new().await;
    buy(&app, "buy-key-hist", 3_290_400, "NGN").await;
    convert(&app, "conv-key-hist", 500, "NGN").await;

    let (status, body) = app.get("/settlement/transactions").await;
    assert_eq!(status, StatusCode::OK);
    let trades = body.as_array().unwrap();
    assert_eq!(trades.len(), 2);
    assert_eq!(trades[0]["kind"], "CONVERT");
    assert_eq!(trades[1]["kind"], "BUY");
}

#[tokio::test]
#[file_serial]
async fn dashboard_balances_never_show_usdc() {
    let app = TestApp::new().await;
    buy(&app, "buy-key-dash", 1_645_200, "NGN").await;

    let (status, body) = app.get("/dashboard/overview").await;
    assert_eq!(status, StatusCode::OK, "{body}");
    assert!(
        !body.to_string().contains("USDC"),
        "dashboard must not surface the settlement asset"
    );
}

#[tokio::test]
#[file_serial]
async fn requires_a_session() {
    let app = TestApp::new().await;
    app.clear_cookie();
    for path in ["/settlement/balance", "/settlement/transactions"] {
        let (status, _) = app.get(path).await;
        assert_eq!(status, StatusCode::UNAUTHORIZED, "{path}");
    }
    let (status, _) = buy(&app, "buy-key-anon", 100, "USD").await;
    assert_eq!(status, StatusCode::UNAUTHORIZED);
}
