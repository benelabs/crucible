use axum::{
    body::Body,
    http::{Request, StatusCode},
    routing::{get, post},
    Router,
};
use backend::api::handlers::contracts::{analyze_dependencies, compile_contract, get_networks};
use backend::api::handlers::profiling::AppState;
use backend::config::reload::ConfigManager;
use backend::config::AppConfig;
use backend::services::{
    contract_benchmark::ContractBenchmarkService, error_recovery::ErrorManager,
    log_aggregator::LogAggregator, sys_metrics::MetricsExporter,
};
use redis::Client as RedisClient;
use sqlx::postgres::PgPoolOptions;
use std::sync::Arc;
use tower::ServiceExt;

fn get_mock_state() -> Arc<AppState> {
    let db = PgPoolOptions::new()
        .max_connections(1)
        .connect_lazy("postgres://postgres:postgres@localhost/crucible_test")
        .unwrap();

    let metrics_exporter = Arc::new(MetricsExporter::new());
    let error_manager = Arc::new(ErrorManager::new());
    let (log_aggregator, _) = LogAggregator::new();
    let config = AppConfig::default();
    let config_manager = Arc::new(ConfigManager::new(config));
    let redis = RedisClient::open("redis://127.0.0.1:6379/").unwrap();

    Arc::new(AppState {
        db: Some(db),
        metrics_exporter,
        error_manager,
        config_manager,
        log_aggregator: Arc::new(log_aggregator),
        contract_benchmark_service: Arc::new(ContractBenchmarkService::new()),
        redis,
    })
}

#[tokio::test]
async fn test_compile_endpoint_success() {
    let state = get_mock_state();
    let app = Router::new()
        .route("/api/v1/contracts/compile", post(compile_contract))
        .with_state(state);

    let response = app
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/api/v1/contracts/compile")
                .header("content-type", "application/json")
                .body(Body::from(
                    r#"{"projectName": "test-contract", "sourceCode": "fn test() {}"}"#,
                ))
                .unwrap(),
        )
        .await
        .unwrap();

    assert_eq!(response.status(), StatusCode::OK);
    let bytes = axum::body::to_bytes(response.into_body(), usize::MAX)
        .await
        .unwrap();
    let json: serde_json::Value = serde_json::from_slice(&bytes).unwrap();
    assert_eq!(json["status"], "success");
    assert_eq!(json["data"]["status"], "success");
    assert!(!json["data"]["wasmHash"].as_str().unwrap().is_empty());
}

#[tokio::test]
async fn test_analyze_endpoint_success() {
    let state = get_mock_state();
    let app = Router::new()
        .route(
            "/api/v1/contracts/analyze-dependencies",
            post(analyze_dependencies),
        )
        .with_state(state);

    let response = app
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/api/v1/contracts/analyze-dependencies")
                .header("content-type", "application/json")
                .body(Body::from(
                    r#"{"cargoToml": "[dependencies]\nsoroban-sdk = \"25.0.0\""}"#,
                ))
                .unwrap(),
        )
        .await
        .unwrap();

    assert_eq!(response.status(), StatusCode::OK);
    let bytes = axum::body::to_bytes(response.into_body(), usize::MAX)
        .await
        .unwrap();
    let json: serde_json::Value = serde_json::from_slice(&bytes).unwrap();
    assert_eq!(json["status"], "success");
    assert!(json["data"]["vulnerabilityCount"].as_u64().is_some());
}

#[tokio::test]
async fn test_networks_endpoint_success() {
    let app = Router::new().route("/api/v1/networks", get(get_networks));

    let response = app
        .oneshot(
            Request::builder()
                .method("GET")
                .uri("/api/v1/networks")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();

    assert_eq!(response.status(), StatusCode::OK);
    let bytes = axum::body::to_bytes(response.into_body(), usize::MAX)
        .await
        .unwrap();
    let json: serde_json::Value = serde_json::from_slice(&bytes).unwrap();
    assert_eq!(json["status"], "success");
    assert!(json["data"].is_array());
    assert_eq!(json["data"][0]["id"], "mainnet");
}
