use axum::{
    routing::{get, post},
    Router,
};
use backend::api::handlers::profiling::{
    get_system_status, run_contract_benchmark, trigger_profile_collection, AppState,
};
use backend::config::reload::ConfigManager;
use backend::config::AppConfig;
use backend::services::{
    contract_benchmark::ContractBenchmarkService, error_recovery::ErrorManager,
    log_aggregator::LogAggregator, sys_metrics::MetricsExporter,
};
use hyper::{Request, StatusCode};
use redis::Client as RedisClient;
use serde_json::json;
use std::sync::Arc;
use tower::ServiceExt;

fn test_state() -> Arc<AppState> {
    let (log_aggregator, _receiver) = LogAggregator::new();

    Arc::new(AppState {
        db: None,
        metrics_exporter: Arc::new(MetricsExporter::new()),
        error_manager: Arc::new(ErrorManager::new()),
        config_manager: Arc::new(ConfigManager::new(AppConfig::default())),
        log_aggregator: Arc::new(log_aggregator),
        contract_benchmark_service: Arc::new(ContractBenchmarkService::new()),
        redis: RedisClient::open("redis://127.0.0.1:6379/").unwrap(),
    })
}

#[tokio::test]
async fn test_system_status_contract() {
    let state = test_state();

    let app = Router::new()
        .route("/api/status", get(get_system_status))
        .with_state(state);

    let response = app
        .oneshot(
            Request::builder()
                .uri("/api/status")
                .body(axum::body::Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();

    assert_eq!(response.status(), StatusCode::OK);

    let body = axum::body::to_bytes(response.into_body(), usize::MAX)
        .await
        .unwrap();
    let json: serde_json::Value = serde_json::from_slice(&body).unwrap();

    assert_eq!(json["status"], "success");
    assert!(json["data"]["status"].is_string());
    assert!(json["data"]["uptime_secs"].is_number());
}

#[tokio::test]
async fn test_profile_trigger_validation_success() {
    let state = Arc::new(AppState {
        db: None,
        metrics_exporter: Arc::new(MetricsExporter::new()),
        error_manager: Arc::new(ErrorManager::new()),
        config_manager: Arc::new(ConfigManager::new(AppConfig::default())),
        log_aggregator: Arc::new(backend::services::log_aggregator::LogAggregator::new().0),
        redis: redis::Client::open("redis://127.0.0.1/").unwrap(),
    });
    let state = test_state();

    let app = Router::new()
        .route("/api/profile", post(trigger_profile_collection))
        .with_state(state);

    let payload = json!({
        "duration_secs": 30,
        "sample_rate_hz": 100,
        "label": "load-test"
    });

    let response = app
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/api/profile")
                .header("content-type", "application/json")
                .body(axum::body::Body::from(
                    serde_json::to_vec(&payload).unwrap(),
                ))
                .unwrap(),
        )
        .await
        .unwrap();

    assert_eq!(response.status(), StatusCode::OK);
}

#[tokio::test]
async fn test_profile_trigger_validation_failure() {
    let state = Arc::new(AppState {
        db: None,
        metrics_exporter: Arc::new(MetricsExporter::new()),
        error_manager: Arc::new(ErrorManager::new()),
        config_manager: Arc::new(ConfigManager::new(AppConfig::default())),
        log_aggregator: Arc::new(backend::services::log_aggregator::LogAggregator::new().0),
        redis: redis::Client::open("redis://127.0.0.1/").unwrap(),
    });
    let state = test_state();

    let app = Router::new()
        .route("/api/profile", post(trigger_profile_collection))
        .with_state(state);

    let payload = json!({
        "duration_secs": 0, // Invalid: must be > 0
        "sample_rate_hz": 100,
        "label": "" // Invalid: cannot be empty
    });

    let response = app
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/api/profile")
                .header("content-type", "application/json")
                .body(axum::body::Body::from(
                    serde_json::to_vec(&payload).unwrap(),
                ))
                .unwrap(),
        )
        .await
        .unwrap();

    assert_eq!(response.status(), StatusCode::BAD_REQUEST);

    let body = axum::body::to_bytes(response.into_body(), usize::MAX)
        .await
        .unwrap();
    let json: serde_json::Value = serde_json::from_slice(&body).unwrap();

    assert_eq!(json["code"], "VALIDATION_ERROR");
    assert!(json["error"]
        .as_str()
        .unwrap()
        .contains("Validation failed"));
}

#[tokio::test]
async fn test_contract_benchmark_endpoint_success() {
    let state = test_state();

    let app = Router::new()
        .route(
            "/api/v1/profiling/contracts/benchmark",
            post(run_contract_benchmark),
        )
        .with_state(state);

    let payload = json!({
        "contract_id": "counter",
        "benchmark_name": "increment_hot_path",
        "samples": [
            {
                "operation": "increment",
                "duration_us": 100,
                "cpu_instructions": 1200,
                "memory_bytes": 4096,
                "ledger_read_entries": 1,
                "ledger_write_entries": 1,
                "ledger_read_bytes": 256,
                "ledger_write_bytes": 128,
                "transaction_size_bytes": 512,
                "events_return_bytes": 64,
                "ledger_space_rent_stroops": 100,
                "resource_fee_stroops": 700,
                "success": true
            },
            {
                "operation": "increment",
                "duration_us": 150,
                "cpu_instructions": 1300,
                "memory_bytes": 4096,
                "ledger_read_entries": 1,
                "ledger_write_entries": 1,
                "ledger_read_bytes": 256,
                "ledger_write_bytes": 128,
                "transaction_size_bytes": 512,
                "events_return_bytes": 64,
                "ledger_space_rent_stroops": 100,
                "resource_fee_stroops": 750,
                "success": true
            }
        ],
        "baseline": {
            "p95_duration_us": 200,
            "avg_cpu_instructions": 1500.0,
            "peak_memory_bytes": 8192,
            "avg_resource_fee_stroops": 1000.0
        }
    });

    let response = app
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/api/v1/profiling/contracts/benchmark")
                .header("content-type", "application/json")
                .body(axum::body::Body::from(
                    serde_json::to_vec(&payload).unwrap(),
                ))
                .unwrap(),
        )
        .await
        .unwrap();

    assert_eq!(response.status(), StatusCode::OK);

    let body = axum::body::to_bytes(response.into_body(), usize::MAX)
        .await
        .unwrap();
    let json: serde_json::Value = serde_json::from_slice(&body).unwrap();

    assert_eq!(json["status"], "success");
    assert_eq!(json["data"]["contract_id"], "counter");
    assert_eq!(json["data"]["status"], "passed");
    assert_eq!(json["data"]["overall"]["p95_duration_us"], 150);
    assert_eq!(json["data"]["overall"]["total_cpu_instructions"], 2500);
    assert_eq!(
        json["data"]["overall"]["total_ledger_space_rent_stroops"],
        200
    );
    assert_eq!(json["data"]["overall"]["total_resource_fee_stroops"], 1450);
}
