//! Standardized error response structures and response helpers for the Crucible API.

use axum::{
    http::StatusCode,
    response::{IntoResponse, Response},
    Json,
};
use serde::Serialize;

/// Standardized JSON error response returned by middlewares and fallbacks.
#[derive(Debug, Serialize)]
pub struct ErrorResponse {
    pub code: String,
    pub message: String,
    pub timestamp: String,
}

/// Create a standardized axum JSON response with a timestamp.
pub fn make_error_response(status: StatusCode, code: &str, message: &str) -> Response {
    (
        status,
        Json(ErrorResponse {
            code: code.to_string(),
            message: message.to_string(),
            timestamp: chrono::Utc::now().to_rfc3339(),
        }),
    )
        .into_response()
}

/// A fallback handler returning a standardized 404 JSON response.
pub async fn api_fallback() -> impl IntoResponse {
    make_error_response(
        StatusCode::NOT_FOUND,
        "not_found",
        "The requested resource was not found",
    )
}
