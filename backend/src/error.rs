//! Custom error types for the Crucible backend.
//!
//! Provides a unified [`AppError`] type that maps internal errors into
//! appropriate HTTP status codes and JSON error responses.

use axum::{
    http::StatusCode,
    response::{IntoResponse, Response},
    Json,
};
use serde::Serialize;
use thiserror::Error;
use tracing::error;

/// Structured error response returned to API clients.
#[derive(Debug, Serialize)]
pub struct ErrorResponse {
    /// Machine-readable error code (e.g., `"database_error"`, `"not_found"`).
    pub code: String,
    /// Human-readable error message.
    pub message: String,
    /// Timestamp of when the error occurred.
    pub timestamp: String,
}

/// Application-level error type that unifies all possible error sources.
///
/// Each variant maps to an HTTP status code and produces a consistent
/// JSON error response via the [`IntoResponse`] implementation.
///
/// # Examples
///
/// ```rust,no_run
/// use crucible_backend::error::AppError;
///
/// async fn handler() -> Result<String, AppError> {
///     Err(AppError::NotFound("Contract not found".into()))
/// }
/// ```
#[derive(Debug, Error)]
pub enum AppError {
    /// 404 — The requested resource was not found.
    #[error("Not found: {0}")]
    NotFound(String),

    /// 400 — The request was malformed or contained invalid data.
    #[error("Bad request: {0}")]
    BadRequest(String),

    /// 401 — Authentication is required or failed.
    #[error("Unauthorized: {0}")]
    Unauthorized(String),

    /// 403 — The authenticated user lacks permission.
    #[error("Forbidden: {0}")]
    Forbidden(String),

    /// 411 — The request must contain a Content-Length header.
    #[error("Length required: {0}")]
    LengthRequired(String),

    /// 409 — The request conflicts with the current state.
    #[error("Conflict: {0}")]
    Conflict(String),

    /// 422 — The request body was well-formed but semantically invalid.
    #[error("Validation error: {0}")]
    ValidationError(String),

    /// 415 — The request Content-Type is not supported.
    #[error("Unsupported media type: {0}")]
    UnsupportedMediaType(String),

    /// 503 — Service temporarily unavailable (e.g. database pool exhaustion).
    #[error("Service unavailable")]
    ServiceUnavailable { retry_after: u64 },

    /// 500 — An internal database error occurred.
    #[error("Database error: {0}")]
    Database(#[from] sqlx::Error),

    /// 500 — A database migration failed.
    #[error("Migration error: {0}")]
    Migration(#[from] sqlx::migrate::MigrateError),

    /// 500 — An internal Redis error occurred.
    #[error("Redis error: {0}")]
    Redis(#[from] redis::RedisError),

    /// 500 — A serialization error occurred.
    #[error("Serialization error: {0}")]
    Serialization(#[from] serde_json::Error),

    /// 500 — A catch-all for unexpected internal errors.
    #[error("Internal error: {0}")]
    InternalError(String),

    /// 500 — Internal server error (no message).
    #[error("Internal server error")]
    Internal,

    /// 502 — Stellar network communication failure.
    #[error("URL parse error: {0}")]
    UrlParse(#[from] url::ParseError),

    /// 502 — Stellar network communication failure.
    #[error("Stellar operation failed: {0}")]
    StellarError(String),
}

// Convenience constructors used by services.
impl AppError {
    /// Wrap a database error.
    pub fn db(e: sqlx::Error) -> Self {
        match e {
            sqlx::Error::PoolTimedOut => AppError::ServiceUnavailable { retry_after: 5 },
            other => AppError::Database(other),
        }
    }

    /// Wrap a Redis error.
    pub fn redis(e: redis::RedisError) -> Self {
        AppError::Redis(e)
    }

    /// Wrap a serialization error.
    pub fn serialization(e: serde_json::Error) -> Self {
        AppError::Serialization(e)
    }
}

impl IntoResponse for AppError {
    fn into_response(self) -> Response {
        match self {
            AppError::ServiceUnavailable { retry_after } => {
                crate::services::metrics::inc_pool_exhaustion_metric();
                let mut resp = (
                    StatusCode::SERVICE_UNAVAILABLE,
                    Json(serde_json::json!({
                        "error": "service temporarily unavailable",
                        "retry_after_seconds": retry_after
                    })),
                )
                    .into_response();
                resp.headers_mut().insert(
                    axum::http::header::RETRY_AFTER,
                    axum::http::HeaderValue::from(retry_after),
                );
                return resp;
            }
            AppError::Database(sqlx::Error::PoolTimedOut) => {
                crate::services::metrics::inc_pool_exhaustion_metric();
                let retry_after = 5;
                let mut resp = (
                    StatusCode::SERVICE_UNAVAILABLE,
                    Json(serde_json::json!({
                        "error": "service temporarily unavailable",
                        "retry_after_seconds": retry_after
                    })),
                )
                    .into_response();
                resp.headers_mut().insert(
                    axum::http::header::RETRY_AFTER,
                    axum::http::HeaderValue::from(retry_after),
                );
                return resp;
            }
            _ => {}
        }

        let (status, code, message) = match &self {
            AppError::NotFound(msg) => (StatusCode::NOT_FOUND, "not_found", msg.clone()),
            AppError::BadRequest(msg) => (StatusCode::BAD_REQUEST, "bad_request", msg.clone()),
            AppError::Unauthorized(msg) => (StatusCode::UNAUTHORIZED, "unauthorized", msg.clone()),
            AppError::Forbidden(msg) => (StatusCode::FORBIDDEN, "forbidden", msg.clone()),
            AppError::Conflict(msg) => (StatusCode::CONFLICT, "conflict", msg.clone()),
            AppError::ValidationError(msg) => (
                StatusCode::UNPROCESSABLE_ENTITY,
                "validation_error",
                msg.clone(),
            ),
            AppError::UnsupportedMediaType(msg) => (
                StatusCode::UNSUPPORTED_MEDIA_TYPE,
                "unsupported_media_type",
                msg.clone(),
            ),
            AppError::Database(e) => {
                error!("Database error: {e:?}");
                (
                    StatusCode::INTERNAL_SERVER_ERROR,
                    "database_error",
                    "An internal database error occurred".to_string(),
                )
            }
            AppError::Migration(e) => {
                error!("Migration error: {e:?}");
                (
                    StatusCode::INTERNAL_SERVER_ERROR,
                    "migration_error",
                    "A database migration failed".to_string(),
                )
            }
            AppError::Redis(e) => {
                error!("Redis error: {e:?}");
                (
                    StatusCode::INTERNAL_SERVER_ERROR,
                    "redis_error",
                    "An internal cache error occurred".to_string(),
                )
            }
            AppError::Serialization(e) => {
                error!("Serialization error: {e:?}");
                (
                    StatusCode::INTERNAL_SERVER_ERROR,
                    "serialization_error",
                    "A serialization error occurred".to_string(),
                )
            }
            AppError::InternalError(msg) => {
                error!("Internal error: {msg}");
                (
                    StatusCode::INTERNAL_SERVER_ERROR,
                    "internal_error",
                    "An internal error occurred".to_string(),
                )
            }
            AppError::Internal => {
                error!("Internal server error");
                (
                    StatusCode::INTERNAL_SERVER_ERROR,
                    "internal_error",
                    "An internal server error occurred".to_string(),
                )
            }
            AppError::UrlParse(e) => {
                error!("URL parse error: {e}");
                (
                    StatusCode::BAD_REQUEST,
                    "invalid_url",
                    "The provided URL was invalid".to_string(),
                )
            }
            AppError::StellarError(msg) => {
                error!("Stellar error: {msg}");
                (
                    StatusCode::BAD_GATEWAY,
                    "stellar_error",
                    "Failed to communicate with Stellar network".to_string(),
                )
            }
            AppError::LengthRequired(msg) => (StatusCode::LENGTH_REQUIRED, "length_required", msg.clone()),
        };

        (
            status,
            Json(ErrorResponse {
                code: code.to_string(),
                message,
                timestamp: chrono::Utc::now().to_rfc3339(),
            }),
        )
            .into_response()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_not_found_error_display() {
        let err = AppError::NotFound("Contract not found".into());
        assert_eq!(err.to_string(), "Not found: Contract not found");
    }

    #[test]
    fn test_bad_request_error_display() {
        let err = AppError::BadRequest("Invalid address format".into());
        assert_eq!(err.to_string(), "Bad request: Invalid address format");
    }

    #[test]
    fn test_validation_error_display() {
        let err = AppError::ValidationError("name is required".into());
        assert_eq!(err.to_string(), "Validation error: name is required");
    }

    #[test]
    fn test_internal_error_display() {
        let err = AppError::InternalError("unexpected state".into());
        assert_eq!(err.to_string(), "Internal error: unexpected state");
    }

    #[test]
    fn test_length_required_error_display() {
        let err = AppError::LengthRequired("Content-Length header required".into());
        assert_eq!(err.to_string(), "Length required: Content-Length header required");
    }

    #[test]
    fn test_error_response_serialization() {
        let resp = ErrorResponse {
            code: "not_found".into(),
            message: "Resource not found".into(),
            timestamp: "2026-07-29T16:00:00Z".into(),
        };
        let json = serde_json::to_string(&resp).unwrap();
        assert!(json.contains("\"code\":\"not_found\""));
        assert!(json.contains("\"message\":\"Resource not found\""));
        assert!(json.contains("\"timestamp\":\"2026-07-29T16:00:00Z\""));
    }

    #[tokio::test]
    async fn test_pool_timed_out_response() {
        let err = AppError::Database(sqlx::Error::PoolTimedOut);
        let resp = err.into_response();
        assert_eq!(resp.status(), StatusCode::SERVICE_UNAVAILABLE);
        assert_eq!(resp.headers().get("retry-after").unwrap(), "5");

        let body = axum::body::to_bytes(resp.into_body(), usize::MAX).await.unwrap();
        let json: serde_json::Value = serde_json::from_slice(&body).unwrap();
        assert_eq!(json["error"], "service temporarily unavailable");
        assert_eq!(json["retry_after_seconds"], 5);
    }

    #[tokio::test]
    async fn test_service_unavailable_response() {
        let err = AppError::ServiceUnavailable { retry_after: 10 };
        let resp = err.into_response();
        assert_eq!(resp.status(), StatusCode::SERVICE_UNAVAILABLE);
        assert_eq!(resp.headers().get("retry-after").unwrap(), "10");

        let body = axum::body::to_bytes(resp.into_body(), usize::MAX).await.unwrap();
        let json: serde_json::Value = serde_json::from_slice(&body).unwrap();
        assert_eq!(json["error"], "service temporarily unavailable");
        assert_eq!(json["retry_after_seconds"], 10);
    }
}
