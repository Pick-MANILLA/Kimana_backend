mod common;

use axum::http::StatusCode;
use common::TestApp;
use serde_json::json;
use serial_test::file_serial;

#[tokio::test]
#[file_serial]
async fn session_returns_seeded_demo_customer() {
    let app = TestApp::new().await;
    let (status, body) = app.get("/session").await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(body["role"], "customer");
    assert_eq!(body["displayName"], "Chinonso");
}

#[tokio::test]
#[file_serial]
async fn overview_sums_ledger_entries_into_balances() {
    let app = TestApp::new().await;
    let (status, body) = app.get("/dashboard/overview").await;
    assert_eq!(status, StatusCode::OK);

    let mut by_currency = std::collections::HashMap::new();
    for b in body["balances"].as_array().unwrap() {
        by_currency.insert(
            b["currency"].as_str().unwrap().to_string(),
            b["balance"]["amountMinor"].as_i64().unwrap(),
        );
    }
    assert_eq!(by_currency["NGN"], 4_825_000_000);
    assert_eq!(by_currency["USD"], 12_450_000);
    assert_eq!(by_currency["EUR"], 1_820_000);

    assert_eq!(body["displayName"], "Chinonso");
    assert_eq!(body["businessName"], "Adunola Exports Ltd");
    assert_eq!(body["accountId"], "AEL-00029");
    assert_eq!(body["stats"]["payoutSuccessRatePercent"], 98.3);
    assert_eq!(body["pendingActions"].as_array().unwrap().len(), 3);
    assert_eq!(body["workingCapitalOffer"]["maxAdvance"]["currency"], "USD");
}

#[tokio::test]
#[file_serial]
async fn overview_reflects_onboarding_after_save() {
    let app = TestApp::new().await;
    app.put(
        "/onboarding/application/business",
        json!({ "business": {
            "legalName": "Kano Cashew Traders",
            "cacNumber": "RC-9090909",
            "businessType": "partnership",
            "industry": "trading_commodities",
            "tradingAddress": { "state": "Kano", "country": "NG" },
            "countryOfIncorporation": "NG"
        }}),
    )
    .await;
    app.put(
        "/onboarding/application/principals",
        json!({ "principals": [{
            "fullName": "Amina Bello", "role": "director",
            "dateOfBirth": "1990-01-01", "bvn": "11111111111", "nin": "22222222222"
        }] }),
    )
    .await;

    let (_, body) = app.get("/dashboard/overview").await;
    assert_eq!(body["businessName"], "Kano Cashew Traders");
    assert_eq!(body["displayName"], "Amina");
}

#[tokio::test]
#[file_serial]
async fn health_needs_no_database_session() {
    let app = TestApp::new().await;
    let (_, body) = app.get("/health").await;
    assert_eq!(body, json!({ "ok": true }));
}
