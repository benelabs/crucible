//! Custom error type hierarchy for the Crucible backend.
//!
//! Provides domain-specific error types that compose into [`AppError`] for
//! HTTP responses, while preserving rich context for logging and tracing.

use axum::{http::StatusCode, response::{IntoResponse, Response}, Json};
use serde::Serialize;
use serde_json::json;
use thiserror::Error;
use tracing::error;

/// Result type alias for backend services and handlers.
pub type Result<T> = std::result::Result<T, AppError>;

// ---------------------------------------------------------------------------
// Domain error types
// ---------------------------------------------------------------------------

/// Errors arising from file upload and validation operations.
#[derive(Debug, Error)]
pub enum FileError {
    #[error("File too large: {size} bytes exceeds limit of {limit} bytes")]
    TooLarge { size: u64, limit: u64 },

    #[error("Unsupported MIME type: {0}")]
    UnsupportedMimeType(String),

    #[error("Invalid file name: {0}")]
    InvalidFileName(String),

    #[error("Malformed file content: {0}")]
    MalformedContent(String),
}

/// Errors arising from database operations.
#[derive(Debug, Error)]
pub enum DatabaseError {
    #[error("Record not found: {0}")]
    NotFound(String),

    #[error("Unique constraint violation: {0}")]
    UniqueViolation(String),

    #[error("Foreign key violation: {0}")]
    ForeignKeyViolation(String),

    #[error("Connection error: {0}")]
    Connection(String),

    #[error("Query error: {0}")]
    Query(#[from] sqlx::Error),
}

/// Errors arising from cache / Redis operations.
#[derive(Debug, Error)]
pub enum CacheError {
    #[error("Cache miss for key: {0}")]
    Miss(String),

    #[error("Serialization error: {0}")]
    Serialization(String),

    #[error("Redis error: {0}")]
    Redis(#[from] redis::RedisError),
}

/// Errors arising from authentication and authorisation.
#[derive(Debug, Error)]
pub enum AuthError {
    #[error("Missing credentials")]
    MissingCredentials,

    #[error("Invalid token")]
    InvalidToken,

    #[error("Token expired")]
    TokenExpired,

    #[error("Insufficient permissions: required {required}, got {actual}")]
    InsufficientPermissions { required: String, actual: String },
}

/// Errors arising from external service calls (e.g. Stellar network).
#[derive(Debug, Error)]
pub enum ExternalServiceError {
    #[error("Request timeout after {timeout_ms}ms")]
    Timeout { timeout_ms: u64 },

    #[error("Service unavailable: {0}")]
    Unavailable(String),

    #[error("Unexpected response: {0}")]
    UnexpectedResponse(String),
}

// ---------------------------------------------------------------------------
// Top-level application error
// ---------------------------------------------------------------------------

/// Structured error response body returned to API clients.
#[derive(Debug, Serialize)]
pub struct ErrorResponse {
    pub code: String,
    pub message: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub details: Option<serde_json::Value>,
}

/// Unified application error that maps all domain errors to HTTP responses.
#[derive(Debug, Error)]
pub enum AppError {
    // --- 400 Bad Request ---
    #[error("Bad request: {0}")]
    BadRequest(String),

    // --- 400 Validation ---
    #[error("Validation error: {0}")]
    Validation(String),

    // --- 401 Unauthorized ---
    #[error("Unauthorized: {0}")]
    Unauthorized(String),

    // --- 403 Forbidden ---
    #[error("Forbidden: {0}")]
    Forbidden(String),

    // --- 404 Not Found ---
    #[error("Not found: {0}")]
    NotFound(String),

    // --- 409 Conflict ---
    #[error("Conflict: {0}")]
    Conflict(String),

    // --- 413 Payload Too Large ---
    #[error("Payload too large: {0}")]
    PayloadTooLarge(String),

    // --- 415 Unsupported Media Type ---
    #[error("Unsupported media type: {0}")]
    UnsupportedMediaType(String),

    // --- 500 Database ---
    #[error("Database error: {0}")]
    Database(#[from] sqlx::Error),

    // --- 500 Redis ---
    #[error("Redis error: {0}")]
    Redis(#[from] redis::RedisError),

    // --- 500 Internal ---
    #[error("Internal error: {0}")]
    Internal(String),

    // --- Domain errors (mapped to appropriate HTTP codes) ---
    #[error(transparent)]
    File(#[from] FileError),

    #[error(transparent)]
    Auth(#[from] AuthError),

    #[error(transparent)]
    Cache(#[from] CacheError),

    #[error(transparent)]
    ExternalService(#[from] ExternalServiceError),
}

impl AppError {
    fn status_and_code(&self) -> (StatusCode, &'static str) {
        match self {
            AppError::BadRequest(_) => (StatusCode::BAD_REQUEST, "bad_request"),
            AppError::Validation(_) => (StatusCode::UNPROCESSABLE_ENTITY, "validation_error"),
            AppError::Unauthorized(_) => (StatusCode::UNAUTHORIZED, "unauthorized"),
            AppError::Forbidden(_) => (StatusCode::FORBIDDEN, "forbidden"),
            AppError::NotFound(_) => (StatusCode::NOT_FOUND, "not_found"),
            AppError::Conflict(_) => (StatusCode::CONFLICT, "conflict"),
            AppError::PayloadTooLarge(_) => (StatusCode::PAYLOAD_TOO_LARGE, "payload_too_large"),
            AppError::UnsupportedMediaType(_) => {
                (StatusCode::UNSUPPORTED_MEDIA_TYPE, "unsupported_media_type")
            }
            AppError::Database(e) => {
                error!("Database error: {e:?}");
                (StatusCode::INTERNAL_SERVER_ERROR, "database_error")
            }
            AppError::Redis(e) => {
                error!("Redis error: {e:?}");
                (StatusCode::INTERNAL_SERVER_ERROR, "redis_error")
            }
            AppError::Internal(msg) => {
                error!("Internal error: {msg}");
                (StatusCode::INTERNAL_SERVER_ERROR, "internal_error")
            }
            AppError::File(e) => match e {
                FileError::TooLarge { .. } => (StatusCode::PAYLOAD_TOO_LARGE, "file_too_large"),
                FileError::UnsupportedMimeType(_) => {
                    (StatusCode::UNSUPPORTED_MEDIA_TYPE, "unsupported_mime_type")
                }
                FileError::InvalidFileName(_) | FileError::MalformedContent(_) => {
                    (StatusCode::BAD_REQUEST, "invalid_file")
                }
            },
            AppError::Auth(e) => match e {
                AuthError::MissingCredentials | AuthError::InvalidToken | AuthError::TokenExpired => {
                    (StatusCode::UNAUTHORIZED, "auth_error")
                }
                AuthError::InsufficientPermissions { .. } => (StatusCode::FORBIDDEN, "forbidden"),
            },
            AppError::Cache(e) => {
                error!("Cache error: {e:?}");
                (StatusCode::INTERNAL_SERVER_ERROR, "cache_error")
            }
            AppError::ExternalService(e) => {
                error!("External service error: {e:?}");
                (StatusCode::BAD_GATEWAY, "external_service_error")
            }
        }
    }
}

impl IntoResponse for AppError {
    fn into_response(self) -> Response {
        let (status, code) = self.status_and_code();
        let message = self.to_string();
        (
            status,
            Json(json!({ "code": code, "message": message })),
        )
            .into_response()
    }
}

// ---------------------------------------------------------------------------
// Conversions from domain errors to AppError
// ---------------------------------------------------------------------------

impl From<DatabaseError> for AppError {
    fn from(e: DatabaseError) -> Self {
        match e {
            DatabaseError::NotFound(msg) => AppError::NotFound(msg),
            DatabaseError::UniqueViolation(msg) => AppError::Conflict(msg),
            DatabaseError::ForeignKeyViolation(msg) => AppError::BadRequest(msg),
            DatabaseError::Connection(msg) => AppError::Internal(msg),
            DatabaseError::Query(sqlx::Error::PoolTimedOut) => {
                AppError::Internal("database pool timed out".into())
            }
            DatabaseError::Query(e) => AppError::Database(e),
        }
    }
}



// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn file_error_too_large_display() {
        let e = FileError::TooLarge { size: 20_000_000, limit: 10_000_000 };
        assert!(e.to_string().contains("20000000"));
    }

    #[test]
    fn file_error_unsupported_mime() {
        let e = FileError::UnsupportedMimeType("application/exe".into());
        assert!(e.to_string().contains("application/exe"));
    }

    #[test]
    fn auth_error_insufficient_permissions_display() {
        let e = AuthError::InsufficientPermissions {
            required: "admin".into(),
            actual: "user".into(),
        };
        assert!(e.to_string().contains("admin"));
    }

    #[test]
    fn app_error_not_found_status() {
        let e = AppError::NotFound("contract".into());
        let (status, code) = e.status_and_code();
        assert_eq!(status, StatusCode::NOT_FOUND);
        assert_eq!(code, "not_found");
    }

    #[test]
    fn app_error_validation_status() {
        let e = AppError::Validation("field required".into());
        let (status, code) = e.status_and_code();
        assert_eq!(status, StatusCode::UNPROCESSABLE_ENTITY);
        assert_eq!(code, "validation_error");
    }

    #[test]
    fn app_error_from_file_too_large() {
        let e = AppError::File(FileError::TooLarge { size: 1, limit: 0 });
        let (status, code) = e.status_and_code();
        assert_eq!(status, StatusCode::PAYLOAD_TOO_LARGE);
        assert_eq!(code, "file_too_large");
    }

    #[test]
    fn app_error_from_auth_forbidden() {
        let e = AppError::Auth(AuthError::InsufficientPermissions {
            required: "admin".into(),
            actual: "user".into(),
        });
        let (status, code) = e.status_and_code();
        assert_eq!(status, StatusCode::FORBIDDEN);
        assert_eq!(code, "forbidden");
    }

    #[test]
    fn database_error_not_found_converts() {
        let e: AppError = DatabaseError::NotFound("user 42".into()).into();
        assert!(matches!(e, AppError::NotFound(_)));
    }

    #[test]
    fn database_error_unique_violation_converts() {
        let e: AppError = DatabaseError::UniqueViolation("email".into()).into();
        assert!(matches!(e, AppError::Conflict(_)));
    }

    #[test]
    fn sqlx_error_converts_to_app_error() {
        let err = sqlx::Error::RowNotFound;
        let e: AppError = err.into();
        assert!(matches!(e, AppError::Database(_)));
    }

    #[tokio::test]
    async fn app_error_into_response_renders_json() {
        let e = AppError::NotFound("contract".into());
        let response = e.into_response();
        let body = response.into_body();
        let body = axum::body::to_bytes(body, usize::MAX).await.unwrap();
        let json: serde_json::Value = serde_json::from_slice(&body).unwrap();
        assert_eq!(json["code"], "not_found");
    }

    #[test]
    fn error_response_serializes() {
        let resp = ErrorResponse {
            code: "not_found".into(),
            message: "Resource not found".into(),
            details: None,
        };
        let json = serde_json::to_string(&resp).unwrap();
        assert!(json.contains("\"code\":\"not_found\""));
        assert!(!json.contains("details"));
    }
}
