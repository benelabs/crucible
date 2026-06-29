use axum::{
    body::Body,
    http::{Request, StatusCode},
    response::IntoResponse,
    routing::get,
    Router,
};
use backend::api::handlers::profiling::{get_system_status, AppState};
use backend::config::{reload::ConfigManager, AppConfig};
use backend::services::{
    contract_benchmark::ContractBenchmarkService, error_recovery::ErrorManager,
    log_aggregator::LogAggregator, sys_metrics::MetricsExporter,
};
use redis::Client as RedisClient;
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
        redis: RedisClient::open("redis://127.0.0.1:1/").unwrap(),
    })
}

#[tokio::test]
async fn test_health_check_integration() {
    // Placeholder — full integration test requires a live DB.
}

#[tokio::test]
async fn test_stellar_toml_headers() {
    use backend::api::handlers::stellar::get_stellar_toml;
    let response = get_stellar_toml().await.into_response();

    assert_eq!(response.status(), StatusCode::OK);
    let cors = response
        .headers()
        .get("access-control-allow-origin")
        .unwrap();
    assert_eq!(cors, "*");
}

#[tokio::test]
async fn test_get_status_endpoint() {
    let state = Arc::new(AppState {
        db: None,
        metrics_exporter: Arc::new(MetricsExporter::new()),
        error_manager: Arc::new(ErrorManager::new()),
        config_manager: Arc::new(backend::config::reload::ConfigManager::new(
            backend::config::AppConfig::default(),
        )),
        log_aggregator: Arc::new(backend::services::log_aggregator::LogAggregator::new().0),
        redis: redis::Client::open("redis://127.0.0.1/").unwrap(),
    });
    let state = test_state();

    let app = Router::new()
        .route("/api/status", get(get_system_status))
        .with_state(state);

    let response = app
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
