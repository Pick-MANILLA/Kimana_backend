mod common;

use axum::http::StatusCode;
use common::TestApp;
use serde_json::json;
use serial_test::file_serial;

fn register_body(email: &str) -> serde_json::Value {
    json!({
        "email": email,
        "password": "correcthorsebattery",
        "displayName": "Ada Lovelace",
        "legalName": "Analytical Engines Ltd",
    })
}

#[tokio::test]
#[file_serial]
async fn register_creates_user_and_session() {
    let app = TestApp::new().await;
    app.clear_cookie();

    let (status, body) = app
        .post("/register", register_body("ada@example.com"))
        .await;
    assert_eq!(status, StatusCode::CREATED);
    assert_eq!(body["role"], "customer");
    assert_eq!(body["displayName"], "Ada Lovelace");

    let (status, session) = app.get("/session").await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(session["userId"], body["userId"]);
}

#[tokio::test]
#[file_serial]
async fn register_creates_draft_onboarding_application() {
    let app = TestApp::new().await;
    app.clear_cookie();

    let (status, _) = app
        .post("/register", register_body("bola@example.com"))
        .await;
    assert_eq!(status, StatusCode::CREATED);

    let (status, application) = app.get("/onboarding/application").await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(application["status"], "draft");
}

#[tokio::test]
#[file_serial]
async fn register_rejects_duplicate_email() {
    let app = TestApp::new().await;
    app.clear_cookie();

    let (status, _) = app
        .post("/register", register_body("chidi@example.com"))
        .await;
    assert_eq!(status, StatusCode::CREATED);

    app.clear_cookie();
    let (status, body) = app
        .post("/register", register_body("CHIDI@example.com"))
        .await;
    assert_eq!(status, StatusCode::CONFLICT);
    assert_eq!(body["code"], "CONFLICT");
}

#[tokio::test]
#[file_serial]
async fn login_succeeds_and_matches_registered_identity() {
    let app = TestApp::new().await;
    app.clear_cookie();

    let (_, registered) = app
        .post("/register", register_body("efe@example.com"))
        .await;
    app.clear_cookie();

    let (status, logged_in) = app.login("efe@example.com", "correcthorsebattery").await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(logged_in["userId"], registered["userId"]);
}

#[tokio::test]
#[file_serial]
async fn login_rejects_wrong_password() {
    let app = TestApp::new().await;
    app.clear_cookie();

    let (status, body) = app.login(common_test_email(), "not-the-password").await;
    assert_eq!(status, StatusCode::UNAUTHORIZED);
    assert_eq!(body["code"], "UNAUTHORIZED");
}

#[tokio::test]
#[file_serial]
async fn logout_invalidates_session() {
    let app = TestApp::new().await;
    let (status, _) = app.get("/session").await;
    assert_eq!(status, StatusCode::OK);

    let (status, _) = app.logout().await;
    assert_eq!(status, StatusCode::NO_CONTENT);

    let (status, _) = app.get("/session").await;
    assert_eq!(status, StatusCode::UNAUTHORIZED);
}

#[tokio::test]
#[file_serial]
async fn session_requires_authentication() {
    let app = TestApp::new().await;
    app.clear_cookie();

    let (status, _) = app.get("/session").await;
    assert_eq!(status, StatusCode::UNAUTHORIZED);
}

fn common_test_email() -> &'static str {
    kimana_backend::seed::DEMO_EMAIL
}
