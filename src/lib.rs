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
use axum::http::{HeaderName, HeaderValue, Method};
use axum::routing::get;
use axum::{Json, Router};
use serde_json::json;
use state::AppState;
use tower_http::cors::CorsLayer;

pub fn build_app(state: AppState) -> Router {
    let cors = CorsLayer::new()
        .allow_origin(
            state
                .config
                .cors_origin
                .parse::<HeaderValue>()
                .expect("CORS_ORIGIN must be a valid header value"),
        )
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
        .route("/health", get(|| async { Json(json!({ "ok": true })) }))
        .merge(routes::session_routes())
        .merge(domain::onboarding::routes())
        .merge(domain::dashboard::routes())
        .merge(domain::fx::routes())
        .merge(domain::recipients::routes())
        .merge(domain::quote::routes())
        .merge(domain::transfers::routes())
        .layer(DefaultBodyLimit::max(12 * 1024 * 1024))
        .layer(cors)
        .with_state(state)
}
