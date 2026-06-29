use axum::{
    routing::{get, post},
    Router,
};
use backend::api::handlers::profiling::AppState;
use backend::config::{
    reload::{handle_get_config, handle_reload, ConfigManager},
    AppConfig,
};
use backend::services::{
    contract_benchmark::ContractBenchmarkService, error_recovery::ErrorManager,
    log_aggregator::LogAggregator, sys_metrics::MetricsExporter,
};
use hyper::{Request, StatusCode};
use redis::Client as RedisClient;
use std::sync::Arc;
use tower::ServiceExt;

fn test_state(config_manager: Arc<ConfigManager>) -> Arc<AppState> {
    let (log_aggregator, _receiver) = LogAggregator::new();

    Arc::new(AppState {
        db: None,
        metrics_exporter: Arc::new(MetricsExporter::new()),
        error_manager: Arc::new(ErrorManager::new()),
        config_manager,
        log_aggregator: Arc::new(log_aggregator),
        contract_benchmark_service: Arc::new(ContractBenchmarkService::new()),
        redis: RedisClient::open("redis://127.0.0.1:1/").unwrap(),
    })
}

#[tokio::test]
async fn test_config_get_endpoint() {
    let config = AppConfig::default();
    let config_manager = Arc::new(ConfigManager::new(config));
    let state = Arc::new(AppState {
        db: None,
        metrics_exporter: Arc::new(MetricsExporter::new()),
        error_manager: Arc::new(ErrorManager::new()),
        config_manager: config_manager.clone(),
        log_aggregator: Arc::new(backend::services::log_aggregator::LogAggregator::new().0),
        redis: redis::Client::open("redis://127.0.0.1/").unwrap(),
    });
    let state = test_state(config_manager.clone());

    let app = Router::new()
        .route("/api/config", get(handle_get_config))
        .with_state(state);

    let response = app
        .oneshot(
            Request::builder()
                .uri("/api/config")
                .body(axum::body::Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();

    assert_eq!(response.status(), StatusCode::OK);
}

#[tokio::test]
async fn test_config_reload_endpoint_no_file() {
    let config = AppConfig::default();
    let config_manager = Arc::new(ConfigManager::new(config));
    let state = Arc::new(AppState {
        db: None,
        metrics_exporter: Arc::new(MetricsExporter::new()),
        error_manager: Arc::new(ErrorManager::new()),
        config_manager: config_manager.clone(),
        log_aggregator: Arc::new(backend::services::log_aggregator::LogAggregator::new().0),
        redis: redis::Client::open("redis://127.0.0.1/").unwrap(),
    });
    let state = test_state(config_manager.clone());

    let app = Router::new()
        .route("/api/config/reload", post(handle_reload))
        .with_state(state);

    let response = app
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/api/config/reload")
                .body(axum::body::Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();

    // Since config.json doesn't exist, it should return an error
    // In our implementation, ConfigReloadError::Io maps to INTERNAL_SERVER_ERROR
    assert_eq!(response.status(), StatusCode::INTERNAL_SERVER_ERROR);
}

#[tokio::test]
async fn test_config_manager_patch() {
    let config = AppConfig::default();
    let config_manager = ConfigManager::new(config);

    let patch = serde_json::json!({
        "log_level": "debug",
        "server": {
            "port": 4000
        }
    });

    config_manager.update_from_patch(patch).unwrap();

    let updated = config_manager.load();
    assert_eq!(updated.log_level, "debug");
    assert_eq!(updated.server.port, 4000);
    // Ensure other fields are preserved
    assert_eq!(updated.server.host, "0.0.0.0");
}


#[tokio::test]
async fn test_sanitized_config_endpoint() {
    use backend::api::handlers::admin::get_effective_config;
    
    let config = AppConfig::default();
    let config_manager = Arc::new(ConfigManager::new(config));

    let response = get_effective_config(axum::extract::State(config_manager.clone()))
        .await
        .unwrap();

    let body = response.into_response();
    let status = body.status();
    
    assert_eq!(status, StatusCode::OK);
}

#[tokio::test]
async fn test_sanitized_config_redacts_secrets() {
    use backend::config::sanitize;
    
    let config = AppConfig::default();
    let sanitized = sanitize(&config);

    // Verify database URL is redacted
    assert_eq!(sanitized.database.url, "[REDACTED]");
    
    // Verify Redis URL is redacted
    assert_eq!(sanitized.redis.url, "[REDACTED]");
    
    // Verify TLS key is redacted if present
    if let Some(tls) = &sanitized.server.tls {
        assert_eq!(tls.key_path, "[REDACTED]");
    }
}

#[tokio::test]
async fn test_sanitized_config_preserves_non_secrets() {
    use backend::config::sanitize;
    
    let config = AppConfig::default();
    let sanitized = sanitize(&config);

    // Verify non-sensitive fields are preserved
    assert_eq!(sanitized.server.host, config.server.host);
    assert_eq!(sanitized.server.port, config.server.port);
    assert_eq!(sanitized.database.max_connections, config.database.max_connections);
    assert_eq!(sanitized.redis.pool_size, config.redis.pool_size);
    assert_eq!(sanitized.cors.allowed_origins, config.cors.allowed_origins);
}

#[tokio::test]
async fn test_sanitized_config_serializes_without_secrets() {
    use backend::config::sanitize;
    
    let config = AppConfig::default();
    let sanitized = sanitize(&config);

    let json = serde_json::to_string(&sanitized).expect("Serialization failed");

    // Verify no actual secrets appear in JSON output
    assert!(!json.contains(&config.database.url));
    assert!(!json.contains(&config.redis.url));
    
    // Verify redaction markers are present
    assert!(json.contains("[REDACTED]"));
}

#[tokio::test]
async fn test_sanitized_config_optional_redis_job_queue_url() {
    use backend::config::sanitize;
    
    let config = AppConfig::default();
    let sanitized = sanitize(&config);

    // If job queue URL exists, it should be redacted
    if sanitized.redis.job_queue_url.is_some() {
        assert_eq!(sanitized.redis.job_queue_url, Some("[REDACTED]".to_string()));
    } else {
        assert_eq!(sanitized.redis.job_queue_url, None);
    }
}

#[tokio::test]
async fn test_sanitized_config_preserves_tls_cert_path() {
    use backend::config::sanitize;
    
    let config = AppConfig::default();
    let sanitized = sanitize(&config);

    if let Some(original_tls) = &config.server.tls {
        if let Some(sanitized_tls) = &sanitized.server.tls {
            // Cert path should be preserved
            assert_eq!(sanitized_tls.cert_path, original_tls.cert_path);
            // Key path should be redacted
            assert_eq!(sanitized_tls.key_path, "[REDACTED]");
        }
    }
}

#[tokio::test]
async fn test_sanitized_config_produces_valid_json() {
    use backend::config::sanitize;
    
    let config = AppConfig::default();
    let sanitized = sanitize(&config);

    let json_str = serde_json::to_string_pretty(&sanitized)
        .expect("Failed to serialize sanitized config");

    // Verify we can parse it back
    let _: serde_json::Value = 
        serde_json::from_str(&json_str).expect("Invalid JSON produced");
}
