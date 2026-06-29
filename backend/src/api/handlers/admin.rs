use crate::api::contracts::ApiResponse;
use crate::api::handlers::profiling::AppState;
use crate::config::reload::ConfigManager;
use crate::config::sanitize;
use crate::error::AppError;
use crate::services::contract_call_logger::{ContractCallLog, ContractCallLogger};
use axum::{extract::State, response::IntoResponse, Json};
use serde::{Deserialize, Serialize};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;

// Global static for maintenance mode (mock implementation)
pub static MAINTENANCE_MODE: AtomicBool = AtomicBool::new(false);

#[derive(Debug, Clone, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct MaintenanceRequest {
    pub enabled: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SystemStatsResponse {
    pub database_connected: bool,
    pub redis_connected: bool,
    pub total_contract_calls: i64,
    pub system_uptime_secs: u64,
    pub maintenance_mode: bool,
}

/// GET /api/v1/admin/system-stats
pub async fn get_system_stats(
    State(state): State<Arc<AppState>>,
) -> Result<impl IntoResponse, AppError> {
    let db_connected = if let Some(ref pool) = state.db {
        sqlx::query("SELECT 1").fetch_optional(pool).await.is_ok()
    } else {
        false
    };

    let redis_connected = state.redis.get_async_connection().await.is_ok();

    let total_calls = if let Some(ref pool) = state.db {
        sqlx::query_scalar::<_, i64>("SELECT COUNT(*) FROM contract_call_logs")
            .fetch_one(pool)
            .await
            .unwrap_or(0)
    } else {
        0
    };

    let metrics = state.metrics_exporter.get_metrics().await;

    Ok(Json(ApiResponse::new(SystemStatsResponse {
        database_connected: db_connected,
        redis_connected,
        total_contract_calls: total_calls,
        system_uptime_secs: metrics.uptime,
        maintenance_mode: MAINTENANCE_MODE.load(Ordering::Relaxed),
    })))
}

/// POST /api/v1/admin/maintenance
pub async fn set_maintenance_mode(
    Json(payload): Json<MaintenanceRequest>,
) -> Result<impl IntoResponse, AppError> {
    MAINTENANCE_MODE.store(payload.enabled, Ordering::Relaxed);
    Ok(Json(ApiResponse::new(serde_json::json!({
        "success": true,
        "maintenanceMode": payload.enabled
    }))))
}

/// GET /api/v1/admin/logs
pub async fn get_admin_logs(
    State(state): State<Arc<AppState>>,
) -> Result<impl IntoResponse, AppError> {
    let db = state
        .db
        .clone()
        .ok_or_else(|| AppError::InternalError("Database connection not configured".to_string()))?;

    let logger = ContractCallLogger::new(db);
    let logs = logger.get_logs(None, 50).await?;

    Ok(Json(ApiResponse::new(logs)))
}

/// GET /api/v1/admin/config
///
/// Returns the current effective configuration with all sensitive fields redacted.
/// This endpoint is intended for operators to inspect non-secret config for diagnostics.
///
/// **Redacted fields:**
/// - `database.url` → `"[REDACTED]"`
/// - `redis.url` → `"[REDACTED]"`
/// - `redis.job_queue_url` → `"[REDACTED]"`
/// - `server.tls.key_path` → `"[REDACTED]"`
pub async fn get_effective_config(
    State(config_manager): State<Arc<ConfigManager>>,
) -> Result<impl IntoResponse, AppError> {
    let config = config_manager.load();
    let sanitized = sanitize::sanitize(&config);
    Ok(Json(ApiResponse::new(sanitized)))
}
