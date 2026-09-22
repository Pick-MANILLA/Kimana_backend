#![allow(clippy::inconsistent_digit_grouping)]
pub mod audit;
pub mod config;
pub mod contract;
pub mod db;
pub mod domain;
pub mod error;
pub mod http;
pub mod ids;
pub mod routes;
pub mod seed;
pub mod state;
pub mod storage;
pub mod util;

use axum::extract::DefaultBodyLimit;
use axum::http::{HeaderName, HeaderValue, Method, StatusCode};
use axum::routing::get;
use axum::{Json, Router};
use serde_json::json;
use state::AppState;
use std::time::Duration;
use tower_http::cors::{AllowOrigin, CorsLayer};
use tower_http::timeout::TimeoutLayer;
use utoipa::OpenApi;
use utoipa_swagger_ui::SwaggerUi;

/// Default cap for standard JSON request bodies (see ISSUE-BE-05). The
/// document upload route needs more room and sets its own, larger limit —
/// see `DOCUMENT_UPLOAD_BODY_LIMIT` in `domain::onboarding::routes`.
const DEFAULT_BODY_LIMIT: usize = 128 * 1024;

/// How long a request may run before the server aborts it with `408 Request
/// Timeout`, to stop slow clients from tying up a Tokio worker/DB connection
/// indefinitely (ISSUE-BE-05).
const REQUEST_TIMEOUT: Duration = Duration::from_secs(15);

#[derive(OpenApi)]
#[openapi(
    paths(
        routes::get_session,
        domain::auth::routes::register,
        domain::auth::routes::login,
        domain::auth::routes::logout,
    ),
    components(schemas(
        contract::auth::SessionResponse,
        domain::auth::routes::RegisterBody,
        domain::auth::routes::LoginBody,
    )),
    tags(
        (name = "auth", description = "Registration, login, session and logout")
    )
)]
struct ApiDoc;

pub fn build_app(state: AppState) -> Router {
    let origins: Vec<HeaderValue> = state
        .config
        .cors_origins
        .iter()
        .map(|origin| {
            origin
                .parse::<HeaderValue>()
                .expect("CORS_ORIGIN entries must be valid header values")
        })
        .collect();

    let cors = CorsLayer::new()
        .allow_origin(AllowOrigin::list(origins))
        .allow_methods([
            Method::GET,
            Method::POST,
            Method::PUT,
            Method::DELETE,
            Method::OPTIONS,
        ])
        .allow_headers([
            HeaderName::from_static("content-type"),
            HeaderName::from_static("idempotency-key"),
        ])
        .allow_credentials(true);

    Router::new()
        .merge(SwaggerUi::new("/swagger-ui").url("/api-docs/openapi.json", ApiDoc::openapi()))
        .route("/health", get(|| async { Json(json!({ "ok": true })) }))
        .merge(routes::session_routes())
        .merge(domain::auth::routes())
        .merge(domain::onboarding::routes())
        .merge(domain::dashboard::routes())
        .merge(domain::fx::routes())
        .merge(domain::recipients::routes())
        .merge(domain::quote::routes())
        .merge(domain::transfers::routes())
        .layer(DefaultBodyLimit::max(DEFAULT_BODY_LIMIT))
        .layer(TimeoutLayer::with_status_code(
            StatusCode::REQUEST_TIMEOUT,
            REQUEST_TIMEOUT,
        ))
        .layer(cors)
        .with_state(state)
}
