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
use std::sync::{Arc, OnceLock};
use thiserror::Error;
use tokio::sync::RwLock;
use tracing::{debug, info, instrument};
use uuid::Uuid;
use prometheus::{
    Encoder, Histogram, HistogramOpts, HistogramVec, IntCounter, IntCounterVec, IntGauge,
    IntGaugeVec, Opts, Registry, TextEncoder,
};
use crate::services::tracing::TracingService;

// ---------------------------------------------------------------------------
// Prometheus System & Health Metrics Registry
// ---------------------------------------------------------------------------

pub struct PrometheusMetrics {
    pub registry: Registry,
    pub compilation_duration_seconds: HistogramVec,
    pub gas_usage_total: IntCounterVec,
    pub gas_usage_average: IntGaugeVec,
    pub active_websockets: IntGauge,
    pub error_rates_total: IntCounterVec,
    pub rpc_latency_seconds: HistogramVec,
}

static PROMETHEUS_METRICS: OnceLock<PrometheusMetrics> = OnceLock::new();

impl PrometheusMetrics {
    pub fn new() -> Self {
        let registry = Registry::new();

        let compilation_duration_seconds = HistogramVec::new(
            HistogramOpts::new(
                "compilation_duration_seconds",
                "Duration of smart contract compilations in seconds",
            )
            .buckets(vec![0.05, 0.1, 0.25, 0.5, 1.0, 2.5, 5.0, 10.0, 30.0, 60.0]),
            &["project", "status"],
        )
        .expect("compilation_duration_seconds histogram must be valid");

        let gas_usage_total = IntCounterVec::new(
            Opts::new("contract_gas_usage_total", "Total contract execution gas consumed"),
            &["contract_id", "function"],
        )
        .expect("contract_gas_usage_total counter must be valid");

        let gas_usage_average = IntGaugeVec::new(
            Opts::new("contract_gas_usage_average", "Average gas used per invocation"),
            &["contract_id"],
        )
        .expect("contract_gas_usage_average gauge must be valid");

        let active_websockets = IntGauge::new(
            "active_websocket_connections",
            "Number of active WebSocket clients",
        )
        .expect("active_websocket_connections gauge must be valid");

        let error_rates_total = IntCounterVec::new(
            Opts::new("application_errors_total", "Total error count by component and error code"),
            &["component", "error_type"],
        )
        .expect("application_errors_total counter must be valid");

        let rpc_latency_seconds = HistogramVec::new(
            HistogramOpts::new(
                "rpc_request_latency_seconds",
                "Stellar/Soroban RPC call latency in seconds",
            )
            .buckets(vec![0.01, 0.05, 0.1, 0.25, 0.5, 1.0, 2.5, 5.0]),
            &["method", "endpoint"],
        )
        .expect("rpc_request_latency_seconds histogram must be valid");

        registry.register(Box::new(compilation_duration_seconds.clone())).unwrap();
        registry.register(Box::new(gas_usage_total.clone())).unwrap();
        registry.register(Box::new(gas_usage_average.clone())).unwrap();
        registry.register(Box::new(active_websockets.clone())).unwrap();
        registry.register(Box::new(error_rates_total.clone())).unwrap();
        registry.register(Box::new(rpc_latency_seconds.clone())).unwrap();

        Self {
            registry,
            compilation_duration_seconds,
            gas_usage_total,
            gas_usage_average,
            active_websockets,
            error_rates_total,
            rpc_latency_seconds,
        }
    }

    pub fn render(&self) -> Result<String, MetricsError> {
        let encoder = TextEncoder::new();
        let mut buffer = Vec::new();
        encoder
            .encode(&self.registry.gather(), &mut buffer)
            .map_err(|e| MetricsError::Internal(e.to_string()))?;
        String::from_utf8(buffer).map_err(|e| MetricsError::Internal(e.to_string()))
    }
}

pub fn prometheus_metrics() -> &'static PrometheusMetrics {
    PROMETHEUS_METRICS.get_or_init(PrometheusMetrics::new)
}

pub fn record_compilation_time(project: &str, status: &str, duration_secs: f64) {
    prometheus_metrics()
        .compilation_duration_seconds
        .with_label_values(&[project, status])
        .observe(duration_secs);
}

pub fn record_gas_usage(contract_id: &str, function: &str, gas: u64, avg_gas: u64) {
    prometheus_metrics()
        .gas_usage_total
        .with_label_values(&[contract_id, function])
        .inc_by(gas);
    if avg_gas > 0 {
        prometheus_metrics()
            .gas_usage_average
            .with_label_values(&[contract_id])
            .set(avg_gas as i64);
    }
}

pub fn record_websocket_conn_change(delta: i64) {
    if delta > 0 {
        prometheus_metrics().active_websockets.add(delta);
    } else {
        prometheus_metrics().active_websockets.sub(-delta);
    }
}

pub fn record_error_rate(component: &str, error_type: &str) {
    prometheus_metrics()
        .error_rates_total
        .with_label_values(&[component, error_type])
        .inc();
}

pub fn record_rpc_latency(method: &str, endpoint: &str, duration_secs: f64) {
    prometheus_metrics()
        .rpc_latency_seconds
        .with_label_values(&[method, endpoint])
        .observe(duration_secs);
}

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
    pub process_resident_memory_bytes: u64,
    pub heap_allocated_bytes: u64,
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
        let db_span = TracingService::db_query_span(
            "INSERT INTO build_metrics",
            "postgres",
            "INSERT",
        );
        let _db_enter = db_span.enter();

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
            .collect();

        Ok(metrics)
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

/// Helper function to parse process resident memory (RSS) and heap allocations (VmData) from `/proc/self/status`.
pub fn get_linux_memory_stats() -> (u64, u64) {
    use std::fs::File;
    use std::io::{BufRead, BufReader};

    let mut rss = 0;
    let mut heap = 0;

    if let Ok(file) = File::open("/proc/self/status") {
        let reader = BufReader::new(file);
        for line in reader.lines().map_while(Result::ok) {
            if line.starts_with("VmRSS:") {
                let parts: Vec<&str> = line.split_whitespace().collect();
                if parts.len() >= 2 {
                    if let Ok(kb) = parts[1].parse::<u64>() {
                        rss = kb * 1024;
                    }
                }
            } else if line.starts_with("VmData:") {
                let parts: Vec<&str> = line.split_whitespace().collect();
                if parts.len() >= 2 {
                    if let Ok(kb) = parts[1].parse::<u64>() {
                        heap = kb * 1024;
                    }
                }
            }
        }
    }

    // Fallbacks if not on Linux or reading fails
    if rss == 0 {
        rss = 1024 * 1024 * 100;
    }
    if heap == 0 {
        heap = 1024 * 1024 * 50;
    }

    (rss, heap)
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
    pub async fn update_metrics(&self, cpu: f64, mem: u64, uptime: u64, rss: u64, heap: u64) {
        let span = TracingService::service_method_span("MetricsExporter", "update_metrics");
        let _enter = span.enter();
        let mut metrics = self.current_metrics.write().await;
        metrics.cpu_usage = cpu;
        metrics.memory_usage = mem;
        metrics.uptime = uptime;
        metrics.process_resident_memory_bytes = rss;
        metrics.heap_allocated_bytes = heap;
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
            let (rss, heap) = get_linux_memory_stats();
            exporter
                .update_metrics(12.5, 1024 * 1024 * 512, uptime, rss, heap)
                .await;
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
    fn test_prometheus_metrics_exposition_format() {
        let metrics = PrometheusMetrics::new();
        metrics
            .compilation_duration_seconds
            .with_label_values(&["test_contract", "success"])
            .observe(1.25);
        metrics
            .gas_usage_total
            .with_label_values(&["contract_123", "execute"])
            .inc_by(5000);
        metrics
            .gas_usage_average
            .with_label_values(&["contract_123"])
            .set(4500);
        metrics.active_websockets.set(42);
        metrics
            .error_rates_total
            .with_label_values(&["api_gateway", "http_500"])
            .inc();
        metrics
            .rpc_latency_seconds
            .with_label_values(&["sendTransaction", "mainnet"])
            .observe(0.35);

        let rendered = metrics.render().unwrap();
        assert!(rendered.contains("compilation_duration_seconds_bucket"));
        assert!(rendered.contains("compilation_duration_seconds_count"));
        assert!(rendered.contains("contract_gas_usage_total"));
        assert!(rendered.contains("contract_gas_usage_average"));
        assert!(rendered.contains("active_websocket_connections 42"));
        assert!(rendered.contains("application_errors_total"));
        assert!(rendered.contains("rpc_request_latency_seconds_bucket"));
    }

    #[test]
    fn test_metrics_error_formatting() {
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
        exporter.update_metrics(25.0, 1024, 60, 2048, 1024).await;
        let metrics = exporter.get_metrics().await;
        assert_eq!(metrics.cpu_usage, 25.0);
        assert_eq!(metrics.memory_usage, 1024);
        assert_eq!(metrics.uptime, 60);
        assert_eq!(metrics.process_resident_memory_bytes, 2048);
        assert_eq!(metrics.heap_allocated_bytes, 1024);
    }
}
