use axum::{
    body::{to_bytes, Body},
    http::{Request, StatusCode},
    middleware,
    routing::post,
    Router,
};
use backend::{
    api::{
        handlers::{alerts::ingest_alert, profiling::AppState},
        middleware::idempotency::idempotency_middleware,
    },
    config::{reload::ConfigManager, AppConfig},
    services::{
        contract_benchmark::ContractBenchmarkService, error_recovery::ErrorManager,
        log_aggregator::LogAggregator, sys_metrics::MetricsExporter,
    },
};
use std::sync::Arc;
use tower::ServiceExt;

fn idempotency_app() -> Router {
    let (log_aggregator, _receiver) = LogAggregator::new();
    let state = Arc::new(AppState {
        db: None,
        metrics_exporter: Arc::new(MetricsExporter::new()),
        error_manager: Arc::new(ErrorManager::new()),
        config_manager: Arc::new(ConfigManager::new(AppConfig::default())),
        log_aggregator: Arc::new(log_aggregator),
        contract_benchmark_service: Arc::new(ContractBenchmarkService::new()),
        redis: redis::Client::open("redis://127.0.0.1:6379").unwrap(),
    });

    Router::new()
        .route(
            "/api/alerts/ingest",
            post(ingest_alert).route_layer(middleware::from_fn_with_state(
                state.clone(),
                idempotency_middleware,
            )),
        )
        .with_state(state)
}

#[tokio::test]
#[ignore = "requires local Redis server"]
async fn duplicate_ingest_with_idempotency_key_returns_cached_response() {
    let app = idempotency_app();
    let payload = r#"{"source":"integration-test","message":"duplicate-block-check"}"#;

    let make_request = || {
        Request::builder()
            .method("POST")
            .uri("/api/alerts/ingest")
            .header("content-type", "application/json")
            .header("Idempotency-Key", "ingest-integration-1")
            .body(Body::from(payload))
            .unwrap()
    };

    let first = app.clone().oneshot(make_request()).await.unwrap();
    let second = app.oneshot(make_request()).await.unwrap();

    assert_eq!(first.status(), StatusCode::OK);
    assert_eq!(second.status(), StatusCode::OK);
    assert_eq!(
        first.headers().get("x-idempotency-replayed").unwrap(),
        "false"
    );
    assert_eq!(
        second.headers().get("x-idempotency-replayed").unwrap(),
        "true"
    );

    let first_body = to_bytes(first.into_body(), usize::MAX).await.unwrap();
    let second_body = to_bytes(second.into_body(), usize::MAX).await.unwrap();
    assert_eq!(first_body, second_body);
}
