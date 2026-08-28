pub mod auth;

pub use auth::Session;

use crate::error::ApiError;
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
            Err(rej) => Err(ApiError::validation(rej.body_text())),
        }
    }
}
