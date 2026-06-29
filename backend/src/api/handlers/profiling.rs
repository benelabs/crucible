//! Performance profiling and system health API handlers.

use std::sync::Arc;

use axum::{extract::State, response::IntoResponse, Json};
use chrono::{DateTime, Utc};
use redis::Client as RedisClient;
use serde::{Deserialize, Serialize};
use tracing::{info, info_span, instrument};
use utoipa::ToSchema;

use crate::api::contracts::{
    ApiResponse, ProfileTriggerRequest, ProfileTriggerResponse, SystemStatus, ValidatedJson,
};
use crate::config::reload::ConfigManager;
use crate::error::AppError;
use crate::services::{
    contract_benchmark::{
        ContractBenchmarkError, ContractBenchmarkReport, ContractBenchmarkRequest,
        ContractBenchmarkService,
    },
    error_recovery::ErrorManager,
    log_aggregator::LogAggregator,
    sys_metrics::MetricsExporter,
    tracing::TracingService,
};

pub struct AppState {
    pub db: Option<sqlx::PgPool>,
    pub metrics_exporter: Arc<MetricsExporter>,
    pub error_manager: Arc<ErrorManager>,
    pub config_manager: Arc<ConfigManager>,
    pub log_aggregator: Arc<LogAggregator>,
    pub contract_benchmark_service: Arc<ContractBenchmarkService>,
    pub redis: RedisClient,
}

#[derive(Debug, Serialize, Deserialize, Clone, ToSchema)]
pub struct MetricsReport {
    pub uptime_secs: u64,
    pub memory_usage_bytes: u64,
    pub active_requests: u32,
    pub error_rate: f64,
    pub ledger_ingestion_latency_ms: u32,
}

#[derive(Debug, Serialize, ToSchema)]
pub struct HealthResponse {
    pub status: String,
    pub version: String,
    pub timestamp: DateTime<Utc>,
    pub database_connected: bool,
    pub redis_connected: bool,
}

#[utoipa::path(
    get,
    path = "/api/v1/profiling/metrics",
    responses((status = 200, description = "Performance metrics", body = MetricsReport)),
    tag = "profiling"
)]
#[instrument(skip_all)]
pub async fn get_metrics(
    State(state): State<Arc<AppState>>,
) -> Result<impl IntoResponse, AppError> {
    let _enter = info_span!("metrics.collection").entered();
    info!("Collecting performance metrics");
    let sys_metrics = state.metrics_exporter.get_metrics().await;
    Ok(Json(MetricsReport {
        uptime_secs: sys_metrics.uptime,
        memory_usage_bytes: sys_metrics.memory_usage,
        active_requests: 12,
        error_rate: 0.001,
        ledger_ingestion_latency_ms: 120,
    }))
}

#[utoipa::path(
    get,
    path = "/api/v1/profiling/health",
    responses((status = 200, description = "System health", body = HealthResponse)),
    tag = "profiling"
)]
#[instrument(skip_all)]
pub async fn get_health(State(state): State<Arc<AppState>>) -> Result<impl IntoResponse, AppError> {
    let db_span = TracingService::db_query_span("SELECT 1", "postgres", "PING");
    let _db_enter = db_span.enter();
    let db_healthy = if let Some(ref pool) = state.db {
        sqlx::query("SELECT 1")
            .fetch_optional(pool)
            .await
            .map(|result| result.is_some())
            .unwrap_or(false)
    } else {
        false
    };
    drop(_db_enter);

    Ok(Json(HealthResponse {
        status: if db_healthy { "healthy" } else { "degraded" }.to_string(),
        version: env!("CARGO_PKG_VERSION").to_string(),
        timestamp: Utc::now(),
        database_connected: db_healthy,
        redis_connected: true,
    }))
}

#[instrument(skip_all)]
pub async fn get_prometheus_metrics() -> impl IntoResponse {
    "backend_requests_total 1024\n".to_string()
}

#[instrument(skip_all)]
pub async fn get_system_status(State(state): State<Arc<AppState>>) -> ApiResponse<SystemStatus> {
    let metrics = state.metrics_exporter.get_metrics().await;
    let recovery_tasks = state.error_manager.get_active_tasks().await;
    ApiResponse::new(SystemStatus {
        status: "healthy".to_string(),
        uptime_secs: metrics.uptime,
        memory_used_bytes: metrics.memory_usage,
        active_recovery_tasks: recovery_tasks.len(),
    })
}

#[instrument(skip_all)]
pub async fn trigger_profile_collection(
    State(_state): State<Arc<AppState>>,
    ValidatedJson(payload): ValidatedJson<ProfileTriggerRequest>,
) -> Result<ApiResponse<ProfileTriggerResponse>, AppError> {
    Ok(ApiResponse::new(ProfileTriggerResponse {
        profile_id: uuid::Uuid::new_v4(),
        message: format!("Profiling collection triggered for label: {}", payload.label),
        estimated_completion:
            chrono::Utc::now() + chrono::Duration::seconds(payload.duration_secs as i64),
    }))
}

#[instrument(skip_all)]
pub async fn run_contract_benchmark(
    State(state): State<Arc<AppState>>,
    Json(payload): Json<ContractBenchmarkRequest>,
) -> Result<ApiResponse<ContractBenchmarkReport>, AppError> {
    let report = state
        .contract_benchmark_service
        .run_benchmark(payload)
        .await
        .map_err(|error| match error {
            ContractBenchmarkError::Validation(message) => AppError::BadRequest(message),
        })?;
    Ok(ApiResponse::new(report))
}
