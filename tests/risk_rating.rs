mod common;

use axum::http::StatusCode;
use common::TestApp;
use kimana_backend::domain::onboarding::service::rescreen_approved_customers;
use kimana_backend::ids::DEMO_CUSTOMER_ID;
use serde_json::{json, Value};
use serial_test::file_serial;

const RECIPIENT: &str = "00000000-0000-4000-8000-000000000020";

fn business(legal_name: &str) -> Value {
    json!({
        "legalName": legal_name,
        "cacNumber": "RC-1234567",
        "businessType": "limited_liability_company",
        "industry": "agriculture_agro_export",
        "tradingAddress": { "state": "Lagos", "country": "NG" },
        "countryOfIncorporation": "NG"
    })
}

/// Saves the business and submits, returning the resulting application body
/// regardless of whether KYB approves or rejects it.
async fn submit_with_legal_name(app: &TestApp, legal_name: &str) -> Value {
    app.put(
        "/onboarding/application/business",
        json!({ "business": business(legal_name) }),
    )
    .await;
    let (status, body) = app.post("/onboarding/application/submit", json!({})).await;
    assert_eq!(status, StatusCode::OK);
    body
}

async fn approve_with_legal_name(app: &TestApp, legal_name: &str) -> Value {
    let body = submit_with_legal_name(app, legal_name).await;
    assert_eq!(body["status"], "approved");
    body
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

async fn create_transfer(app: &TestApp, amount_minor: i64, key: &str) -> (StatusCode, Value) {
    let quote = quote_for_amount(app, amount_minor).await;
    app.post(
        "/transfers",
        json!({ "idempotencyKey": key, "quoteId": quote["id"], "recipientId": RECIPIENT }),
    )
    .await
}

#[tokio::test]
#[file_serial]
async fn clean_business_is_approved_with_low_risk_rating() {
    let app = TestApp::new().await;
    let approved = approve_with_legal_name(&app, "Adunola Exports Ltd").await;
    assert_eq!(approved["approvedSummary"]["riskRatingLabel"], "Low");
}

#[tokio::test]
#[file_serial]
async fn high_risk_customer_is_capped_below_platform_default() {
    let app = TestApp::new().await;
    let approved = approve_with_legal_name(&app, "Highrisk Traders Ltd").await;
    assert_eq!(approved["approvedSummary"]["riskRatingLabel"], "High");

    // Above the High-risk limit (1,000,000 minor) but below the platform
    // default (10,000,000 minor) — only the risk-derived limit blocks this.
    let (status, body) = create_transfer(&app, 2_000_000, "high-risk-over-key").await;
    assert_eq!(status, StatusCode::BAD_REQUEST);
    assert_eq!(body["code"], "VALIDATION");

    let (status, body) = create_transfer(&app, 500_000, "high-risk-under-key").await;
    assert_eq!(status, StatusCode::CREATED);
    assert_eq!(body["state"]["status"], "AWAITING_FUNDS");
}

#[tokio::test]
#[file_serial]
async fn medium_risk_customer_gets_a_looser_but_still_real_limit() {
    let app = TestApp::new().await;
    let approved = approve_with_legal_name(&app, "Mediumrisk Traders Ltd").await;
    assert_eq!(approved["approvedSummary"]["riskRatingLabel"], "Medium");

    let (status, body) = create_transfer(&app, 6_000_000, "medium-risk-over-key").await;
    assert_eq!(status, StatusCode::BAD_REQUEST);
    assert_eq!(body["code"], "VALIDATION");
}

#[tokio::test]
#[file_serial]
async fn rescreening_updates_stored_risk_rating_and_limit() {
    let app = TestApp::new().await;
    let approved = approve_with_legal_name(&app, "Adunola Exports Ltd").await;
    assert_eq!(approved["approvedSummary"]["riskRatingLabel"], "Low");

    sqlx::query(
        "update onboarding_applications
            set business = jsonb_set(business, '{legalName}', '\"Highrisk Exports Ltd\"')
          where customer_id = $1",
    )
    .bind(DEMO_CUSTOMER_ID)
    .execute(&app.pool)
    .await
    .unwrap();

    let outcomes = rescreen_approved_customers(&app.state).await.unwrap();
    assert_eq!(outcomes.len(), 1);
    assert_eq!(outcomes[0].risk_rating, "High");
    assert_eq!(outcomes[0].transaction_limit_minor, Some(1_000_000));

    let (_, refreshed) = app.get("/onboarding/application").await;
    assert_eq!(refreshed["approvedSummary"]["riskRatingLabel"], "High");

    let (status, body) = create_transfer(&app, 2_000_000, "rescreened-over-key").await;
    assert_eq!(status, StatusCode::BAD_REQUEST);
    assert_eq!(body["code"], "VALIDATION");
}

#[tokio::test]
#[file_serial]
async fn adverse_media_check_can_fail() {
    let app = TestApp::new().await;
    let rejected = submit_with_legal_name(&app, "Adverse Media Traders Ltd").await;
    assert_eq!(rejected["status"], "rejected");
    let failed: Vec<String> =
        sqlx::query_scalar("select check_key from kyb_checks where passed = false")
            .fetch_all(&app.pool)
            .await
            .unwrap();
    assert!(failed.contains(&"adverse_media".to_string()));
}

#[tokio::test]
#[file_serial]
async fn pep_principal_is_rejected() {
    let app = TestApp::new().await;
    app.put(
        "/onboarding/application/business",
        json!({ "business": business("Adunola Exports Ltd") }),
    )
    .await;
    app.put(
        "/onboarding/application/principals",
        json!({ "principals": [{
            "fullName": "A Known Pep",
            "role": "director",
            "dateOfBirth": "1980-02-02",
            "bvn": "12345678901",
            "nin": "10987654321"
        }] }),
    )
    .await;
    let (_, body) = app.post("/onboarding/application/submit", json!({})).await;
    assert_eq!(body["status"], "rejected");
    let failed: Vec<String> =
        sqlx::query_scalar("select check_key from kyb_checks where passed = false")
            .fetch_all(&app.pool)
            .await
            .unwrap();
    assert!(failed.contains(&"sanctions_pep".to_string()));
}
