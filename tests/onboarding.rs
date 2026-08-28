#![allow(clippy::inconsistent_digit_grouping)]
mod common;

use axum::http::StatusCode;
use common::TestApp;
use serde_json::json;
use serial_test::file_serial;

fn valid_business() -> serde_json::Value {
    json!({
        "legalName": "Adunola Exports Ltd",
        "cacNumber": "RC-1234567",
        "businessType": "limited_liability_company",
        "industry": "agriculture_agro_export",
        "tradingAddress": { "state": "Lagos", "country": "NG" },
        "countryOfIncorporation": "NG"
    })
}

#[tokio::test]
#[file_serial]
async fn get_application_returns_seeded_draft() {
    let app = TestApp::new().await;
    let (status, body) = app.get("/onboarding/application").await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(body["status"], "draft");
    assert!(body["business"].is_null());
    assert_eq!(body["principals"].as_array().unwrap().len(), 0);
}

#[tokio::test]
#[file_serial]
async fn put_business_persists_and_echoes() {
    let app = TestApp::new().await;
    let (status, body) = app
        .put(
            "/onboarding/application/business",
            json!({ "business": valid_business() }),
        )
        .await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(body["business"]["legalName"], "Adunola Exports Ltd");

    let (_, reread) = app.get("/onboarding/application").await;
    assert_eq!(reread["business"]["cacNumber"], "RC-1234567");
}

#[tokio::test]
#[file_serial]
async fn put_business_rejects_bad_rc() {
    let app = TestApp::new().await;
    let mut biz = valid_business();
    biz["cacNumber"] = json!("not-a-number");
    let (status, body) = app
        .put(
            "/onboarding/application/business",
            json!({ "business": biz }),
        )
        .await;
    assert_eq!(status, StatusCode::BAD_REQUEST);
    assert_eq!(body["code"], "VALIDATION");
    assert_eq!(body["retryable"], false);
}

#[tokio::test]
#[file_serial]
async fn put_principals_replaces_the_list() {
    let app = TestApp::new().await;
    let (status, body) = app
        .put(
            "/onboarding/application/principals",
            json!({ "principals": [{
                "fullName": "Chinonso Okafor",
                "role": "director",
                "dateOfBirth": "1985-04-12",
                "bvn": "12345678901",
                "nin": "10987654321"
            }] }),
        )
        .await;
    assert_eq!(status, StatusCode::OK);
    let principals = body["principals"].as_array().unwrap();
    assert_eq!(principals.len(), 1);
    assert_eq!(principals[0]["fullName"], "Chinonso Okafor");
    assert!(principals[0]["id"].is_string());
}

#[tokio::test]
#[file_serial]
async fn put_principals_rejects_short_bvn() {
    let app = TestApp::new().await;
    let (status, body) = app
        .put(
            "/onboarding/application/principals",
            json!({ "principals": [{ "fullName": "Bad Actor", "role": "director", "bvn": "123" }] }),
        )
        .await;
    assert_eq!(status, StatusCode::BAD_REQUEST);
    assert_eq!(body["code"], "VALIDATION");
}

#[tokio::test]
#[file_serial]
async fn submit_walks_to_approved_with_summary() {
    let app = TestApp::new().await;
    app.put(
        "/onboarding/application/business",
        json!({ "business": valid_business() }),
    )
    .await;
    let (status, body) = app.post("/onboarding/application/submit", json!({})).await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(body["status"], "approved");
    assert!(body["submittedAt"].is_string());
    assert!(body["reviewedAt"].is_string());
    let account_id = body["approvedSummary"]["accountId"].as_str().unwrap();
    assert!(regex_lite_account_id(account_id), "got {account_id}");
    assert_eq!(body["approvedSummary"]["segment"], "Agro Exporter");
    assert_eq!(
        body["approvedSummary"]["monthlyLimit"]["amountMinor"],
        100_000_00
    );
    assert_eq!(body["approvedSummary"]["monthlyLimit"]["currency"], "USD");
}

fn regex_lite_account_id(s: &str) -> bool {
    let mut parts = s.split('-');
    let (a, b) = (parts.next(), parts.next());
    match (a, b, parts.next()) {
        (Some(a), Some(b), None) => {
            (1..=3).contains(&a.len())
                && a.chars().all(|c| c.is_ascii_uppercase())
                && b.len() == 5
                && b.chars().all(|c| c.is_ascii_digit())
        }
        _ => false,
    }
}

#[tokio::test]
#[file_serial]
async fn submit_without_business_fails_validation() {
    let app = TestApp::new().await;
    let (status, body) = app.post("/onboarding/application/submit", json!({})).await;
    assert_eq!(status, StatusCode::BAD_REQUEST);
    assert_eq!(body["code"], "VALIDATION");
}

#[tokio::test]
#[file_serial]
async fn submit_with_wrong_application_id_is_not_found() {
    let app = TestApp::new().await;
    app.put(
        "/onboarding/application/business",
        json!({ "business": valid_business() }),
    )
    .await;
    let (status, body) = app
        .post(
            "/onboarding/application/submit",
            json!({ "applicationId": "nope" }),
        )
        .await;
    assert_eq!(status, StatusCode::NOT_FOUND);
    assert_eq!(body["code"], "NOT_FOUND");
}

#[tokio::test]
#[file_serial]
async fn every_mutation_writes_an_audit_row() {
    let app = TestApp::new().await;
    app.put(
        "/onboarding/application/business",
        json!({ "business": valid_business() }),
    )
    .await;
    app.post("/onboarding/application/submit", json!({})).await;

    let actions: Vec<String> =
        sqlx::query_scalar("select action from audit_log order by occurred_at")
            .fetch_all(&app.pool)
            .await
            .unwrap();
    assert!(actions.contains(&"onboarding.business_saved".to_string()));
    assert_eq!(
        actions
            .iter()
            .filter(|a| *a == "onboarding.state_change")
            .count(),
        3
    );
}

#[tokio::test]
#[file_serial]
async fn document_upload_stores_file_and_rejects_bad_mime() {
    let app = TestApp::new().await;
    let (ok_status, ok_body) = app
        .upload(
            "/onboarding/application/documents",
            &[("type", "cac_certificate")],
            ("file", "cac.pdf", b"%PDF-1.4 test"),
        )
        .await;
    assert_eq!(ok_status, StatusCode::OK);
    assert_eq!(ok_body["type"], "cac_certificate");
    assert_eq!(ok_body["status"], "uploaded");

    let (bad_status, bad_body) = app
        .upload(
            "/onboarding/application/documents",
            &[("type", "memart")],
            ("file", "x.txt", b"hello"),
        )
        .await;
    assert_eq!(bad_status, StatusCode::BAD_REQUEST);
    assert_eq!(bad_body["code"], "VALIDATION");
}
