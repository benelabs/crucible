use axum::{
    body::Body,
    http::{Request, StatusCode},
    routing::get,
    Router,
};
use backend::api::handlers::dashboard::{
    get_contract_stats, get_dashboard_metrics, DashboardState,
};
use backend::services::{
    error_recovery::ErrorManager, log_alerts::AlertManager, sys_metrics::MetricsExporter,
};
use redis::Client as RedisClient;
use sqlx::postgres::PgPoolOptions;
use std::sync::Arc;
use tower::ServiceExt;

async fn setup_test_db() -> sqlx::PgPool {
    let database_url = std::env::var("DATABASE_URL")
        .unwrap_or_else(|_| "postgres://postgres:postgres@localhost/crucible_test".to_string());

    let pool = PgPoolOptions::new()
        .max_connections(5)
        .connect(&database_url)
        .await
        .expect("Failed to connect to test database");

    // Create test tables
    sqlx::query(
        r#"
        CREATE TABLE IF NOT EXISTS contracts (
            id SERIAL PRIMARY KEY,
            contract_id VARCHAR(255) UNIQUE NOT NULL,
            created_at TIMESTAMPTZ DEFAULT NOW()
        )
        "#,
    )
    .execute(&pool)
    .await
    .ok();

    sqlx::query(
        r#"
        CREATE TABLE IF NOT EXISTS transactions (
            id SERIAL PRIMARY KEY,
            contract_id VARCHAR(255) NOT NULL,
            status VARCHAR(50) NOT NULL,
            processing_time_ms DOUBLE PRECISION,
            gas_cost DOUBLE PRECISION,
            created_at TIMESTAMPTZ DEFAULT NOW()
        )
        "#,
    )
    .execute(&pool)
    .await
    .ok();

    pool
}

async fn setup_test_redis() -> RedisClient {
    let redis_url =
        std::env::var("REDIS_URL").unwrap_or_else(|_| "redis://localhost:6379".to_string());

    RedisClient::open(redis_url).expect("Failed to create Redis client")
}

async fn cleanup_test_data(pool: &sqlx::PgPool) {
    sqlx::query("DELETE FROM transactions")
        .execute(pool)
        .await
        .ok();
    sqlx::query("DELETE FROM contracts")
        .execute(pool)
        .await
        .ok();
}

#[tokio::test]
async fn test_get_dashboard_metrics_empty_database() {
    let pool = setup_test_db().await;
    let redis = setup_test_redis().await;
    cleanup_test_data(&pool).await;

    let state = Arc::new(DashboardState {
        metrics_exporter: Arc::new(MetricsExporter::new()),
        error_manager: Arc::new(ErrorManager::new()),
        alert_manager: Arc::new(AlertManager::new()),
        db: pool.clone(),
        redis_conn: redis.clone(),
        metrics_exporter: Arc::new(backend::services::sys_metrics::MetricsExporter::new()),
        error_manager: Arc::new(backend::services::error_recovery::ErrorManager::new()),
        alert_manager: Arc::new(backend::services::log_alerts::AlertManager::new()),
        redis_client: redis::Client::open(
            std::env::var("REDIS_URL").unwrap_or_else(|_| "redis://localhost:6379".to_string()),
        )
        .unwrap(),
    });

    let app = Router::new()
        .route("/metrics", get(get_dashboard_metrics))
        .with_state(state);

    let response = app
        .oneshot(
            Request::builder()
                .uri("/metrics")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();

    assert_eq!(response.status(), StatusCode::OK);

    let body = axum::body::to_bytes(response.into_body(), usize::MAX)
        .await
        .unwrap();
    let metrics: serde_json::Value = serde_json::from_slice(&body).unwrap();

    assert_eq!(metrics["total_contracts"], 0);
    assert_eq!(metrics["total_transactions"], 0);

    cleanup_test_data(&pool).await;
}

#[tokio::test]
async fn test_get_dashboard_metrics_with_data() {
    let pool = setup_test_db().await;
    let redis = setup_test_redis().await;
    cleanup_test_data(&pool).await;

    // Insert test data
    sqlx::query("INSERT INTO contracts (contract_id) VALUES ('contract_1'), ('contract_2')")
        .execute(&pool)
        .await
        .unwrap();

    sqlx::query(
        "INSERT INTO transactions (contract_id, status, processing_time_ms, gas_cost) 
         VALUES ('contract_1', 'success', 100.0, 1500.0),
                ('contract_1', 'success', 150.0, 1600.0),
                ('contract_2', 'failed', 200.0, 1700.0)",
    )
    .execute(&pool)
    .await
    .unwrap();

    let state = Arc::new(DashboardState {
        metrics_exporter: Arc::new(MetricsExporter::new()),
        error_manager: Arc::new(ErrorManager::new()),
        alert_manager: Arc::new(AlertManager::new()),
        db: pool.clone(),
        redis_conn: redis.clone(),
        metrics_exporter: Arc::new(backend::services::sys_metrics::MetricsExporter::new()),
        error_manager: Arc::new(backend::services::error_recovery::ErrorManager::new()),
        alert_manager: Arc::new(backend::services::log_alerts::AlertManager::new()),
        redis_client: redis::Client::open(
            std::env::var("REDIS_URL").unwrap_or_else(|_| "redis://localhost:6379".to_string()),
        )
        .unwrap(),
    });

    let app = Router::new()
        .route("/metrics", get(get_dashboard_metrics))
        .with_state(state);

    let response = app
        .oneshot(
            Request::builder()
                .uri("/metrics")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();

    assert_eq!(response.status(), StatusCode::OK);

    let body = axum::body::to_bytes(response.into_body(), usize::MAX)
        .await
        .unwrap();
    let metrics: serde_json::Value = serde_json::from_slice(&body).unwrap();

    assert_eq!(metrics["total_contracts"], 2);
    assert_eq!(metrics["total_transactions"], 3);
    assert!(metrics["avg_processing_time_ms"].as_f64().unwrap() > 0.0);

    cleanup_test_data(&pool).await;
}

#[tokio::test]
async fn test_get_contract_stats_not_found() {
    let pool = setup_test_db().await;
    let redis = setup_test_redis().await;
    cleanup_test_data(&pool).await;

    let state = Arc::new(DashboardState {
        metrics_exporter: Arc::new(MetricsExporter::new()),
        error_manager: Arc::new(ErrorManager::new()),
        alert_manager: Arc::new(AlertManager::new()),
        db: pool.clone(),
        redis_conn: redis.clone(),
        metrics_exporter: Arc::new(backend::services::sys_metrics::MetricsExporter::new()),
        error_manager: Arc::new(backend::services::error_recovery::ErrorManager::new()),
        alert_manager: Arc::new(backend::services::log_alerts::AlertManager::new()),
        redis_client: redis::Client::open(
            std::env::var("REDIS_URL").unwrap_or_else(|_| "redis://localhost:6379".to_string()),
        )
        .unwrap(),
    });

    let app = Router::new()
        .route("/contracts/:contract_id/stats", get(get_contract_stats))
        .with_state(state);

    let response = app
        .oneshot(
            Request::builder()
                .uri("/contracts/nonexistent/stats")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();

    assert_eq!(response.status(), StatusCode::NOT_FOUND);

    cleanup_test_data(&pool).await;
}

#[tokio::test]
async fn test_get_contract_stats_success() {
    let pool = setup_test_db().await;
    let redis = setup_test_redis().await;
    cleanup_test_data(&pool).await;

    // Insert test data
    sqlx::query("INSERT INTO contracts (contract_id) VALUES ('test_contract')")
        .execute(&pool)
        .await
        .unwrap();

    sqlx::query(
        "INSERT INTO transactions (contract_id, status, processing_time_ms, gas_cost) 
         VALUES ('test_contract', 'success', 100.0, 1500.0),
                ('test_contract', 'success', 150.0, 1600.0)",
    )
    .execute(&pool)
    .await
    .unwrap();

    let state = Arc::new(DashboardState {
        metrics_exporter: Arc::new(MetricsExporter::new()),
        error_manager: Arc::new(ErrorManager::new()),
        alert_manager: Arc::new(AlertManager::new()),
        db: pool.clone(),
        redis_conn: redis.clone(),
        metrics_exporter: Arc::new(backend::services::sys_metrics::MetricsExporter::new()),
        error_manager: Arc::new(backend::services::error_recovery::ErrorManager::new()),
        alert_manager: Arc::new(backend::services::log_alerts::AlertManager::new()),
        redis_client: redis::Client::open(
            std::env::var("REDIS_URL").unwrap_or_else(|_| "redis://localhost:6379".to_string()),
        )
        .unwrap(),
    });

    let app = Router::new()
        .route("/contracts/:contract_id/stats", get(get_contract_stats))
        .with_state(state);

    let response = app
        .oneshot(
            Request::builder()
                .uri("/contracts/test_contract/stats")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();

    assert_eq!(response.status(), StatusCode::OK);

    let body = axum::body::to_bytes(response.into_body(), usize::MAX)
        .await
        .unwrap();
    let stats: serde_json::Value = serde_json::from_slice(&body).unwrap();

    assert_eq!(stats["contract_id"], "test_contract");
    assert_eq!(stats["invocation_count"], 2);
    assert!(stats["avg_gas_cost"].as_f64().unwrap() > 0.0);

    cleanup_test_data(&pool).await;
}

#[tokio::test]
async fn test_redis_caching() {
    let pool = setup_test_db().await;
    let redis = setup_test_redis().await;
    cleanup_test_data(&pool).await;

    // Clear cache
    let mut conn = redis
        .clone()
        .get_multiplexed_async_connection()
        .await
        .unwrap();
    let _: Result<(), _> = redis::cmd("DEL")
        .arg("dashboard:metrics")
        .query_async(&mut conn)
        .await;

    sqlx::query("INSERT INTO contracts (contract_id) VALUES ('contract_1')")
        .execute(&pool)
        .await
        .unwrap();

    let state = Arc::new(DashboardState {
        metrics_exporter: Arc::new(MetricsExporter::new()),
        error_manager: Arc::new(ErrorManager::new()),
        alert_manager: Arc::new(AlertManager::new()),
        db: pool.clone(),
        redis_conn: redis.clone(),
        metrics_exporter: Arc::new(backend::services::sys_metrics::MetricsExporter::new()),
        error_manager: Arc::new(backend::services::error_recovery::ErrorManager::new()),
        alert_manager: Arc::new(backend::services::log_alerts::AlertManager::new()),
        redis_client: redis::Client::open(
            std::env::var("REDIS_URL").unwrap_or_else(|_| "redis://localhost:6379".to_string()),
        )
        .unwrap(),
    });

    let app = Router::new()
        .route("/metrics", get(get_dashboard_metrics))
        .with_state(state);

    // First request - should hit database
    let response1 = app
        .clone()
        .oneshot(
            Request::builder()
                .uri("/metrics")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();

    assert_eq!(response1.status(), StatusCode::OK);

    // Second request - should hit cache
    let response2 = app
        .oneshot(
            Request::builder()
                .uri("/metrics")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();

    assert_eq!(response2.status(), StatusCode::OK);

    cleanup_test_data(&pool).await;
}
