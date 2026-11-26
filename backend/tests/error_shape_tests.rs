//! All error paths must produce one JSON shape: handler errors, middleware
//! rejections and the router fallback.

use axum::{
    body::Body,
    http::{Request, StatusCode},
    middleware::{self, Next},
    response::Response,
    routing::get,
    Router,
};
use backend::api::errors::{api_fallback, make_error_response};
use backend::api::middleware::request_id::request_id_middleware;
use backend::error::AppError;
use serde_json::Value;
use std::collections::BTreeSet;
use tower::ServiceExt;

async fn failing_handler() -> Result<&'static str, AppError> {
    Err(AppError::NotFound("missing thing".into()))
}

async fn rejecting_middleware(req: Request<Body>, next: Next) -> Response {
    if req.uri().path() == "/rejected" {
        return make_error_response(StatusCode::FORBIDDEN, "forbidden", "nope");
    }
    next.run(req).await
}

fn app() -> Router {
    Router::new()
        .route("/handler-error", get(failing_handler))
        .route("/rejected", get(|| async { "unreachable" }))
        .layer(middleware::from_fn(rejecting_middleware))
        .fallback(api_fallback)
        .layer(middleware::from_fn(request_id_middleware))
}

async fn body_json(app: &Router, path: &str) -> (StatusCode, Value) {
    let resp = app
        .clone()
        .oneshot(
            Request::builder()
                .uri(path)
                .header("x-request-id", "req-123")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    let status = resp.status();
    let bytes = axum::body::to_bytes(resp.into_body(), usize::MAX)
        .await
        .unwrap();
    (status, serde_json::from_slice(&bytes).unwrap())
}

fn keys(v: &Value) -> BTreeSet<String> {
    v.as_object().unwrap().keys().cloned().collect()
}

#[tokio::test]
async fn handler_middleware_and_fallback_errors_share_one_shape() {
    let app = app();

    let (s1, handler) = body_json(&app, "/handler-error").await;
    let (s2, middleware) = body_json(&app, "/rejected").await;
    let (s3, fallback) = body_json(&app, "/does-not-exist").await;

    assert_eq!(s1, StatusCode::NOT_FOUND);
    assert_eq!(s2, StatusCode::FORBIDDEN);
    assert_eq!(s3, StatusCode::NOT_FOUND);

    assert_eq!(keys(&handler), keys(&middleware));
    assert_eq!(keys(&handler), keys(&fallback));
    for k in ["code", "message", "timestamp", "request_id"] {
        assert!(handler.get(k).is_some(), "missing {k}");
    }
    assert_eq!(handler["code"], "not_found");
    assert_eq!(middleware["code"], "forbidden");
    assert_eq!(fallback["code"], "not_found");
    assert_eq!(handler["request_id"], "req-123");
    assert_eq!(middleware["request_id"], "req-123");
    assert_eq!(fallback["request_id"], "req-123");
}

#[tokio::test]
async fn service_unavailable_uses_shared_shape_with_details() {
    use axum::response::IntoResponse;
    let resp = AppError::ServiceUnavailable { retry_after: 7 }.into_response();
    let bytes = axum::body::to_bytes(resp.into_body(), usize::MAX)
        .await
        .unwrap();
    let v: Value = serde_json::from_slice(&bytes).unwrap();
    assert_eq!(v["code"], "service_unavailable");
    assert_eq!(v["details"]["retry_after_seconds"], 7);
    assert!(v["timestamp"].is_string());
}
