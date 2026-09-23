mod common;

use axum::http::StatusCode;
use common::TestApp;
use serde_json::{json, Value};
use serial_test::file_serial;

fn assert_error_shape(body: &Value, code: &str) {
    assert_eq!(body["code"], code, "unexpected body: {body}");
    assert!(body["message"].is_string(), "missing message: {body}");
    assert!(body["retryable"].is_boolean(), "missing retryable: {body}");
    assert!(body["requestId"].is_string(), "missing requestId: {body}");
}

#[tokio::test]
#[file_serial]
async fn unknown_route_returns_json_not_found() {
    let app = TestApp::new().await;
    let (status, body) = app.get("/does-not-exist").await;
    assert_eq!(status, StatusCode::NOT_FOUND);
    assert_error_shape(&body, "NOT_FOUND");
}

#[tokio::test]
#[file_serial]
async fn wrong_method_returns_json_method_not_allowed() {
    let app = TestApp::new().await;
    let (status, body) = app.delete("/session").await;
    assert_eq!(status, StatusCode::METHOD_NOT_ALLOWED);
    assert_error_shape(&body, "VALIDATION");
}

#[tokio::test]
#[file_serial]
async fn malformed_query_returns_sanitized_validation_error() {
    let app = TestApp::new().await;
    let (status, body) = app.get("/rates/indicative?send=USD").await;
    assert_eq!(status, StatusCode::BAD_REQUEST);
    assert_error_shape(&body, "VALIDATION");
    let message = body["message"].as_str().unwrap();
    assert!(
        !message.contains("receive"),
        "query rejection leaked a field name: {message}"
    );
}

#[tokio::test]
#[file_serial]
async fn oversized_upload_returns_json_payload_too_large() {
    let app = TestApp::new().await;
    let file = vec![b'a'; 13 * 1024 * 1024];
    let (status, body) = app
        .upload(
            "/onboarding/application/documents",
            &[("type", "cac_certificate")],
            ("file", "cac.pdf", &file),
        )
        .await;
    assert_eq!(status, StatusCode::PAYLOAD_TOO_LARGE);
    assert_error_shape(&body, "VALIDATION");
}

#[tokio::test]
#[file_serial]
async fn upload_without_multipart_content_type_returns_json_validation_error() {
    let app = TestApp::new().await;
    let (status, body) = app
        .post(
            "/onboarding/application/documents",
            json!({ "type": "cac_certificate" }),
        )
        .await;
    assert_eq!(status, StatusCode::BAD_REQUEST);
    assert_error_shape(&body, "VALIDATION");
}

#[tokio::test]
#[file_serial]
async fn oversized_json_body_returns_json_payload_too_large() {
    let app = TestApp::new().await;
    let padding = "a".repeat(200 * 1024);
    let (status, body) = app
        .post(
            "/login",
            json!({ "email": "ada@example.com", "password": padding }),
        )
        .await;
    assert_eq!(status, StatusCode::PAYLOAD_TOO_LARGE);
    assert_error_shape(&body, "VALIDATION");
}
