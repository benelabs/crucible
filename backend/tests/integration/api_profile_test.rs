//! Integration tests for `POST /api/profile`.

use axum::body::Body;
use hyper::{Request, StatusCode};
use tower::ServiceExt;

use crate::integration::test_app;

#[tokio::test]
async fn profile_returns_200() {
    let response = test_app()
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/api/profile")
                .header("content-type", "application/json")
                .body(Body::from(
                    serde_json::json!({
                        "duration_secs": 10,
                        "sample_rate_hz": 100,
                        "label": "integration-test"
                    })
                    .to_string(),
                ))
                .unwrap(),
        )
        .await
        .unwrap();

    assert_eq!(response.status(), StatusCode::OK);
}

#[tokio::test]
async fn profile_body_contains_profile_id() {
    let response = test_app()
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/api/profile")
                .header("content-type", "application/json")
                .body(Body::from(
                    serde_json::json!({
                        "duration_secs": 10,
                        "sample_rate_hz": 100,
                        "label": "integration-test"
                    })
                    .to_string(),
                ))
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
    assert!(data["message"]
        .as_str()
        .unwrap()
        .starts_with("Profiling collection triggered"));
    assert!(
        data["profile_id"].is_string(),
        "profile_id must be a string"
    );

    // profile_id should be a valid UUID
    let id = data["profile_id"].as_str().unwrap();
    uuid::Uuid::parse_str(id).expect("profile_id must be a valid UUID");
}

#[tokio::test]
async fn profile_ids_are_unique_per_request() {
    let app = test_app();

    let make_request = || {
        Request::builder()
            .method("POST")
            .uri("/api/profile")
            .header("content-type", "application/json")
            .body(Body::from(
                serde_json::json!({
                    "duration_secs": 10,
                    "sample_rate_hz": 100,
                    "label": "integration-test"
                })
                .to_string(),
            ))
            .unwrap()
    };

    let r1 = app.clone().oneshot(make_request()).await.unwrap();
    let r2 = app.oneshot(make_request()).await.unwrap();

    let b1 = axum::body::to_bytes(r1.into_body(), usize::MAX)
        .await
        .unwrap();
    let b2 = axum::body::to_bytes(r2.into_body(), usize::MAX)
        .await
        .unwrap();

    let j1: serde_json::Value = serde_json::from_slice(&b1).unwrap();
    let j2: serde_json::Value = serde_json::from_slice(&b2).unwrap();

    assert_ne!(
        j1["data"]["profile_id"], j2["data"]["profile_id"],
        "each request must produce a unique profile_id"
    );
}
