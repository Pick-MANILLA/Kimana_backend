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

use axum::body::{to_bytes, Body as AxumBody};
use axum::extract::{DefaultBodyLimit, Request};
use axum::http::{header, HeaderName, HeaderValue, Method, StatusCode};
use axum::middleware::{self, Next};
use axum::response::Response;
use axum::routing::get;
use axum::{Json, Router};
use serde_json::json;
use state::AppState;
use std::time::Duration;
use tower_http::cors::{AllowOrigin, CorsLayer};
use tower_http::request_id::{MakeRequestUuid, PropagateRequestIdLayer, RequestId, SetRequestIdLayer};
use tower_http::timeout::TimeoutLayer;
use tracing::Instrument;
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

const REQUEST_ID_HEADER: HeaderName = HeaderName::from_static("x-request-id");

/// Our own JSON error bodies are always tiny; this is just a sanity ceiling
/// on the buffering `attach_request_id` does to merge `requestId` in.
const MAX_ERROR_BODY_BYTES: usize = 64 * 1024;

/// Runs the request inside a tracing span carrying the request id set by
/// `SetRequestIdLayer`, and merges `requestId` into JSON error bodies —
/// `ApiError::into_response` has no access to the request, so it can't add
/// this itself. The span means any `tracing::error!` call anywhere in the
/// request's call chain (e.g. the `sqlx::Error` log in `error.rs`) is
/// automatically correlated in the logs without changing that call site.
async fn attach_request_id(req: Request, next: Next) -> Response {
    let request_id = req
        .extensions()
        .get::<RequestId>()
        .and_then(|id| id.header_value().to_str().ok())
        .map(str::to_string)
        .unwrap_or_else(|| "unknown".to_string());

    let span = tracing::info_span!("request", request_id = %request_id);
    let response = next.run(req).instrument(span).await;

    if !response.status().is_client_error() && !response.status().is_server_error() {
        return response;
    }
    let is_json = response
        .headers()
        .get(header::CONTENT_TYPE)
        .and_then(|v| v.to_str().ok())
        .is_some_and(|ct| ct.starts_with("application/json"));
    if !is_json {
        return response;
    }

    let (mut parts, body) = response.into_parts();
    let bytes = match to_bytes(body, MAX_ERROR_BODY_BYTES).await {
        Ok(bytes) => bytes,
        Err(_) => {
            parts.headers.remove(header::CONTENT_LENGTH);
            return Response::from_parts(parts, AxumBody::empty());
        }
    };
    let Ok(mut value) = serde_json::from_slice::<serde_json::Value>(&bytes) else {
        return Response::from_parts(parts, AxumBody::from(bytes));
    };
    if let Some(obj) = value.as_object_mut() {
        obj.insert("requestId".into(), json!(request_id));
    }
    let Ok(new_bytes) = serde_json::to_vec(&value) else {
        return Response::from_parts(parts, AxumBody::from(bytes));
    };
    parts.headers.insert(
        header::CONTENT_LENGTH,
        HeaderValue::from_str(&new_bytes.len().to_string())
            .expect("a decimal length is a valid header value"),
    );
    Response::from_parts(parts, AxumBody::from(new_bytes))
}

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
        .expose_headers([REQUEST_ID_HEADER])
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
        .layer(middleware::from_fn(attach_request_id))
        .layer(TimeoutLayer::with_status_code(
            StatusCode::REQUEST_TIMEOUT,
            REQUEST_TIMEOUT,
        ))
        .layer(PropagateRequestIdLayer::new(REQUEST_ID_HEADER))
        .layer(SetRequestIdLayer::new(REQUEST_ID_HEADER, MakeRequestUuid))
        .layer(cors)
        .with_state(state)
}
