//! Concurrent load tests for the `GET /api/status` endpoint.
//!
//! These tests verify that the status handler remains stable and correct
//! under concurrent load without requiring a live database or Redis instance.
//!
//! # Running
//!
//! ```bash
//! cargo test -p backend --test load_tests load::status_load -- --nocapture
//! ```

use std::sync::Arc;
use std::time::Instant;

use axum::http::StatusCode;
use axum::{body::to_bytes, routing::get, Router};
use hyper::Request;
use tower::ServiceExt;

use backend::api::handlers::profiling::{get_system_status, AppState};
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

/// Build a test router wired to the `/api/status` handler with mock state.
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
        .route("/api/status", get(get_system_status))
        .with_state(state)
}

/// Fire `n` concurrent requests and assert all return 200.
async fn run_concurrent(n: usize) {
    let handles: Vec<_> = (0..n)
        .map(|_| {
            let app = build_app();
            tokio::spawn(async move {
                let resp = app
                    .oneshot(
                        Request::builder()
                            .uri("/api/status")
                            .body(axum::body::Body::empty())
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
                    .uri("/api/status")
                    .body(axum::body::Body::empty())
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
async fn test_status_10_concurrent() {
    run_concurrent(10).await;
}

#[tokio::test]
async fn test_status_50_concurrent() {
    run_concurrent(50).await;
}

#[tokio::test]
async fn test_status_100_concurrent() {
    run_concurrent(100).await;
}

// ---------------------------------------------------------------------------
// Sequential stability
// ---------------------------------------------------------------------------

/// Verify that repeated sequential requests all succeed.
#[tokio::test]
async fn test_status_sequential_stability() {
    let app = build_app();
    for _ in 0..20 {
        let resp = app
            .clone()
            .oneshot(
                Request::builder()
                    .uri("/api/status")
                    .body(axum::body::Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(resp.status(), StatusCode::OK);
    }
}

// ---------------------------------------------------------------------------
// Response shape
// ---------------------------------------------------------------------------

/// Verify response body contains expected JSON keys.
#[tokio::test]
async fn test_status_response_shape() {
    let app = build_app();
    let resp = app
        .oneshot(
            Request::builder()
                .uri("/api/status")
                .body(axum::body::Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();

    assert_eq!(resp.status(), StatusCode::OK);

    let bytes = to_bytes(resp.into_body(), usize::MAX).await.unwrap();
    let json: serde_json::Value = serde_json::from_slice(&bytes).unwrap();

    assert_eq!(json["status"], "success");
    assert!(json.get("data").is_some(), "response must have 'data' key");
    assert!(
        json["data"].get("status").is_some(),
        "data must have 'status' key"
    );
    assert!(
        json["data"].get("uptime_secs").is_some(),
        "data must have 'uptime_secs' key"
    );
    assert!(
        json["data"].get("active_recovery_tasks").is_some(),
        "data must have 'active_recovery_tasks' key"
    );
}

/// Verify the `status` field value is `"healthy"`.
#[tokio::test]
async fn test_status_healthy_value() {
    let app = build_app();
    let resp = app
        .oneshot(
            Request::builder()
                .uri("/api/status")
                .body(axum::body::Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();

    let bytes = to_bytes(resp.into_body(), usize::MAX).await.unwrap();
    let json: serde_json::Value = serde_json::from_slice(&bytes).unwrap();
    assert_eq!(json["data"]["status"], "healthy");
}

/// Verify `active_recovery_tasks` starts at zero with a fresh state.
#[tokio::test]
async fn test_status_zero_recovery_tasks_initially() {
    let app = build_app();
    let resp = app
        .oneshot(
            Request::builder()
                .uri("/api/status")
                .body(axum::body::Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();

    let bytes = to_bytes(resp.into_body(), usize::MAX).await.unwrap();
    let json: serde_json::Value = serde_json::from_slice(&bytes).unwrap();
    assert_eq!(json["data"]["active_recovery_tasks"], 0);
}

/// Verify `uptime_secs` is a non-negative integer.
#[tokio::test]
async fn test_status_uptime_is_non_negative() {
    let app = build_app();
    let resp = app
        .oneshot(
            Request::builder()
                .uri("/api/status")
                .body(axum::body::Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();

    let bytes = to_bytes(resp.into_body(), usize::MAX).await.unwrap();
    let json: serde_json::Value = serde_json::from_slice(&bytes).unwrap();
    let uptime = json["data"]["uptime_secs"].as_u64();
    assert!(
        uptime.is_some(),
        "uptime_secs must be a non-negative integer"
    );
}

// ---------------------------------------------------------------------------
// Framework-based load tests with SLO assertions
// ---------------------------------------------------------------------------

/// 10 concurrent tasks × 10 requests each = 100 total.
/// SLO: 0% errors, p99 < 500ms.
#[tokio::test]
async fn test_status_load_100_requests_slo() {
    let result = run_framework_load(10, 10).await;
    result.print_summary("GET /api/status — 100 requests");
    assert_load_result(&result, 0.0, std::time::Duration::from_millis(500));
}

/// 20 concurrent tasks × 10 requests each = 200 total.
/// SLO: 0% errors, p99 < 1s.
#[tokio::test]
async fn test_status_load_200_requests_slo() {
    let result = run_framework_load(20, 10).await;
    result.print_summary("GET /api/status — 200 requests");
    assert_load_result(&result, 0.0, std::time::Duration::from_secs(1));
}

/// Verify that all responses under load have the correct JSON shape.
#[tokio::test]
async fn test_status_load_response_shape_under_load() {
    use crate::load::framework::run_load;

    let cfg = LoadConfig::new(5, 4); // 20 total
    let outcomes: Vec<(StatusCode, Vec<u8>)> = {
        let mut join_set = tokio::task::JoinSet::new();
        for _ in 0..cfg.concurrency {
            join_set.spawn(async {
                let mut results = Vec::new();
                for _ in 0..4 {
                    let app = build_app();
                    let resp = app
                        .oneshot(
                            Request::builder()
                                .uri("/api/status")
                                .body(axum::body::Body::empty())
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
        let mut all = Vec::new();
        while let Some(Ok(batch)) = join_set.join_next().await {
            all.extend(batch);
        }
        all
    };

    for (status, body) in outcomes {
        assert_eq!(status, StatusCode::OK);
        let json: serde_json::Value = serde_json::from_slice(&body).unwrap();
        assert_eq!(json["status"], "success");
        assert!(json["data"].get("status").is_some());
        assert!(json["data"].get("uptime_secs").is_some());
        assert!(json["data"].get("active_recovery_tasks").is_some());
    }
}

/// Verify that the handler is idempotent — repeated calls return the same shape.
#[tokio::test]
async fn test_status_idempotent_responses() {
    let app = build_app();
    let mut previous: Option<serde_json::Value> = None;

    for _ in 0..5 {
        let resp = app
            .clone()
            .oneshot(
                Request::builder()
                    .uri("/api/status")
                    .body(axum::body::Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();

        let bytes = to_bytes(resp.into_body(), usize::MAX).await.unwrap();
        let json: serde_json::Value = serde_json::from_slice(&bytes).unwrap();

        if let Some(ref prev) = previous {
            // Keys must be identical; values may differ (e.g. uptime_secs)
            assert_eq!(
                prev.as_object().unwrap().keys().collect::<Vec<_>>(),
                json.as_object().unwrap().keys().collect::<Vec<_>>(),
                "response keys must be stable across calls"
            );
        }
        previous = Some(json);
    }
}

/// Verify that the handler correctly reflects recovery tasks added to state.
#[tokio::test]
async fn test_status_reflects_recovery_tasks() {
    use backend::services::error_recovery::RecoveryError;

    let error_manager = Arc::new(ErrorManager::new());
    error_manager
        .handle_error(RecoveryError::Internal("boom".into()), "worker_a")
        .await
        .unwrap();

    let (log_aggregator, _rx) = backend::services::log_aggregator::LogAggregator::new();
    let state = Arc::new(AppState {
        db: None,
        metrics_exporter: Arc::new(MetricsExporter::new()),
        error_manager: error_manager.clone(),
        config_manager: Arc::new(ConfigManager::new(AppConfig::default())),
        log_aggregator: Arc::new(log_aggregator),
        redis: redis::Client::open("redis://127.0.0.1:1/").unwrap(),
    });

    let app = Router::new()
        .route("/api/status", get(get_system_status))
        .with_state(state);

    let resp = app
        .oneshot(
            Request::builder()
                .uri("/api/status")
                .body(axum::body::Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();

    let bytes = to_bytes(resp.into_body(), usize::MAX).await.unwrap();
    let json: serde_json::Value = serde_json::from_slice(&bytes).unwrap();
    assert_eq!(json["data"]["active_recovery_tasks"], 1);
}

/// Verify that the handler correctly reflects updated metrics.
#[tokio::test]
async fn test_status_reflects_updated_metrics() {
    let metrics_exporter = Arc::new(MetricsExporter::new());
    metrics_exporter.update_metrics(55.0, 2048, 300).await;

    let (log_aggregator, _rx) = backend::services::log_aggregator::LogAggregator::new();
    let state = Arc::new(AppState {
        db: None,
        metrics_exporter: metrics_exporter.clone(),
        error_manager: Arc::new(ErrorManager::new()),
        config_manager: Arc::new(ConfigManager::new(AppConfig::default())),
        log_aggregator: Arc::new(log_aggregator),
        redis: redis::Client::open("redis://127.0.0.1:1/").unwrap(),
    });

    let app = Router::new()
        .route("/api/status", get(get_system_status))
        .with_state(state);

    let resp = app
        .oneshot(
            Request::builder()
                .uri("/api/status")
                .body(axum::body::Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();

    let bytes = to_bytes(resp.into_body(), usize::MAX).await.unwrap();
    let json: serde_json::Value = serde_json::from_slice(&bytes).unwrap();
    assert_eq!(json["data"]["uptime_secs"], 300);
    assert_eq!(json["data"]["memory_used_bytes"], 2048);
}
