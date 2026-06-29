//! Concurrent load tests for the `POST /api/profile` endpoint.
//!
//! These tests verify that the profiling trigger handler remains stable and
//! correct under concurrent load without requiring a live database or Redis.
//!
//! # Running
//!
//! ```bash
//! cargo test -p backend --test load_tests load::profile_load -- --nocapture
//! ```

use std::sync::Arc;
use std::time::Instant;

use axum::http::StatusCode;
use axum::{body::to_bytes, routing::post, Router};
use hyper::Request;
use tower::ServiceExt;

use backend::api::handlers::profiling::{trigger_profile_collection, AppState};
use backend::config::{reload::ConfigManager, AppConfig};
use backend::config::{reload::ConfigManager, AppConfig};
use backend::services::{
    contract_benchmark::ContractBenchmarkService, error_recovery::ErrorManager,
    log_aggregator::LogAggregator, sys_metrics::MetricsExporter,
};
use backend::services::{error_recovery::ErrorManager, sys_metrics::MetricsExporter};
use redis::Client as RedisClient;

use crate::load::framework::{assert_load_result, LoadConfig, LoadResult};

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

/// Build a test router wired to the `POST /api/profile` handler.
fn build_app() -> Router {
    let (log_aggregator, _rx) = backend::services::log_aggregator::LogAggregator::new();
    let (log_aggregator, _receiver) = LogAggregator::new();
    let state = Arc::new(AppState {
        db: None,
        metrics_exporter: Arc::new(MetricsExporter::new()),
        error_manager: Arc::new(ErrorManager::new()),
        config_manager: Arc::new(ConfigManager::new(AppConfig::default())),
        log_aggregator: Arc::new(backend::services::log_aggregator::LogAggregator::new().0),
        redis: redis::Client::open("redis://127.0.0.1/").unwrap(),
        log_aggregator: Arc::new(log_aggregator),
        redis: redis::Client::open("redis://127.0.0.1:1/").unwrap(),
        contract_benchmark_service: Arc::new(ContractBenchmarkService::new()),
        redis: RedisClient::open("redis://127.0.0.1:1/").unwrap(),
    });
    Router::new()
        .route("/api/profile", post(trigger_profile_collection))
        .with_state(state)
}

/// Build a valid profile trigger request body.
fn profile_request_body(label: &str) -> axum::body::Body {
    axum::body::Body::from(
        serde_json::json!({
            "duration_secs": 10,
            "sample_rate_hz": 100,
            "label": label
        })
        .to_string(),
    )
}

/// Fire `n` concurrent requests and assert all return 200.
async fn run_concurrent(n: usize) {
    let handles: Vec<_> = (0..n)
        .map(|i| {
            let app = build_app();
            tokio::spawn(async move {
                let resp = app
                    .oneshot(
                        Request::builder()
                            .method("POST")
                            .uri("/api/profile")
                            .header("content-type", "application/json")
                            .body(profile_request_body(&format!("load-test-{i}")))
                            .unwrap(),
                    )
                    .await
                    .unwrap();
                resp.status()
            })
        })
        .collect();

    for handle in handles {
        let status = handle.await.unwrap();
        assert_eq!(status, StatusCode::OK);
    }
}

/// Run a full load test using the framework and return the [`LoadResult`].
async fn run_framework_load(concurrency: usize, requests_per_task: usize) -> LoadResult {
    use crate::load::framework::run_load;

    let cfg = LoadConfig::new(concurrency, requests_per_task);
    run_load(cfg, || async {
        let app = build_app();
        let start = Instant::now();
        let resp = app
            .oneshot(
                Request::builder()
                    .method("POST")
                    .uri("/api/profile")
                    .header("content-type", "application/json")
                    .body(profile_request_body("load-test"))
                    .unwrap(),
            )
            .await
            .unwrap();
        (resp.status(), start.elapsed())
    })
    .await
}

// ---------------------------------------------------------------------------
// Basic concurrency tests
// ---------------------------------------------------------------------------

#[tokio::test]
async fn test_profile_10_concurrent() {
    run_concurrent(10).await;
}

#[tokio::test]
async fn test_profile_50_concurrent() {
    run_concurrent(50).await;
}

// ---------------------------------------------------------------------------
// Response shape
// ---------------------------------------------------------------------------

/// Verify response body shape.
#[tokio::test]
async fn test_profile_response_shape() {
    let app = build_app();
    let resp = app
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/api/profile")
                .header("content-type", "application/json")
                .body(profile_request_body("shape-test"))
                .unwrap(),
        )
        .await
        .unwrap();

    assert_eq!(resp.status(), StatusCode::OK);

    let bytes = to_bytes(resp.into_body(), usize::MAX).await.unwrap();
    let json: serde_json::Value = serde_json::from_slice(&bytes).unwrap();

    assert!(json.get("data").is_some(), "response must have 'data' key");
    assert!(
        json["data"].get("message").is_some(),
        "data must have 'message' key"
    );
    assert!(
        json["data"].get("profile_id").is_some(),
        "data must have 'profile_id' key"
    );
    assert!(
        json["data"].get("estimated_completion").is_some(),
        "data must have 'estimated_completion' key"
    );
}

/// Verify each response contains a unique profile_id.
#[tokio::test]
async fn test_profile_unique_ids() {
    use std::collections::HashSet;

    let mut ids = HashSet::new();
    for i in 0..10 {
        let app = build_app();
        let resp = app
            .oneshot(
                Request::builder()
                    .method("POST")
                    .uri("/api/profile")
                    .header("content-type", "application/json")
                    .body(profile_request_body(&format!("unique-id-test-{i}")))
                    .unwrap(),
            )
            .await
            .unwrap();

        let bytes = to_bytes(resp.into_body(), usize::MAX).await.unwrap();
        let json: serde_json::Value = serde_json::from_slice(&bytes).unwrap();
        let id = json["data"]["profile_id"]
            .as_str()
            .expect("profile_id must be a string")
            .to_string();
        ids.insert(id);
    }

    assert_eq!(ids.len(), 10, "all 10 profile IDs must be unique");
}

/// Verify the `message` field contains the label from the request.
#[tokio::test]
async fn test_profile_message_contains_label() {
    let app = build_app();
    let label = "my-custom-label";
    let resp = app
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/api/profile")
                .header("content-type", "application/json")
                .body(profile_request_body(label))
                .unwrap(),
        )
        .await
        .unwrap();

    let bytes = to_bytes(resp.into_body(), usize::MAX).await.unwrap();
    let json: serde_json::Value = serde_json::from_slice(&bytes).unwrap();
    let message = json["data"]["message"].as_str().unwrap();
    assert!(
        message.contains(label),
        "message '{message}' must contain label '{label}'"
    );
}

// ---------------------------------------------------------------------------
// Validation tests
// ---------------------------------------------------------------------------

/// Verify that a missing `label` field returns 400 / 422.
#[tokio::test]
async fn test_profile_missing_label_rejected() {
    let app = build_app();
    let resp = app
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/api/profile")
                .header("content-type", "application/json")
                .body(axum::body::Body::from(
                    serde_json::json!({
                        "duration_secs": 10,
                        "sample_rate_hz": 100,
                        "label": ""
                    })
                    .to_string(),
                ))
                .unwrap(),
        )
        .await
        .unwrap();

    // Empty label should fail validation → 400 or 422
    assert!(
        resp.status() == StatusCode::BAD_REQUEST
            || resp.status() == StatusCode::UNPROCESSABLE_ENTITY,
        "expected 400 or 422, got {}",
        resp.status()
    );
}

/// Verify that `duration_secs = 0` is rejected.
#[tokio::test]
async fn test_profile_zero_duration_rejected() {
    let app = build_app();
    let resp = app
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/api/profile")
                .header("content-type", "application/json")
                .body(axum::body::Body::from(
                    serde_json::json!({
                        "duration_secs": 0,
                        "sample_rate_hz": 100,
                        "label": "test"
                    })
                    .to_string(),
                ))
                .unwrap(),
        )
        .await
        .unwrap();

    assert!(
        resp.status() == StatusCode::BAD_REQUEST
            || resp.status() == StatusCode::UNPROCESSABLE_ENTITY,
        "expected 400 or 422, got {}",
        resp.status()
    );
}

/// Verify that `duration_secs` exceeding 3600 is rejected.
#[tokio::test]
async fn test_profile_excessive_duration_rejected() {
    let app = build_app();
    let resp = app
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/api/profile")
                .header("content-type", "application/json")
                .body(axum::body::Body::from(
                    serde_json::json!({
                        "duration_secs": 9999,
                        "sample_rate_hz": 100,
                        "label": "test"
                    })
                    .to_string(),
                ))
                .unwrap(),
        )
        .await
        .unwrap();

    assert!(
        resp.status() == StatusCode::BAD_REQUEST
            || resp.status() == StatusCode::UNPROCESSABLE_ENTITY,
        "expected 400 or 422, got {}",
        resp.status()
    );
}

/// Verify that a non-JSON body returns 400 / 415.
#[tokio::test]
async fn test_profile_non_json_body_rejected() {
    let app = build_app();
    let resp = app
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/api/profile")
                .header("content-type", "text/plain")
                .body(axum::body::Body::from("not json"))
                .unwrap(),
        )
        .await
        .unwrap();

    assert!(
        resp.status().is_client_error(),
        "expected 4xx, got {}",
        resp.status()
    );
}

// ---------------------------------------------------------------------------
// Framework-based load tests with SLO assertions
// ---------------------------------------------------------------------------

/// 10 concurrent tasks × 10 requests each = 100 total.
/// SLO: 0% errors, p99 < 500ms.
#[tokio::test]
async fn test_profile_load_100_requests_slo() {
    let result = run_framework_load(10, 10).await;
    result.print_summary("POST /api/profile — 100 requests");
    assert_load_result(&result, 0.0, std::time::Duration::from_millis(500));
}

/// 20 concurrent tasks × 10 requests each = 200 total.
/// SLO: 0% errors, p99 < 1s.
#[tokio::test]
async fn test_profile_load_200_requests_slo() {
    let result = run_framework_load(20, 10).await;
    result.print_summary("POST /api/profile — 200 requests");
    assert_load_result(&result, 0.0, std::time::Duration::from_secs(1));
}

/// Verify that all responses under load have the correct JSON shape.
#[tokio::test]
async fn test_profile_load_response_shape_under_load() {
    let mut join_set = tokio::task::JoinSet::new();
    for i in 0..5_usize {
        join_set.spawn(async move {
            let mut results = Vec::new();
            for j in 0..4_usize {
                let app = build_app();
                let resp = app
                    .oneshot(
                        Request::builder()
                            .method("POST")
                            .uri("/api/profile")
                            .header("content-type", "application/json")
                            .body(profile_request_body(&format!("task-{i}-req-{j}")))
                            .unwrap(),
                    )
                    .await
                    .unwrap();
                let status = resp.status();
                let bytes = to_bytes(resp.into_body(), usize::MAX).await.unwrap();
                results.push((status, bytes.to_vec()));
            }
            results
        });
    }

    while let Some(Ok(batch)) = join_set.join_next().await {
        for (status, body) in batch {
            assert_eq!(status, StatusCode::OK);
            let json: serde_json::Value = serde_json::from_slice(&body).unwrap();
            assert_eq!(json["status"], "success");
            assert!(json["data"].get("profile_id").is_some());
            assert!(json["data"].get("message").is_some());
            assert!(json["data"].get("estimated_completion").is_some());
        }
    }
}

/// Verify that concurrent requests each produce a unique profile_id.
#[tokio::test]
async fn test_profile_concurrent_unique_ids() {
    use std::collections::HashSet;
    use std::sync::Mutex;

    let ids = Arc::new(Mutex::new(HashSet::new()));
    let mut join_set = tokio::task::JoinSet::new();

    for i in 0..20_usize {
        let ids_clone = ids.clone();
        join_set.spawn(async move {
            let app = build_app();
            let resp = app
                .oneshot(
                    Request::builder()
                        .method("POST")
                        .uri("/api/profile")
                        .header("content-type", "application/json")
                        .body(profile_request_body(&format!("concurrent-{i}")))
                        .unwrap(),
                )
                .await
                .unwrap();
            let bytes = to_bytes(resp.into_body(), usize::MAX).await.unwrap();
            let json: serde_json::Value = serde_json::from_slice(&bytes).unwrap();
            let id = json["data"]["profile_id"].as_str().unwrap().to_string();
            ids_clone.lock().unwrap().insert(id);
        });
    }

    while join_set.join_next().await.is_some() {}

    let collected = ids.lock().unwrap();
    assert_eq!(
        collected.len(),
        20,
        "all 20 concurrent profile IDs must be unique"
    );
}
