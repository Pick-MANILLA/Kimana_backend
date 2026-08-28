mod common;

use axum::http::StatusCode;
use common::TestApp;
use serde_json::{json, Value};
use serial_test::file_serial;

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

async fn save_business(app: &TestApp, legal_name: &str) {
    app.put(
        "/onboarding/application/business",
        json!({ "business": business(legal_name) }),
    )
    .await;
}

async fn audit_statuses(app: &TestApp) -> Vec<String> {
    let rows: Vec<Value> = sqlx::query_scalar("select after from audit_log order by occurred_at")
        .fetch_all(&app.pool)
        .await
        .unwrap();
    rows.into_iter()
        .filter_map(|v| v.get("status").and_then(|s| s.as_str()).map(String::from))
        .collect()
}

#[tokio::test]
#[file_serial]
async fn clean_data_is_approved_with_five_passing_checks() {
    let app = TestApp::new().await;
    save_business(&app, "Adunola Exports Ltd").await;
    let (status, body) = app.post("/onboarding/application/submit", json!({})).await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(body["status"], "approved");
    assert!(body["rejectionReasons"].is_null());

    let checks: Vec<(String, bool)> = sqlx::query_as("select check_key, passed from kyb_checks")
        .fetch_all(&app.pool)
        .await
        .unwrap();
    assert_eq!(checks.len(), 5);
    assert!(checks.iter().all(|(_, passed)| *passed));
}

#[tokio::test]
#[file_serial]
async fn submit_passes_through_in_review() {
    let app = TestApp::new().await;
    save_business(&app, "Adunola Exports Ltd").await;
    app.post("/onboarding/application/submit", json!({})).await;
    let statuses = audit_statuses(&app).await;
    assert!(statuses.contains(&"submitted".to_string()));
    assert!(statuses.contains(&"in_review".to_string()));
    assert!(statuses.contains(&"approved".to_string()));
    let in_review = statuses.iter().position(|s| s == "in_review").unwrap();
    let approved = statuses.iter().position(|s| s == "approved").unwrap();
    assert!(in_review < approved);
}

#[tokio::test]
#[file_serial]
async fn cac_flagged_name_is_rejected_with_reasons() {
    let app = TestApp::new().await;
    save_business(&app, "Reject Me Traders Ltd").await;
    let (status, body) = app.post("/onboarding/application/submit", json!({})).await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(body["status"], "rejected");
    assert!(body["approvedSummary"].is_null());
    let fields: Vec<&str> = body["rejectionReasons"]
        .as_array()
        .unwrap()
        .iter()
        .map(|r| r["field"].as_str().unwrap())
        .collect();
    assert!(fields.contains(&"business.cacNumber"));

    let failed: Vec<String> =
        sqlx::query_scalar("select check_key from kyb_checks where passed = false")
            .fetch_all(&app.pool)
            .await
            .unwrap();
    assert!(failed.contains(&"cac_lookup".to_string()));
    assert!(audit_statuses(&app).await.contains(&"rejected".to_string()));
}

#[tokio::test]
#[file_serial]
async fn sanctioned_principal_is_rejected() {
    let app = TestApp::new().await;
    save_business(&app, "Adunola Exports Ltd").await;
    app.put(
        "/onboarding/application/principals",
        json!({ "principals": [{
            "fullName": "Sanctioned Person",
            "role": "director",
            "dateOfBirth": "1980-02-02",
            "bvn": "12345678901",
            "nin": "10987654321"
        }] }),
    )
    .await;
    let (_, body) = app.post("/onboarding/application/submit", json!({})).await;
    assert_eq!(body["status"], "rejected");
    let fields: Vec<&str> = body["rejectionReasons"]
        .as_array()
        .unwrap()
        .iter()
        .map(|r| r["field"].as_str().unwrap())
        .collect();
    assert!(fields.contains(&"principals[].fullName"));
}

#[tokio::test]
#[file_serial]
async fn rejected_then_fixed_then_resubmit_clears_reasons() {
    let app = TestApp::new().await;
    save_business(&app, "Reject Me Traders Ltd").await;
    let (_, r1) = app.post("/onboarding/application/submit", json!({})).await;
    assert_eq!(r1["status"], "rejected");

    save_business(&app, "Clean Traders Ltd").await;
    let (_, r2) = app.post("/onboarding/application/submit", json!({})).await;
    assert_eq!(r2["status"], "approved");
    assert!(r2["rejectionReasons"].is_null());
}

#[tokio::test]
#[file_serial]
async fn resubmit_while_in_review_is_conflict() {
    let app = TestApp::new().await;
    save_business(&app, "Adunola Exports Ltd").await;
    sqlx::query("update onboarding_applications set status = 'in_review'")
        .execute(&app.pool)
        .await
        .unwrap();
    let (status, body) = app.post("/onboarding/application/submit", json!({})).await;
    assert_eq!(status, StatusCode::CONFLICT);
    assert_eq!(body["code"], "CONFLICT");
}
