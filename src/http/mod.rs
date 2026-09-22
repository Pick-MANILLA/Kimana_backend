pub mod auth;

pub use auth::{Session, SESSION_COOKIE_NAME};

use crate::error::ApiError;
use axum::extract::rejection::JsonRejection;
use axum::extract::{FromRequest, Request};
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
        other => ApiError::validation(other.body_text()),
    }
}
