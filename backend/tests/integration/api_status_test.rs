//! Integration tests for `GET /api/status`.

use axum::body::Body;
use hyper::{Request, StatusCode};
use tower::ServiceExt;

use crate::integration::test_app;

#[tokio::test]
async fn status_returns_200() {
    let response = test_app()
        .oneshot(
            Request::builder()
                .uri("/api/status")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();

    assert_eq!(response.status(), StatusCode::OK);
}

#[tokio::test]
async fn status_body_is_valid_json() {
    let response = test_app()
        .oneshot(
            Request::builder()
                .uri("/api/status")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();

    let bytes = axum::body::to_bytes(response.into_body(), usize::MAX)
        .await
        .unwrap();
    let json: serde_json::Value = serde_json::from_slice(&bytes).expect("response must be JSON");

    assert_eq!(json["status"], "success");
    let data = &json["data"];
    assert_eq!(data["status"], "healthy");
    assert!(data["uptime_secs"].is_number());
    assert!(data["memory_used_bytes"].is_number());
    assert!(data["active_recovery_tasks"].is_number());
}

#[tokio::test]
async fn status_metrics_fields_present() {
    let response = test_app()
        .oneshot(
            Request::builder()
                .uri("/api/status")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();

    let bytes = axum::body::to_bytes(response.into_body(), usize::MAX)
        .await
        .unwrap();
    let json: serde_json::Value = serde_json::from_slice(&bytes).unwrap();
    let data = &json["data"];

    assert_eq!(data["status"], "healthy");
    assert!(data["uptime_secs"].is_number());
    assert!(data["memory_used_bytes"].is_number());
    assert!(data["active_recovery_tasks"].is_number());
}
