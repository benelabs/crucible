use axum::{
    extract::Request,
    http::{header, Method},
    middleware::Next,
    response::Response,
};

use crate::error::AppError;

/// Middleware that validates the Content-Type header on mutating requests.
///
/// For POST, PUT, and PATCH requests, this middleware ensures the client
/// sends `Content-Type: application/json`. Safe methods (GET, HEAD, OPTIONS)
/// are allowed through without validation.
///
/// Returns `415 Unsupported Media Type` if the Content-Type is missing or
/// does not start with `application/json`.
pub async fn require_json_content_type(req: Request, next: Next) -> Result<Response, AppError> {
    if req.method() == Method::GET || req.method() == Method::HEAD || req.method() == Method::OPTIONS {
        return Ok(next.run(req).await);
    }

    let content_type = req
        .headers()
        .get(header::CONTENT_TYPE)
        .and_then(|v| v.to_str().ok())
        .unwrap_or("");

    if content_type.starts_with("application/json") {
        Ok(next.run(req).await)
    } else {
        Err(AppError::UnsupportedMediaType(
            "Expected Content-Type: application/json".into(),
        ))
    }
}
