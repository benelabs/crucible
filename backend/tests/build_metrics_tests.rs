//! Integration tests for Build System Metrics Exporter
//!
//! These tests require a running PostgreSQL instance and Redis instance.
//! They can be run with: cargo test -p backend --test build_metrics_tests

use backend::services::sys_metrics::{BuildMetric, BuildMetricsService, BuildStatus, MetricsError};
use chrono::Utc;
use redis::Client as RedisClient;
use rust_decimal_macros::dec;
use sqlx::{postgres::PgPoolOptions, PgPool};
use uuid::Uuid;

async fn setup_test_db() -> PgPool {
    dotenvy::dotenv().ok();

    let database_url = std::env::var("DATABASE_URL").unwrap_or_else(|_| {
        "postgresql://postgres:postgres@localhost:5432/crucible_test".to_string()
    });

    PgPoolOptions::new()
        .max_connections(5)
        .connect(&database_url)
        .await
        .expect("Failed to connect to database")
}

async fn setup_test_redis() -> RedisClient {
    let redis_url =
        std::env::var("REDIS_URL").unwrap_or_else(|_| "redis://127.0.0.1:6379".to_string());

    RedisClient::open(redis_url).expect("Failed to create Redis client")
}

async fn run_migrations(pool: &PgPool) {
    sqlx::query(
        r#"
        CREATE TABLE IF NOT EXISTS build_metrics (
            id UUID PRIMARY KEY DEFAULT gen_random_uuid(),
            project_name TEXT NOT NULL,
            build_id TEXT NOT NULL,
            build_status TEXT NOT NULL,
            compilation_time_ms BIGINT NOT NULL,
            dependency_count INTEGER NOT NULL,
            cache_hit_rate DECIMAL(5,2),
            cpu_usage DECIMAL(5,2),
            memory_usage_mb BIGINT,
            build_timestamp TIMESTAMP WITH TIME ZONE NOT NULL,
            created_at TIMESTAMP WITH TIME ZONE DEFAULT CURRENT_TIMESTAMP
        )
        "#,
    )
    .execute(pool)
    .await
    .expect("Failed to create build_metrics table");
}

async fn cleanup_test_db(pool: &PgPool) {
    sqlx::query("DROP TABLE IF EXISTS build_metrics")
        .execute(pool)
        .await
        .ok();
}

#[tokio::test]
#[ignore] // Run with: cargo test -p backend --test build_metrics_tests -- --ignored
async fn test_record_and_retrieve_build_metrics() {
    let pool = setup_test_db().await;
    let redis = setup_test_redis().await;
    run_migrations(&pool).await;

    let service = BuildMetricsService::new(pool.clone(), redis);

    // Record a build metric
    let metric = BuildMetric {
        id: None,
        project_name: "test-project".to_string(),
        build_id: "build-001".to_string(),
        build_status: BuildStatus::Success,
        compilation_time_ms: 5000,
        dependency_count: 42,
        cache_hit_rate: Some(dec!(85.5)),
        cpu_usage: Some(dec!(75.2)),
        memory_usage_mb: Some(1024),
        build_timestamp: Utc::now(),
    };

    let id = service.record_build(metric.clone()).await.unwrap();
    assert!(id != Uuid::nil());

    // Retrieve the metric
    let metrics = service
        .get_project_metrics("test-project", 10)
        .await
        .unwrap();
    assert_eq!(metrics.len(), 1);
    assert_eq!(metrics[0].project_name, "test-project");
    assert_eq!(metrics[0].build_id, "build-001");
    assert_eq!(metrics[0].build_status, BuildStatus::Success);

    cleanup_test_db(&pool).await;
}

#[tokio::test]
#[ignore]
async fn test_get_project_summary() {
    let pool = setup_test_db().await;
    let redis = setup_test_redis().await;
    run_migrations(&pool).await;

    let service = BuildMetricsService::new(pool.clone(), redis);

    // Record multiple builds
    for i in 0..10 {
        let metric = BuildMetric {
            id: None,
            project_name: "summary-project".to_string(),
            build_id: format!("build-{:03}", i),
            build_status: if i < 8 {
                BuildStatus::Success
            } else {
                BuildStatus::Failed
            },
            compilation_time_ms: 3000 + (i as i64 * 100),
            dependency_count: 40 + i,
            cache_hit_rate: Some(rust_decimal::Decimal::from(80 + i * 2)),
            cpu_usage: Some(dec!(70.0)),
            memory_usage_mb: Some(512),
            build_timestamp: Utc::now(),
        };
        service.record_build(metric).await.unwrap();
    }

    // Get summary
    let summary = service
        .get_project_summary("summary-project")
        .await
        .unwrap();
    assert_eq!(summary.project_name, "summary-project");
    assert_eq!(summary.total_builds, 10);
    assert_eq!(summary.successful_builds, 8);
    assert_eq!(summary.failed_builds, 2);
    assert!(summary.success_rate >= dec!(70) && summary.success_rate <= dec!(90));
    assert!(summary.avg_cache_hit_rate.is_some());

    cleanup_test_db(&pool).await;
}

#[tokio::test]
#[ignore]
async fn test_redis_caching() {
    let pool = setup_test_db().await;
    let redis = setup_test_redis().await;
    run_migrations(&pool).await;

    let service = BuildMetricsService::new(pool.clone(), redis.clone());

    // Record a metric
    let metric = BuildMetric {
        id: None,
        project_name: "cache-project".to_string(),
        build_id: "build-cache-001".to_string(),
        build_status: BuildStatus::Success,
        compilation_time_ms: 2000,
        dependency_count: 30,
        cache_hit_rate: Some(dec!(90.0)),
        cpu_usage: Some(dec!(60.0)),
        memory_usage_mb: Some(256),
        build_timestamp: Utc::now(),
    };
    service.record_build(metric).await.unwrap();

    // First call - cache miss
    let metrics1 = service
        .get_project_metrics("cache-project", 10)
        .await
        .unwrap();
    assert_eq!(metrics1.len(), 1);

    // Second call - should hit cache
    let metrics2 = service
        .get_project_metrics("cache-project", 10)
        .await
        .unwrap();
    assert_eq!(metrics2.len(), 1);
    assert_eq!(metrics1[0].build_id, metrics2[0].build_id);

    cleanup_test_db(&pool).await;
}

#[tokio::test]
#[ignore]
async fn test_delete_project_metrics() {
    let pool = setup_test_db().await;
    let redis = setup_test_redis().await;
    run_migrations(&pool).await;

    let service = BuildMetricsService::new(pool.clone(), redis);

    // Record metrics
    for i in 0..5 {
        let metric = BuildMetric {
            id: None,
            project_name: "delete-project".to_string(),
            build_id: format!("build-del-{:03}", i),
            build_status: BuildStatus::Success,
            compilation_time_ms: 1000,
            dependency_count: 20,
            cache_hit_rate: Some(dec!(75.0)),
            cpu_usage: Some(dec!(50.0)),
            memory_usage_mb: Some(128),
            build_timestamp: Utc::now(),
        };
        service.record_build(metric).await.unwrap();
    }

    // Verify metrics exist
    let metrics = service
        .get_project_metrics("delete-project", 10)
        .await
        .unwrap();
    assert_eq!(metrics.len(), 5);

    // Delete metrics
    let deleted = service
        .delete_project_metrics("delete-project")
        .await
        .unwrap();
    assert_eq!(deleted, 5);

    // Verify metrics are gone
    let metrics = service
        .get_project_metrics("delete-project", 10)
        .await
        .unwrap();
    assert_eq!(metrics.len(), 0);

    cleanup_test_db(&pool).await;
}

#[tokio::test]
#[ignore]
async fn test_get_recent_metrics() {
    let pool = setup_test_db().await;
    let redis = setup_test_redis().await;
    run_migrations(&pool).await;

    let service = BuildMetricsService::new(pool.clone(), redis);

    // Record metrics for different projects
    let projects = vec!["project-a", "project-b", "project-c"];
    for (i, project) in projects.iter().enumerate() {
        let metric = BuildMetric {
            id: None,
            project_name: project.to_string(),
            build_id: format!("build-{:03}", i),
            build_status: BuildStatus::Success,
            compilation_time_ms: 1500,
            dependency_count: 25,
            cache_hit_rate: Some(dec!(80.0)),
            cpu_usage: Some(dec!(65.0)),
            memory_usage_mb: Some(384),
            build_timestamp: Utc::now(),
        };
        service.record_build(metric).await.unwrap();
    }

    // Get recent metrics
    let recent = service.get_recent_metrics(10).await.unwrap();
    assert_eq!(recent.len(), 3);

    cleanup_test_db(&pool).await;
}

#[tokio::test]
#[ignore]
async fn test_build_status_variations() {
    let pool = setup_test_db().await;
    let redis = setup_test_redis().await;
    run_migrations(&pool).await;

    let service = BuildMetricsService::new(pool.clone(), redis);

    let statuses = vec![
        BuildStatus::Success,
        BuildStatus::Failed,
        BuildStatus::Cancelled,
        BuildStatus::Running,
    ];

    for (i, status) in statuses.iter().enumerate() {
        let metric = BuildMetric {
            id: None,
            project_name: "status-project".to_string(),
            build_id: format!("build-status-{:03}", i),
            build_status: status.clone(),
            compilation_time_ms: 2000,
            dependency_count: 35,
            cache_hit_rate: Some(dec!(85.0)),
            cpu_usage: Some(dec!(70.0)),
            memory_usage_mb: Some(512),
            build_timestamp: Utc::now(),
        };
        service.record_build(metric).await.unwrap();
    }

    // Retrieve and verify all statuses
    let metrics = service
        .get_project_metrics("status-project", 10)
        .await
        .unwrap();
    assert_eq!(metrics.len(), 4);

    let summary = service.get_project_summary("status-project").await.unwrap();
    assert_eq!(summary.total_builds, 4);
    assert_eq!(summary.successful_builds, 1);
    assert_eq!(summary.failed_builds, 1);

    cleanup_test_db(&pool).await;
}

#[tokio::test]
#[ignore]
async fn test_cache_invalidation_on_update() {
    let pool = setup_test_db().await;
    let redis = setup_test_redis().await;
    run_migrations(&pool).await;

    let service = BuildMetricsService::new(pool.clone(), redis.clone());

    // Record initial metric
    let metric = BuildMetric {
        id: None,
        project_name: "invalidation-project".to_string(),
        build_id: "build-inv-001".to_string(),
        build_status: BuildStatus::Success,
        compilation_time_ms: 1000,
        dependency_count: 20,
        cache_hit_rate: Some(dec!(80.0)),
        cpu_usage: Some(dec!(60.0)),
        memory_usage_mb: Some(256),
        build_timestamp: Utc::now(),
    };
    service.record_build(metric).await.unwrap();

    // Populate cache
    let _ = service
        .get_project_metrics("invalidation-project", 10)
        .await
        .unwrap();

    // Add another metric (should invalidate cache)
    let metric2 = BuildMetric {
        id: None,
        project_name: "invalidation-project".to_string(),
        build_id: "build-inv-002".to_string(),
        build_status: BuildStatus::Success,
        compilation_time_ms: 1200,
        dependency_count: 22,
        cache_hit_rate: Some(dec!(82.0)),
        cpu_usage: Some(dec!(62.0)),
        memory_usage_mb: Some(260),
        build_timestamp: Utc::now(),
    };
    service.record_build(metric2).await.unwrap();

    // Should get updated data
    let metrics = service
        .get_project_metrics("invalidation-project", 10)
        .await
        .unwrap();
    assert_eq!(metrics.len(), 2);

    cleanup_test_db(&pool).await;
}
