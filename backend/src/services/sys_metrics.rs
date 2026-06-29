//! Build System Metrics Exporter
//!
//! This module provides a production-ready metrics exporter for build system operations.
//! It collects and persists build-related metrics including compilation times, dependency counts,
//! cache hit rates, and system resource usage. The service uses PostgreSQL for durability
//! and Redis for high-performance caching.

#![allow(dead_code)]

use chrono::{DateTime, Utc};
use redis::{AsyncCommands, Client as RedisClient};
use rust_decimal::Decimal;
use serde::{Deserialize, Serialize};
use sqlx::PgPool;
use std::sync::Arc;
use thiserror::Error;
use tokio::sync::RwLock;
use tracing::{debug, info, instrument, warn, error};
use uuid::Uuid;
use crate::services::tracing::TracingService;

// ---------------------------------------------------------------------------
// MetricsError
// ---------------------------------------------------------------------------

#[derive(Debug, Error)]
pub enum MetricsError {
    #[error("Database error: {0}")]
    Database(#[from] sqlx::Error),
    #[error("Redis error: {0}")]
    Redis(#[from] redis::RedisError),
    #[error("Serialization error: {0}")]
    Serialization(String),
    #[error("Project not found: {0}")]
    ProjectNotFound(String),
    #[error("Invalid build status: {0}")]
    InvalidStatus(String),
    #[error("Internal error: {0}")]
    Internal(String),
}

// ---------------------------------------------------------------------------
// Domain types
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct SystemMetrics {
    pub cpu_usage: f64,
    pub memory_usage: u64,
    pub uptime: u64,
    pub timestamp: DateTime<Utc>,
}

/// Build status enumeration.
// BuildStatus

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "lowercase")]
pub enum BuildStatus {
    Success,
    Failed,
    Cancelled,
    Running,
}

impl BuildStatus {
    pub fn as_str(&self) -> &'static str {
        match self {
            BuildStatus::Success => "success",
            BuildStatus::Failed => "failed",
            BuildStatus::Cancelled => "cancelled",
            BuildStatus::Running => "running",
        }
    }

    pub fn from_str(s: &str) -> Result<Self, MetricsError> {
        match s.to_lowercase().as_str() {
            "success" => Ok(BuildStatus::Success),
            "failed" => Ok(BuildStatus::Failed),
            "cancelled" => Ok(BuildStatus::Cancelled),
            "running" => Ok(BuildStatus::Running),
            _ => Err(MetricsError::InvalidStatus(s.to_string())),
        }
    }
}

// ---------------------------------------------------------------------------
// BuildMetric
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct BuildMetric {
    pub id: Option<Uuid>,
    pub project_name: String,
    pub build_id: String,
    pub build_status: BuildStatus,
    pub compilation_time_ms: i64,
    pub dependency_count: i32,
    pub cache_hit_rate: Option<Decimal>,
    pub cpu_usage: Option<Decimal>,
    pub memory_usage_mb: Option<i64>,
    pub build_timestamp: DateTime<Utc>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct BuildMetricsSummary {
    pub project_name: String,
    pub total_builds: i64,
    pub successful_builds: i64,
    pub failed_builds: i64,
    pub avg_compilation_time_ms: Decimal,
    pub success_rate: Decimal,
    pub avg_cache_hit_rate: Option<Decimal>,
}

// ---------------------------------------------------------------------------
// BuildMetricsService
// ---------------------------------------------------------------------------

pub struct BuildMetricsService {
    db: PgPool,
    redis: RedisClient,
}

impl BuildMetricsService {
    /// Create a new build metrics service.
    pub fn new(db: PgPool, redis: RedisClient) -> Self {
        Self { db, redis }
    }

    /// Record a build metric.
    pub async fn record_build(&self, metric: BuildMetric) -> Result<Uuid, MetricsError> {
        let id = Uuid::new_v4();
        let status_str = metric.build_status.as_str();

        sqlx::query(
            r#"
            INSERT INTO build_metrics
            (id, project_name, build_id, build_status, compilation_time_ms,
             dependency_count, cache_hit_rate, cpu_usage, memory_usage_mb, build_timestamp)
            VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9, $10)
            "#,
        )
        .bind(id)
        .bind(&metric.project_name)
        .bind(&metric.build_id)
        .bind(status_str)
        .bind(metric.compilation_time_ms)
        .bind(metric.dependency_count)
        .bind(metric.cache_hit_rate.map(|d| d.to_string()))
        .bind(metric.cpu_usage.map(|d| d.to_string()))
        .bind(metric.memory_usage_mb)
        .bind(metric.build_timestamp)
        .execute(&self.db)
        .await?;

        self.invalidate_project_cache(&metric.project_name).await?;

        info!(
            project = %metric.project_name,
            build_id = %metric.build_id,
            status = %status_str,
            "Recorded build metric"
        );

        Ok(id)
    }

        /// Get metrics for a specific project.
    pub async fn get_project_metrics(
        &self,
        project_name: &str,
        limit: i64,
    ) -> Result<Vec<BuildMetric>, MetricsError> {
        let cache_key = format!("build_metrics:{}:{}", project_name, limit);
        let mut conn = self.redis.get_multiplexed_async_connection().await?;
        let cached: Option<String> = conn.get(&cache_key).await?;

        if let Some(val) = cached {
            debug!(project = %project_name, "Build metrics cache hit");
            let metrics: Vec<BuildMetric> = serde_json::from_str(&val)
                .map_err(|e| MetricsError::Serialization(e.to_string()))?;
            return Ok(metrics);
        }

        debug!(project = %project_name, "Build metrics cache miss – querying database");
        let rows: Vec<(
            Uuid,
            String,
            String,
            String,
            i64,
            i32,
            Option<Decimal>,
            Option<Decimal>,
            Option<i64>,
            DateTime<Utc>,
        )> = sqlx::query_as(
            r#"
            SELECT id, project_name, build_id, build_status, compilation_time_ms,
                   dependency_count, cache_hit_rate, cpu_usage, memory_usage_mb, build_timestamp
            FROM build_metrics
            WHERE project_name = 
            ORDER BY build_timestamp DESC
            LIMIT 
            "#,
        )
        .bind(project_name)
        .bind(limit)
        .fetch_all(&self.db)
        .await?;

        let metrics: Vec<BuildMetric> = rows
            .into_iter()
            .map(
                |(
                    id,
                    project_name,
                    build_id,
                    status_str,
                    compilation_time_ms,
                    dependency_count,
                    cache_hit_rate,
                    cpu_usage,
                    memory_usage_mb,
                    build_timestamp,
                )| BuildMetric {
                    id: Some(id),
                    project_name,
                    build_id,
                    build_status: BuildStatus::from_str(&status_str)
                        .unwrap_or(BuildStatus::Failed),
                    compilation_time_ms,
                    dependency_count,
                    cache_hit_rate,
                    cpu_usage,
                    memory_usage_mb,
                    build_timestamp,
                },
            )
            .collect();

        if !metrics.is_empty() {
            let json = serde_json::to_string(&metrics)
                .map_err(|e| MetricsError::Serialization(e.to_string()))?;
            let _: () = conn.set_ex(&cache_key, json, 300).await?;
        }

        Ok(metrics)
    }

    /// Get aggregated metrics summary for a project.
    pub async fn get_project_summary(
        &self,
        project_name: &str,
    ) -> Result<BuildMetricsSummary, MetricsError> {
        let row: Option<(i64, i64, i64, Option<f64>, Option<f64>)> = sqlx::query_as(
            r#"
            SELECT
                COUNT(*) as total_builds,
                SUM(CASE WHEN build_status = 'success' THEN 1 ELSE 0 END)::int8 as successful_builds,
                SUM(CASE WHEN build_status = 'failed' THEN 1 ELSE 0 END)::int8 as failed_builds,
                AVG(compilation_time_ms)::float8 as avg_compilation_time,
                AVG(cache_hit_rate)::float8 as avg_cache_hit_rate
            FROM build_metrics
            WHERE project_name = 
            "#,
        )
        .bind(project_name)
        .fetch_optional(&self.db)
        .await?;

        match row {
            Some((
                total_builds,
                successful_builds,
                failed_builds,
                avg_compilation_time,
                avg_cache_hit_rate,
            )) => {
                let success_rate = if total_builds > 0 {
                    Decimal::from(successful_builds) / Decimal::from(total_builds)
                        * Decimal::from(100u32)
                } else {
                    Decimal::from(0)
                };

                Ok(BuildMetricsSummary {
                    project_name: project_name.to_string(),
                    total_builds,
                    successful_builds,
                    failed_builds,
                    avg_compilation_time_ms: avg_compilation_time
                        .map(Decimal::try_from)
                        .and_then(|r| r.ok())
                        .unwrap_or(Decimal::ZERO),
                    success_rate,
                    avg_cache_hit_rate: avg_cache_hit_rate
                        .map(Decimal::try_from)
                        .and_then(|r| r.ok()),
                })
            }
            None => Err(MetricsError::ProjectNotFound(project_name.to_string())),
        }
    }

    /// Get recent build metrics across all projects.
    pub async fn get_recent_metrics(&self, limit: i64) -> Result<Vec<BuildMetric>, MetricsError> {
        let rows: Vec<(
            Uuid,
            String,
            String,
            String,
            i64,
            i32,
            Option<Decimal>,
            Option<Decimal>,
            Option<i64>,
            DateTime<Utc>,
        )> = sqlx::query_as(
            r#"
            SELECT id, project_name, build_id, build_status, compilation_time_ms,
                   dependency_count, cache_hit_rate, cpu_usage, memory_usage_mb, build_timestamp
            FROM build_metrics
            ORDER BY build_timestamp DESC
            LIMIT $1
            "#,
        )
        .bind(limit)
        .fetch_all(&self.db)
        .await?;

        let metrics = rows
            .into_iter()
            .map(
                |(
                    id,
                    project_name,
                    build_id,
                    status_str,
                    compilation_time_ms,
                    dependency_count,
                    cache_hit_rate,
                    cpu_usage,
                    memory_usage_mb,
                    build_timestamp,
                )| BuildMetric {
                    id: Some(id),
                    project_name,
                    build_id,
                    build_status: BuildStatus::from_str(&status_str)
                        .unwrap_or(BuildStatus::Failed),
                    compilation_time_ms,
                    dependency_count,
                    cache_hit_rate,
                    cpu_usage,
                    memory_usage_mb,
                    build_timestamp,
                },
            )
            .collect()
    }

/// Delete all metrics for a project.
    pub async fn delete_project_metrics(&self, project_name: &str) -> Result<u64, MetricsError> {
        let result = sqlx::query("DELETE FROM build_metrics WHERE project_name = $1")
            .bind(project_name)
            .execute(&self.db)
            .await?;

        self.invalidate_project_cache(project_name).await?;

        info!(
            project = %project_name,
            deleted = result.rows_affected(),
            "Deleted project metrics"
        );

        Ok(result.rows_affected())
    }

    async fn invalidate_project_cache(&self, project_name: &str) -> Result<(), MetricsError> {
        let mut conn = self.redis.get_multiplexed_async_connection().await?;
        
        let pattern = format!("build_metrics:{}:*", project_name);
        let keys: Vec<String> = redis::cmd("KEYS")
            .arg(&pattern)
            .query_async(&mut conn)
            .await?;

        for key in &keys {
            let _: () = conn.del(key).await?;
        }

        if !keys.is_empty() {
            let key_count = keys.len();
            for key in &keys {
                let _: () = conn.del(&key).await?;
            }
            debug!(project = %project_name, count = key_count, "Invalidated project cache");
            debug!(project = %project_name, count = keys.len(), "Invalidated project cache");
        }

        Ok(())
    }
}

// ---------------------------------------------------------------------------
// MetricsExporter
// ---------------------------------------------------------------------------

pub struct MetricsExporter {
    current_metrics: Arc<RwLock<SystemMetrics>>,
}

impl Default for MetricsExporter {
    fn default() -> Self {
        Self::new()
    }
}

impl MetricsExporter {
    pub fn new() -> Self {
        Self {
            current_metrics: Arc::new(RwLock::new(SystemMetrics {
                timestamp: Utc::now(),
                ..Default::default()
            })),
        }
    }

    #[instrument(skip(self), fields(service.name = "MetricsExporter", service.method = "update_metrics"))]
    pub async fn update_metrics(&self, cpu: f64, mem: u64, uptime: u64) {
        let span = TracingService::service_method_span("MetricsExporter", "update_metrics");
        let _enter = span.enter();
        let mut metrics = self.current_metrics.write().await;
        metrics.cpu_usage = cpu;
        metrics.memory_usage = mem;
        metrics.uptime = uptime;
        metrics.timestamp = Utc::now();
        info!(metrics = ?*metrics, "Updated system metrics");
    }

    pub async fn get_metrics(&self) -> SystemMetrics {
        let span = TracingService::service_method_span("MetricsExporter", "get_metrics");
        let _enter = span.enter();
        self.current_metrics.read().await.clone()
    }

    pub async fn run_collector(exporter: Arc<Self>) {
        info!("Starting system metrics collector worker");
        let mut interval = tokio::time::interval(tokio::time::Duration::from_secs(5));
        let start_time = Utc::now();

        loop {
            interval.tick().await;
            let uptime = (Utc::now() - start_time).num_seconds() as u64;
            exporter
                .update_metrics(12.5, 1024 * 1024 * 512, uptime)
                .await;
            exporter.update_metrics(12.5, 1024 * 1024 * 512, uptime).await;
        }
    }
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_build_status_conversion() {
        assert_eq!(BuildStatus::Success.as_str(), "success");
        assert_eq!(BuildStatus::Failed.as_str(), "failed");
        assert_eq!(BuildStatus::Cancelled.as_str(), "cancelled");
        assert_eq!(BuildStatus::Running.as_str(), "running");

        assert_eq!(
            BuildStatus::from_str("success").unwrap(),
            BuildStatus::Success
        );
        assert_eq!(
            BuildStatus::from_str("SUCCESS").unwrap(),
            BuildStatus::Success
        );
        assert!(BuildStatus::from_str("invalid").is_err());
    }

    #[test]
    fn test_build_metric_serialization() {
        let metric = BuildMetric {
            id: Some(Uuid::new_v4()),
            project_name: "test-project".to_string(),
            build_id: "build-123".to_string(),
            build_status: BuildStatus::Success,
            compilation_time_ms: 5000,
            dependency_count: 42,
            cache_hit_rate: Some(Decimal::from(85u32)),
            cpu_usage: Some(Decimal::from(75u32)),
            memory_usage_mb: Some(1024),
            build_timestamp: Utc::now(),
        };

        let json = serde_json::to_string(&metric).unwrap();
        assert!(json.contains("test-project"));
        assert!(json.contains("success"));

        let deserialized: BuildMetric = serde_json::from_str(&json).unwrap();
        assert_eq!(deserialized.project_name, "test-project");
        assert_eq!(deserialized.build_status, BuildStatus::Success);
    }

    #[test]
    fn test_metrics_error_display() {
        let err = MetricsError::ProjectNotFound("test-project".to_string());
        assert!(err.to_string().contains("test-project"));

        let err = MetricsError::InvalidStatus("unknown".to_string());
        assert!(err.to_string().contains("unknown"));
    }

    #[tokio::test]
    async fn test_build_status_roundtrip() {
        let statuses = vec![
            BuildStatus::Success,
            BuildStatus::Failed,
            BuildStatus::Cancelled,
            BuildStatus::Running,
        ];
        for status in statuses {
            let s = status.as_str();
            let parsed = BuildStatus::from_str(s).unwrap();
            assert_eq!(status, parsed);
        }
    }

    #[tokio::test]
    async fn test_metrics_collection() {
        let exporter = MetricsExporter::new();
        exporter.update_metrics(25.0, 1024, 60).await;
        let metrics = exporter.get_metrics().await;
        assert_eq!(metrics.cpu_usage, 25.0);
        assert_eq!(metrics.memory_usage, 1024);
        assert_eq!(metrics.uptime, 60);
    }
}
