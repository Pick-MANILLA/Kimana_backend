use axum::http::StatusCode;
use axum::response::{IntoResponse, Response};
use axum::Json;
use serde::Serialize;
use serde_json::json;

/// Mirrors the frontend's `ApiErrorCode` union. See docs/backend-plan.md §02.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "SCREAMING_SNAKE_CASE")]
pub enum ErrorCode {
    Network,
    Timeout,
    Validation,
    NotFound,
    Conflict,
    Unauthorized,
    Forbidden,
    ComplianceHold,
    PartnerFailure,
    RateExpired,
    ServerError,
}

impl ErrorCode {
    fn status(self) -> StatusCode {
        match self {
            ErrorCode::Network => StatusCode::SERVICE_UNAVAILABLE,
            ErrorCode::Timeout => StatusCode::GATEWAY_TIMEOUT,
            ErrorCode::Validation => StatusCode::BAD_REQUEST,
            ErrorCode::NotFound => StatusCode::NOT_FOUND,
            ErrorCode::Conflict => StatusCode::CONFLICT,
            ErrorCode::Unauthorized => StatusCode::UNAUTHORIZED,
            ErrorCode::Forbidden => StatusCode::FORBIDDEN,
            ErrorCode::ComplianceHold => StatusCode::CONFLICT,
            ErrorCode::PartnerFailure => StatusCode::BAD_GATEWAY,
            ErrorCode::RateExpired => StatusCode::CONFLICT,
            ErrorCode::ServerError => StatusCode::INTERNAL_SERVER_ERROR,
        }
    }

    fn default_retryable(self) -> bool {
        matches!(
            self,
            ErrorCode::Network
                | ErrorCode::Timeout
                | ErrorCode::PartnerFailure
                | ErrorCode::RateExpired
                | ErrorCode::ServerError
        )
    }
}

/// Returned (not panicked) by every service and handler on failure. The
/// `IntoResponse` impl renders `{ code, message, retryable }` with the mapped
/// status — the exact shape the frontend's ApiError consumers switch on.
#[derive(Debug, Clone)]
pub struct ApiError {
    pub code: ErrorCode,
    pub message: String,
    pub retryable: bool,
}

impl ApiError {
    pub fn new(code: ErrorCode, message: impl Into<String>) -> Self {
        ApiError {
            code,
            message: message.into(),
            retryable: code.default_retryable(),
        }
    }
    pub fn retryable(mut self, retryable: bool) -> Self {
        self.retryable = retryable;
        self
    }
    pub fn validation(message: impl Into<String>) -> Self {
        ApiError::new(ErrorCode::Validation, message)
    }
    pub fn not_found(message: impl Into<String>) -> Self {
        ApiError::new(ErrorCode::NotFound, message)
    }
    pub fn conflict(message: impl Into<String>) -> Self {
        ApiError::new(ErrorCode::Conflict, message)
    }
    pub fn unauthorized(message: impl Into<String>) -> Self {
        ApiError::new(ErrorCode::Unauthorized, message)
    }
    pub fn forbidden(message: impl Into<String>) -> Self {
        ApiError::new(ErrorCode::Forbidden, message)
    }
    pub fn rate_expired(message: impl Into<String>) -> Self {
        ApiError::new(ErrorCode::RateExpired, message)
    }
    pub fn server_error() -> Self {
        ApiError::new(
            ErrorCode::ServerError,
            "Something went wrong on our end. Try again in a moment.",
        )
    }
}

impl std::fmt::Display for ApiError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{:?}: {}", self.code, self.message)
    }
}
impl std::error::Error for ApiError {}

impl IntoResponse for ApiError {
    fn into_response(self) -> Response {
        (
            self.code.status(),
            Json(json!({
                "code": self.code,
                "message": self.message,
                "retryable": self.retryable,
            })),
        )
            .into_response()
    }
}

impl From<sqlx::Error> for ApiError {
    fn from(err: sqlx::Error) -> Self {
        tracing::error!(error = %err, "database error");
        ApiError::server_error()
    }
}

impl From<serde_json::Error> for ApiError {
    fn from(err: serde_json::Error) -> Self {
        ApiError::validation(format!("Invalid JSON: {err}"))
    }
}

pub type ApiResult<T> = Result<T, ApiError>;
