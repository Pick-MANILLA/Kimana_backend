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
pub mod settlement;
pub mod state;
pub mod storage;
pub mod util;

use axum::body::{to_bytes, Body as AxumBody};
use axum::extract::{DefaultBodyLimit, Request};
use axum::http::{header, HeaderName, HeaderValue, Method, StatusCode};
use axum::middleware::{self, Next};
use axum::response::{IntoResponse, Response};
use axum::routing::get;
use axum::{Json, Router};
use error::{ApiError, ErrorCode};
use serde_json::json;
use state::AppState;
use std::any::Any;
use std::time::Duration;
use tower_http::catch_panic::CatchPanicLayer;
use tower_http::cors::{AllowOrigin, CorsLayer};
use tower_http::request_id::{
    MakeRequestUuid, PropagateRequestIdLayer, RequestId, SetRequestIdLayer,
};
use tracing::Instrument;
use utoipa::openapi::security::{ApiKey, ApiKeyValue, SecurityScheme};
use utoipa::{Modify, OpenApi};
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

/// Aborts a request that runs past `REQUEST_TIMEOUT`. Hand-rolled instead of
/// tower-http's `TimeoutLayer` because that one answers with an empty body,
/// which the frontend's `ApiError` handling can't read.
async fn enforce_timeout(req: Request, next: Next) -> Response {
    match tokio::time::timeout(REQUEST_TIMEOUT, next.run(req)).await {
        Ok(response) => response,
        Err(_) => {
            tracing::warn!("request timed out");
            ApiError::new(
                ErrorCode::Timeout,
                "The request took too long. Try again in a moment.",
            )
            .with_status(StatusCode::REQUEST_TIMEOUT)
            .into_response()
        }
    }
}

/// Turns a handler panic into our 500 JSON body instead of a dropped
/// connection. The payload is logged, never returned (CWE-209).
fn handle_panic(payload: Box<dyn Any + Send + 'static>) -> Response {
    let detail = payload
        .downcast_ref::<String>()
        .map(String::as_str)
        .or_else(|| payload.downcast_ref::<&str>().copied())
        .unwrap_or("non-string panic payload");
    tracing::error!(panic = detail, "handler panicked");
    ApiError::server_error().into_response()
}

async fn route_not_found() -> ApiError {
    ApiError::not_found("That endpoint doesn't exist.")
}

async fn method_not_allowed() -> ApiError {
    ApiError::validation("That HTTP method isn't supported on this endpoint.")
        .with_status(StatusCode::METHOD_NOT_ALLOWED)
}

/// Registers the `kimana_session` cookie as the `cookieAuth` scheme that
/// protected paths reference via `security(("cookieAuth" = []))`.
struct CookieAuth;

impl Modify for CookieAuth {
    fn modify(&self, openapi: &mut utoipa::openapi::OpenApi) {
        let components = openapi.components.get_or_insert_with(Default::default);
        components.add_security_scheme(
            "cookieAuth",
            SecurityScheme::ApiKey(ApiKey::Cookie(ApiKeyValue::new(http::SESSION_COOKIE_NAME))),
        );
    }
}

#[derive(OpenApi)]
#[openapi(
    paths(
        routes::get_session,
        domain::auth::routes::register,
        domain::auth::routes::login,
        domain::auth::routes::logout,
        domain::onboarding::routes::get_application,
        domain::onboarding::routes::save_business,
        domain::onboarding::routes::save_principals,
        domain::onboarding::routes::submit,
        domain::onboarding::routes::upload_document,
        domain::onboarding::routes::retry_document,
        domain::onboarding::routes::remove_document,
        domain::dashboard::overview,
        domain::fx::indicative,
        domain::recipients::list,
        domain::recipients::save,
        domain::recipients::validate,
        domain::quote::create,
        domain::transfers::routes::create,
        domain::transfers::routes::list,
        domain::transfers::routes::get_one,
        domain::transfers::routes::timeline,
        domain::transfers::routes::screening_decision,
    ),
    components(schemas(
        error::ErrorResponse,
        contract::auth::SessionResponse,
        domain::auth::routes::RegisterBody,
        domain::auth::routes::LoginBody,
    )),
    modifiers(&CookieAuth),
    tags(
        (name = "auth", description = "Registration, login, session and logout"),
        (name = "onboarding", description = "KYB onboarding wizard: business, principals, documents, submit"),
        (name = "dashboard", description = "Server-composed dashboard aggregate"),
        (name = "fx", description = "Indicative FX rates"),
        (name = "recipients", description = "Payout recipients"),
        (name = "quotes", description = "Firm quotes"),
        (name = "transfers", description = "Transfer lifecycle"),
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
        .fallback(route_not_found)
        .method_not_allowed_fallback(method_not_allowed)
        .layer(DefaultBodyLimit::max(DEFAULT_BODY_LIMIT))
        .layer(CatchPanicLayer::custom(handle_panic))
        .layer(middleware::from_fn(enforce_timeout))
        .layer(middleware::from_fn(attach_request_id))
        .layer(PropagateRequestIdLayer::new(REQUEST_ID_HEADER))
        .layer(SetRequestIdLayer::new(REQUEST_ID_HEADER, MakeRequestUuid))
        .layer(cors)
        .with_state(state)
}

#[cfg(test)]
mod openapi_tests {
    use super::ApiDoc;
    use std::collections::HashMap;
    use utoipa::OpenApi;

    /// Swagger UI routes "Try it out" by operationId, so a duplicate makes
    /// one endpoint silently execute another. utoipa defaults the id to the
    /// handler's fn name, which collides easily (`create`, `list`).
    #[test]
    fn operation_ids_are_unique() {
        let mut seen: HashMap<String, String> = HashMap::new();
        for (path, item) in ApiDoc::openapi().paths.paths {
            let ops = [
                ("GET", item.get),
                ("PUT", item.put),
                ("POST", item.post),
                ("DELETE", item.delete),
                ("PATCH", item.patch),
            ];
            for (method, op) in ops {
                let Some(id) = op.and_then(|op| op.operation_id) else {
                    continue;
                };
                let here = format!("{method} {path}");
                if let Some(prev) = seen.insert(id.clone(), here.clone()) {
                    panic!("operationId `{id}` is used by both {prev} and {here}");
                }
            }
        }
    }
}
