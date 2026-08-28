mod common;

use axum::http::StatusCode;
use common::TestApp;
use serde_json::json;
use serial_test::file_serial;

#[tokio::test]
#[file_serial]
async fn indicative_rate_returns_seeded_pair() {
    let app = TestApp::new().await;
    let (status, body) = app.get("/rates/indicative?send=USD&receive=NGN").await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(body["sendCurrency"], "USD");
    assert_eq!(body["rate"], 1645.2);
    assert_eq!(body["changePercent24h"], 0.32);
    assert!(body["asOf"].is_string());
}

#[tokio::test]
#[file_serial]
async fn indicative_rate_unknown_pair_is_validation() {
    let app = TestApp::new().await;
    let (status, body) = app.get("/rates/indicative?send=USD&receive=EUR").await;
    assert_eq!(status, StatusCode::BAD_REQUEST);
    assert_eq!(body["code"], "VALIDATION");
}

#[tokio::test]
#[file_serial]
async fn indicative_rate_bad_currency_is_400() {
    let app = TestApp::new().await;
    let (status, _) = app.get("/rates/indicative?send=USD&receive=XYZ").await;
    assert_eq!(status, StatusCode::BAD_REQUEST);
}

#[tokio::test]
#[file_serial]
async fn list_recipients_returns_five_seeded() {
    let app = TestApp::new().await;
    let (status, body) = app.get("/recipients").await;
    assert_eq!(status, StatusCode::OK);
    let names: Vec<&str> = body
        .as_array()
        .unwrap()
        .iter()
        .map(|r| r["accountName"].as_str().unwrap())
        .collect();
    assert!(names.contains(&"Amsterdam Commodities BV"));
    assert_eq!(body.as_array().unwrap().len(), 5);
}

#[tokio::test]
#[file_serial]
async fn validate_then_save_round_trips() {
    let app = TestApp::new().await;
    let input = json!({ "accountNumber": "9876543210", "bankCode": "058", "currency": "USD", "country": "US" });

    let (vs, vb) = app.post("/recipients/validate", input.clone()).await;
    assert_eq!(vs, StatusCode::OK);
    assert_eq!(vb["accountName"], "Verified Beneficiary (3210)");

    let mut save = input.clone();
    save["accountName"] = vb["accountName"].clone();
    let (ss, sb) = app.post("/recipients", save).await;
    assert_eq!(ss, StatusCode::CREATED);
    assert_eq!(sb["bankName"], "Partner Bank");
    assert_eq!(sb["validationStatus"], "valid");

    let (_, list) = app.get("/recipients").await;
    assert_eq!(list.as_array().unwrap().len(), 6);

    let n = app
        .scalar_i64("select count(*)::bigint from audit_log where action = 'recipient.saved'")
        .await;
    assert_eq!(n, 1);
}

#[tokio::test]
#[file_serial]
async fn validate_rejects_short_account_number() {
    let app = TestApp::new().await;
    let (status, body) = app
        .post(
            "/recipients/validate",
            json!({ "accountNumber": "12", "bankCode": "058", "currency": "USD", "country": "US" }),
        )
        .await;
    assert_eq!(status, StatusCode::BAD_REQUEST);
    assert_eq!(body["code"], "VALIDATION");
}
