pub mod auth;

pub use auth::{Session, SESSION_COOKIE_NAME};

use crate::error::ApiError;
use axum::extract::multipart::MultipartRejection;
use axum::extract::rejection::{JsonRejection, PathRejection, QueryRejection};
use axum::extract::{FromRequest, FromRequestParts, Request};
use axum::http::request::Parts;
use axum::http::StatusCode;
use serde::de::DeserializeOwned;

/// JSON body extractor that renders deserialization failures as our
/// `{ code: "VALIDATION", ... }` shape instead of axum's default plain-text 422.
pub struct Body<T>(pub T);

impl<T, S> FromRequest<S> for Body<T>
where
    T: DeserializeOwned,
    S: Send + Sync,
{
    type Rejection = ApiError;

    async fn from_request(req: Request, state: &S) -> Result<Self, Self::Rejection> {
        match axum::Json::<T>::from_request(req, state).await {
            Ok(axum::Json(value)) => Ok(Body(value)),
            Err(rej) => Err(sanitize_json_rejection(rej)),
        }
    }
}

/// `JsonRejection`'s data/syntax variants embed serde_json's error text
/// verbatim — including our internal struct field names and enum variants
/// (CWE-209, ISSUE-BE-08). Those two get a generic message and a server-side
/// log line instead; the other variants (missing content-type, body-too-large)
/// describe the HTTP request shape, not our Rust types, so they're safe as-is.
fn sanitize_json_rejection(rej: JsonRejection) -> ApiError {
    match rej {
        JsonRejection::JsonDataError(_) | JsonRejection::JsonSyntaxError(_) => {
            tracing::warn!(error = %rej, "malformed JSON body");
            ApiError::validation("Malformed JSON payload or invalid field format.")
        }
        other => request_shape_error(other.status(), other.body_text()),
    }
}

/// For rejections that describe the HTTP request itself (content type,
/// boundary, size) rather than our Rust types, so their text is safe to
/// return. Keeps 413 so the client can tell "too big" apart from "malformed".
pub(crate) fn request_shape_error(status: StatusCode, message: String) -> ApiError {
    let api_error = ApiError::validation(message);
    if status == StatusCode::PAYLOAD_TOO_LARGE {
        return api_error.with_status(status);
    }
    api_error
}

/// Multipart extractor that renders a missing or invalid multipart
/// content type as our `VALIDATION` shape instead of axum's plain-text 400.
pub struct Multipart(pub axum::extract::Multipart);

impl<S> FromRequest<S> for Multipart
where
    S: Send + Sync,
{
    type Rejection = ApiError;

    async fn from_request(req: Request, state: &S) -> Result<Self, Self::Rejection> {
        axum::extract::Multipart::from_request(req, state)
            .await
            .map(Multipart)
            .map_err(|rej: MultipartRejection| request_shape_error(rej.status(), rej.body_text()))
    }
}

/// Query-string extractor with the same `VALIDATION` shape as `Body`.
pub struct Query<T>(pub T);

impl<T, S> FromRequestParts<S> for Query<T>
where
    T: DeserializeOwned,
    S: Send + Sync,
{
    type Rejection = ApiError;

    async fn from_request_parts(parts: &mut Parts, state: &S) -> Result<Self, Self::Rejection> {
        match axum::extract::Query::<T>::from_request_parts(parts, state).await {
            Ok(axum::extract::Query(value)) => Ok(Query(value)),
            Err(rej) => Err(sanitize_query_rejection(rej)),
        }
    }
}

/// Path-parameter extractor with the same `VALIDATION` shape as `Body`.
pub struct Path<T>(pub T);

impl<T, S> FromRequestParts<S> for Path<T>
where
    T: DeserializeOwned + Send,
    S: Send + Sync,
{
    type Rejection = ApiError;

    async fn from_request_parts(parts: &mut Parts, state: &S) -> Result<Self, Self::Rejection> {
        match axum::extract::Path::<T>::from_request_parts(parts, state).await {
            Ok(axum::extract::Path(value)) => Ok(Path(value)),
            Err(rej) => Err(sanitize_path_rejection(rej)),
        }
    }
}

/// Same CWE-209 concern as `sanitize_json_rejection`: serde's message names
/// our query struct's fields and enum variants.
fn sanitize_query_rejection(rej: QueryRejection) -> ApiError {
    tracing::warn!(error = %rej, "malformed query string");
    ApiError::validation("Invalid query parameters.")
}

/// `MissingPathParams` means a route/extractor mismatch on our side, not a
/// bad request, so it's a server error rather than a validation one.
fn sanitize_path_rejection(rej: PathRejection) -> ApiError {
    match rej {
        PathRejection::FailedToDeserializePathParams(_) => {
            tracing::warn!(error = %rej, "malformed path parameters");
            ApiError::validation("Invalid path parameters.")
        }
        other => {
            tracing::error!(error = %other, "path extraction failed");
            ApiError::server_error()
        }
    }
}
