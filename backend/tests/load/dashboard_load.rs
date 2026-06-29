//! Concurrent load tests for the `GET /api/dashboard` endpoint.
//!
//! These tests verify that the dashboard handler remains stable and correct
//! under concurrent load. The handler degrades gracefully when Redis is
//! unavailable (falls back to live service data), so tests run without any
//! external infrastructure.
//!
//! # Running
//!
//! ```bash
//! cargo test -p backend --test load_tests load::dashboard_load -- --nocapture
//! ```

use std::sync::Arc;
use std::time::Instant;

use axum::http::StatusCode;
use axum::{body::to_bytes, routing::get, Router};
use hyper::Request;
use tower::ServiceExt;

use backend::api::handlers::dashboard::{get_dashboard, DashboardState};
use backend::services::{
    alerts::AlertDispatcher, error_recovery::ErrorManager, log_alerts::AlertManager,
    sys_metrics::MetricsExporter,
};

use crate::load::framework::{assert_load_result, LoadConfig, LoadResult};

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

/// Build a test router wired to `GET /api/dashboard` with mock state.
///
/// Redis is pointed at a port that will refuse connections so the handler
/// exercises its graceful-degradation path (cache miss → live data).
fn build_app() -> Router {
    let state = Arc::new(DashboardState {
        metrics_exporter: Arc::new(MetricsExporter::new()),
        error_manager: Arc::new(ErrorManager::new()),
        alert_manager: Arc::new(AlertManager::new()),
        // Unreachable Redis — handler must degrade gracefully.
        redis: redis::Client::open("redis://127.0.0.1:1/").unwrap(),
    });
    Router::new()
        .route("/api/dashboard", get(get_dashboard))
        .with_state(state)
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
                    .uri("/api/dashboard")
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
// Basic correctness
// ---------------------------------------------------------------------------

/// Dashboard returns 200 even when Redis is unreachable.
#[tokio::test]
async fn test_dashboard_returns_200_without_redis() {
    let app = build_app();
    let resp = app
        .oneshot(
            Request::builder()
                .uri("/api/dashboard")
                .body(axum::body::Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();

    assert_eq!(resp.status(), StatusCode::OK);
}

/// Response body contains the three top-level keys.
#[tokio::test]
async fn test_dashboard_response_shape() {
    let app = build_app();
    let resp = app
        .oneshot(
            Request::builder()
                .uri("/api/dashboard")
                .body(axum::body::Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();

    let bytes = to_bytes(resp.into_body(), usize::MAX).await.unwrap();
    let json: serde_json::Value = serde_json::from_slice(&bytes).unwrap();

    assert!(json.get("metrics").is_some(), "must have 'metrics'");
    assert!(
        json.get("active_recovery_tasks").is_some(),
        "must have 'active_recovery_tasks'"
    );
    assert!(
        json.get("active_alerts").is_some(),
        "must have 'active_alerts'"
    );
}

/// `metrics` object contains the expected sub-fields.
#[tokio::test]
async fn test_dashboard_metrics_fields() {
    let state = Arc::new(DashboardState {
        metrics_exporter: Arc::new(MetricsExporter::new()),
        error_manager: Arc::new(ErrorManager::new()),
        alert_manager: Arc::new(AlertManager::new()),
        redis: redis::Client::open("redis://127.0.0.1:1/").unwrap(),
    });
    // Seed some metrics so the values are non-zero.
    state.metrics_exporter.update_metrics(42.0, 2048, 120).await;

    let app = Router::new()
        .route("/api/dashboard", get(get_dashboard))
        .with_state(state);

    let resp = app
        .oneshot(
            Request::builder()
                .uri("/api/dashboard")
                .body(axum::body::Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();

    let bytes = to_bytes(resp.into_body(), usize::MAX).await.unwrap();
    let json: serde_json::Value = serde_json::from_slice(&bytes).unwrap();

    assert_eq!(json["metrics"]["cpu_usage"], 42.0);
    assert_eq!(json["metrics"]["memory_usage"], 2048);
    assert_eq!(json["metrics"]["uptime"], 120);
}

/// `active_recovery_tasks` reflects tasks registered in the error manager.
#[tokio::test]
async fn test_dashboard_includes_recovery_tasks() {
    use backend::services::error_recovery::RecoveryError;

    let error_manager = Arc::new(ErrorManager::new());
    error_manager
        .handle_error(RecoveryError::Internal("boom".into()), "worker_a")
        .await
        .unwrap();

    let state = Arc::new(DashboardState {
        metrics_exporter: Arc::new(MetricsExporter::new()),
        error_manager,
        alert_manager: Arc::new(AlertManager::new()),
        redis: redis::Client::open("redis://127.0.0.1:1/").unwrap(),
    });

    let app = Router::new()
        .route("/api/dashboard", get(get_dashboard))
        .with_state(state);

    let resp = app
        .oneshot(
            Request::builder()
                .uri("/api/dashboard")
                .body(axum::body::Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();

    let bytes = to_bytes(resp.into_body(), usize::MAX).await.unwrap();
    let json: serde_json::Value = serde_json::from_slice(&bytes).unwrap();

    let tasks = json["active_recovery_tasks"].as_array().unwrap();
    assert_eq!(tasks.len(), 1);
    assert_eq!(tasks[0]["name"], "worker_a");
}

/// `active_alerts` reflects alerts fired by the alert manager.
#[tokio::test]
async fn test_dashboard_includes_active_alerts() {
    use backend::services::log_aggregator::LogEntry;
    use backend::services::log_alerts::{AlertRule, AlertSeverity};
    use chrono::Utc;
    use uuid::Uuid;

    let alert_manager = Arc::new(AlertManager::new());
    alert_manager
        .add_rule(AlertRule {
            id: Uuid::new_v4(),
            name: "test-rule".to_string(),
            pattern: "CRITICAL".to_string(),
            severity: AlertSeverity::Critical,
            threshold: 1,
            window_secs: 60,
        })
        .await
        .unwrap();

    alert_manager
        .evaluate(&LogEntry {
            timestamp: Utc::now(),
            level: "ERROR".to_string(),
            message: "CRITICAL failure detected".to_string(),
            service: "test".to_string(),
        })
        .await;

    let state = Arc::new(DashboardState {
        metrics_exporter: Arc::new(MetricsExporter::new()),
        error_manager: Arc::new(ErrorManager::new()),
        alert_manager,
        redis: redis::Client::open("redis://127.0.0.1:1/").unwrap(),
    });

    let app = Router::new()
        .route("/api/dashboard", get(get_dashboard))
        .with_state(state);

    let resp = app
        .oneshot(
            Request::builder()
                .uri("/api/dashboard")
                .body(axum::body::Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();

    let bytes = to_bytes(resp.into_body(), usize::MAX).await.unwrap();
    let json: serde_json::Value = serde_json::from_slice(&bytes).unwrap();

    let alerts = json["active_alerts"].as_array().unwrap();
    assert_eq!(alerts.len(), 1, "one alert should be active");
    assert_eq!(alerts[0]["rule_name"], "test-rule");
    assert_eq!(alerts[0]["severity"], "critical");
}

/// Empty state returns empty arrays for tasks and alerts.
#[tokio::test]
async fn test_dashboard_empty_state() {
    let app = build_app();
    let resp = app
        .oneshot(
            Request::builder()
                .uri("/api/dashboard")
                .body(axum::body::Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();

    let bytes = to_bytes(resp.into_body(), usize::MAX).await.unwrap();
    let json: serde_json::Value = serde_json::from_slice(&bytes).unwrap();

    assert_eq!(json["active_recovery_tasks"].as_array().unwrap().len(), 0);
    assert_eq!(json["active_alerts"].as_array().unwrap().len(), 0);
}

// ---------------------------------------------------------------------------
// Concurrency tests
// ---------------------------------------------------------------------------

/// 10 concurrent requests all return 200.
#[tokio::test]
async fn test_dashboard_10_concurrent() {
    let handles: Vec<_> = (0..10)
        .map(|_| {
            let app = build_app();
            tokio::spawn(async move {
                let resp = app
                    .oneshot(
                        Request::builder()
                            .uri("/api/dashboard")
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
        assert_eq!(handle.await.unwrap(), StatusCode::OK);
    }
}

/// 50 concurrent requests all return 200.
#[tokio::test]
async fn test_dashboard_50_concurrent() {
    let handles: Vec<_> = (0..50)
        .map(|_| {
            let app = build_app();
            tokio::spawn(async move {
                let resp = app
                    .oneshot(
                        Request::builder()
                            .uri("/api/dashboard")
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
        assert_eq!(handle.await.unwrap(), StatusCode::OK);
    }
}

// ---------------------------------------------------------------------------
// Framework-based load tests with SLO assertions
// ---------------------------------------------------------------------------

/// 10 concurrent tasks × 10 requests each = 100 total.
/// SLO: 0% errors, p99 < 500ms.
#[tokio::test]
async fn test_dashboard_load_100_requests_slo() {
    let result = run_framework_load(10, 10).await;
    result.print_summary("GET /api/dashboard — 100 requests");
    assert_load_result(&result, 0.0, std::time::Duration::from_millis(500));
}

/// 20 concurrent tasks × 10 requests each = 200 total.
/// SLO: 0% errors, p99 < 1s.
#[tokio::test]
async fn test_dashboard_load_200_requests_slo() {
    let result = run_framework_load(20, 10).await;
    result.print_summary("GET /api/dashboard — 200 requests");
    assert_load_result(&result, 0.0, std::time::Duration::from_secs(1));
}

/// Verify that all responses under load have the correct JSON shape.
#[tokio::test]
async fn test_dashboard_load_response_shape_under_load() {
    let mut join_set = tokio::task::JoinSet::new();
    for _ in 0..5_usize {
        join_set.spawn(async {
            let mut results = Vec::new();
            for _ in 0..4_usize {
                let app = build_app();
                let resp = app
                    .oneshot(
                        Request::builder()
                            .uri("/api/dashboard")
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

    while let Some(Ok(batch)) = join_set.join_next().await {
        for (status, body) in batch {
            assert_eq!(status, StatusCode::OK);
            let json: serde_json::Value = serde_json::from_slice(&body).unwrap();
            assert!(json.get("metrics").is_some());
            assert!(json.get("active_recovery_tasks").is_some());
            assert!(json.get("active_alerts").is_some());
        }
    }
}

/// Verify that shared state is read consistently under concurrent load.
///
/// All concurrent requests should see the same seeded metric values.
#[tokio::test]
async fn test_dashboard_shared_state_consistency() {
    let metrics_exporter = Arc::new(MetricsExporter::new());
    metrics_exporter.update_metrics(77.0, 4096, 500).await;

    let state = Arc::new(DashboardState {
        metrics_exporter,
        error_manager: Arc::new(ErrorManager::new()),
        alert_manager: Arc::new(AlertManager::new()),
        redis: redis::Client::open("redis://127.0.0.1:1/").unwrap(),
    });

    let mut join_set = tokio::task::JoinSet::new();
    for _ in 0..10_usize {
        let state_clone = state.clone();
        join_set.spawn(async move {
            let app = Router::new()
                .route("/api/dashboard", get(get_dashboard))
                .with_state(state_clone);
            let resp = app
                .oneshot(
                    Request::builder()
                        .uri("/api/dashboard")
                        .body(axum::body::Body::empty())
                        .unwrap(),
                )
                .await
                .unwrap();
            let bytes = to_bytes(resp.into_body(), usize::MAX).await.unwrap();
            serde_json::from_slice::<serde_json::Value>(&bytes).unwrap()
        });
    }

    while let Some(Ok(json)) = join_set.join_next().await {
        assert_eq!(json["metrics"]["cpu_usage"], 77.0);
        assert_eq!(json["metrics"]["memory_usage"], 4096);
        assert_eq!(json["metrics"]["uptime"], 500);
    }
}

/// Verify serialization round-trip of the dashboard response.
#[tokio::test]
async fn test_dashboard_serialization_roundtrip() {
    use backend::api::handlers::dashboard::DashboardData;
    use backend::services::sys_metrics::SystemMetrics;

    let data = DashboardData {
        metrics: SystemMetrics::default(),
        active_recovery_tasks: vec![],
        active_alerts: vec![],
    };

    let json = serde_json::to_string(&data).unwrap();
    let back: DashboardData = serde_json::from_str(&json).unwrap();
    assert_eq!(back.active_recovery_tasks.len(), 0);
    assert_eq!(back.active_alerts.len(), 0);
}
