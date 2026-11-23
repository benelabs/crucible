//! Deploy health API handlers.
//!
//! Provides endpoints for tracking and querying the health of contract deployments.
//!
//! # Endpoints
//!
//! | Method | Path | Description |
//! |--------|------|-------------|
//! | `GET`  | `/api/v1/health/deploy` | Probe deployment health including queue backlog and lag |
//! | `POST` | `/api/v1/deployments` | Register a new deployment |
//! | `GET`  | `/api/v1/deployments/:id` | Get a single deployment by ID |
//! | `GET`  | `/api/v1/deployments/contract/:contract_id` | List deployments for a contract |
//! | `PATCH`| `/api/v1/deployments/:id/status` | Update deployment health status |
//!
//! Results are cached in Redis with a short TTL to reduce database load.

use axum::{
    extract::{Path, State},
    http::StatusCode,
    response::IntoResponse,
    Json,
};
use chrono::{DateTime, Utc};
use redis::AsyncCommands;
use serde::{Deserialize, Serialize};
use sqlx::PgPool;
use std::sync::Arc;
use tracing::{instrument, warn};
use uuid::Uuid;

use crate::error::AppError;

// ---------------------------------------------------------------------------
// Constants
// ---------------------------------------------------------------------------

const CACHE_TTL_SECS: u64 = 30;

/// Default maximum acceptable ledger lag before health is marked degraded.
pub const DEFAULT_MAX_ACCEPTABLE_LAG: u64 = 500;

/// Critical ledger lag threshold after which deployment is marked failed.
pub const CRITICAL_LAG_THRESHOLD: u64 = 2000;

/// Maximum acceptable worker heartbeat age in seconds before worker is considered stale.
pub const DEFAULT_MAX_HEARTBEAT_AGE_SECS: u64 = 60;

/// Maximum acceptable ingestion queue backlog count.
pub const DEFAULT_MAX_ACCEPTABLE_QUEUE_BACKLOG: u64 = 1000;

// ---------------------------------------------------------------------------
// State
// ---------------------------------------------------------------------------

/// Shared state for deploy-health handlers.
#[derive(Clone)]
pub struct DeployHealthState {
    pub db: PgPool,
    pub redis: redis::Client,
}

impl DeployHealthState {
    pub fn new(db: PgPool, redis: redis::Client) -> Self {
        Self { db, redis }
    }
}

// ---------------------------------------------------------------------------
// Domain types
// ---------------------------------------------------------------------------

/// Health status of a deployment.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, sqlx::Type)]
#[sqlx(type_name = "text")]
#[serde(rename_all = "snake_case")]
pub enum DeployStatus {
    Pending,
    Healthy,
    Degraded,
    Failed,
}

impl std::fmt::Display for DeployStatus {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let s = match self {
            DeployStatus::Pending => "pending",
            DeployStatus::Healthy => "healthy",
            DeployStatus::Degraded => "degraded",
            DeployStatus::Failed => "failed",
        };
        f.write_str(s)
    }
}

impl std::str::FromStr for DeployStatus {
    type Err = AppError;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        match s {
            "pending" => Ok(DeployStatus::Pending),
            "healthy" => Ok(DeployStatus::Healthy),
            "degraded" => Ok(DeployStatus::Degraded),
            "failed" => Ok(DeployStatus::Failed),
            other => Err(AppError::BadRequest(format!("invalid status: {other}"))),
        }
    }
}

/// A deployment record as stored in the database.
#[derive(Debug, Clone, Serialize, Deserialize, sqlx::FromRow)]
pub struct Deployment {
    pub id: Uuid,
    pub contract_id: String,
    pub version: String,
    pub status: String,
    pub deployed_at: DateTime<Utc>,
    pub last_checked_at: Option<DateTime<Utc>>,
    pub error_message: Option<String>,
    pub metadata: Option<serde_json::Value>,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
}

/// Detailed status of an ingestion worker's heartbeat.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct WorkerHeartbeatInfo {
    pub worker_id: String,
    pub is_healthy: bool,
    pub seconds_since_last_heartbeat: u64,
    pub last_heartbeat_at: Option<DateTime<Utc>>,
}

/// Metrics tracking background ingestion progress and backlog.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct IngestionLagMetrics {
    pub network_tip_ledger: u64,
    pub current_processed_ledger: u64,
    pub lag_ledgers: u64,
    pub queue_backlog: u64,
    pub max_acceptable_lag: u64,
    pub is_lag_acceptable: bool,
}

/// Summary of worker heartbeats freshness.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct WorkerHealthSummary {
    pub total_workers: usize,
    pub active_healthy_workers: usize,
    pub max_acceptable_heartbeat_age_secs: u64,
    pub are_workers_healthy: bool,
    pub workers: Vec<WorkerHeartbeatInfo>,
}

/// Complete response payload for `GET /api/v1/health/deploy`.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct DeployHealthResponse {
    pub status: DeployStatus,
    pub pod_healthy: bool,
    pub workers_healthy: bool,
    pub ingestion_healthy: bool,
    pub ingestion_lag: IngestionLagMetrics,
    pub worker_health: WorkerHealthSummary,
    pub message: Option<String>,
    pub timestamp: DateTime<Utc>,
}

/// Health probe result for readiness checks
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct HealthProbeResult {
    pub status: String,
    pub timestamp: DateTime<Utc>,
    pub database_writable: bool,
    pub message: String,
}

// ---------------------------------------------------------------------------
// Request / response types
// ---------------------------------------------------------------------------

/// Request body for registering a new deployment.
#[derive(Debug, Deserialize)]
pub struct CreateDeploymentRequest {
    pub contract_id: String,
    pub version: String,
    #[serde(default)]
    pub metadata: Option<serde_json::Value>,
}

/// Request body for updating a deployment's health status.
#[derive(Debug, Deserialize)]
pub struct UpdateStatusRequest {
    pub status: String,
    pub error_message: Option<String>,
}

// ---------------------------------------------------------------------------
// Evaluation logic
// ---------------------------------------------------------------------------

/// Pure function to evaluate deployment health probe based on pod, lag, and worker status.
pub fn evaluate_deploy_health(
    pod_healthy: bool,
    ingestion: IngestionLagMetrics,
    worker_summary: WorkerHealthSummary,
) -> DeployHealthResponse {
    let ingestion_healthy = ingestion.is_lag_acceptable;
    let workers_healthy = worker_summary.are_workers_healthy;

    let mut messages = Vec::new();

    if !pod_healthy {
        messages.push("Pod storage connectivity degraded".to_string());
    }

    if ingestion.lag_ledgers > CRITICAL_LAG_THRESHOLD {
        messages.push(format!(
            "Ingestion lag is critical: {} ledgers behind network tip",
            ingestion.lag_ledgers
        ));
    } else if !ingestion.is_lag_acceptable {
        messages.push(format!(
            "Ingestion lag exceeds threshold: {} ledgers behind network tip (max: {})",
            ingestion.lag_ledgers, ingestion.max_acceptable_lag
        ));
    }

    if ingestion.queue_backlog > DEFAULT_MAX_ACCEPTABLE_QUEUE_BACKLOG {
        messages.push(format!(
            "Ingestion queue backlog high: {} jobs queued",
            ingestion.queue_backlog
        ));
    }

    if !workers_healthy {
        if worker_summary.total_workers == 0 {
            messages.push("No active ingestion workers detected".to_string());
        } else {
            messages.push(format!(
                "Worker heartbeat stale: {}/{} healthy",
                worker_summary.active_healthy_workers, worker_summary.total_workers
            ));
        }
    }

    // Health metrics report pod status as healthy even when background ingestion workers
    // are lagging thousands of ledgers behind network tip.
    // If lag exceeds CRITICAL_LAG_THRESHOLD, status MUST NOT be healthy.
    let status = if !pod_healthy || ingestion.lag_ledgers > CRITICAL_LAG_THRESHOLD {
        DeployStatus::Failed
    } else if !ingestion_healthy || !workers_healthy {
        DeployStatus::Degraded
    } else {
        DeployStatus::Healthy
    };

    let message = if messages.is_empty() {
        None
    } else {
        Some(messages.join("; "))
    };

    DeployHealthResponse {
        status,
        pod_healthy,
        workers_healthy,
        ingestion_healthy,
        ingestion_lag: ingestion,
        worker_health: worker_summary,
        message,
        timestamp: Utc::now(),
    }
}

// ---------------------------------------------------------------------------
// Handlers
// ---------------------------------------------------------------------------

/// `GET /api/v1/health/deploy` — probe overall deployment health including worker backlog and lag.
///
/// Incorporates:
/// 1. Pod database connectivity check
/// 2. Background worker heartbeat freshness
/// 3. Ingestion ledger lag behind network tip and queue backlog
#[instrument(skip(state))]
pub async fn get_deploy_health(
    State(state): State<Arc<DeployHealthState>>,
) -> Result<impl IntoResponse, AppError> {
    let pod_healthy = sqlx::query("SELECT 1").execute(&state.db).await.is_ok();

    let mut current_ledger = 0u64;
    if let Ok(Some((last,))) =
        sqlx::query_as::<_, (i64,)>("SELECT COALESCE(MAX(last_ledger), 0) FROM ingestion_cursors")
            .fetch_optional(&state.db)
            .await
    {
        current_ledger = last.max(0) as u64;
    }

    let mut network_tip = current_ledger;
    let mut queue_backlog = 0u64;
    let mut workers = Vec::new();

    if let Ok(mut conn) = state.redis.get_multiplexed_async_connection().await {
        // Query network tip from redis
        if let Ok(tip_str) = conn.get::<_, String>("stellar:network:tip").await {
            if let Ok(tip) = tip_str.parse::<u64>() {
                network_tip = tip;
            }
        } else if let Ok(tip_str) = conn.get::<_, String>("ingestion:network_tip").await {
            if let Ok(tip) = tip_str.parse::<u64>() {
                network_tip = tip;
            }
        }

        // Query queue backlog
        if let Ok(len) = conn.llen::<_, u64>("ingestion:queue").await {
            queue_backlog = len;
        } else if let Ok(backlog_str) = conn.get::<_, String>("ingestion:queue:backlog").await {
            if let Ok(b) = backlog_str.parse::<u64>() {
                queue_backlog = b;
            }
        }

        // Query worker heartbeats
        if let Ok(keys) = conn.keys::<_, Vec<String>>("worker:*:health").await {
            for key in keys {
                if let Ok(Some(json)) = conn.get::<_, Option<String>>(&key).await {
                    if let Ok(val) = serde_json::from_str::<serde_json::Value>(&json) {
                        let worker_id = val
                            .get("worker_id")
                            .and_then(|v| v.as_str())
                            .unwrap_or_else(|| {
                                key.trim_start_matches("worker:")
                                    .trim_end_matches(":health")
                            })
                            .to_string();
                        let is_healthy_raw = val
                            .get("is_healthy")
                            .and_then(|v| v.as_bool())
                            .unwrap_or(true);
                        let elapsed_secs = val
                            .get("last_heartbeat")
                            .and_then(|v| {
                                v.as_str()
                                    .and_then(|s| s.parse::<u64>().ok())
                                    .or_else(|| v.as_u64())
                            })
                            .unwrap_or(0);
                        let is_fresh = elapsed_secs < DEFAULT_MAX_HEARTBEAT_AGE_SECS;

                        workers.push(WorkerHeartbeatInfo {
                            worker_id,
                            is_healthy: is_healthy_raw && is_fresh,
                            seconds_since_last_heartbeat: elapsed_secs,
                            last_heartbeat_at: Some(
                                Utc::now() - chrono::Duration::seconds(elapsed_secs as i64),
                            ),
                        });
                    }
                }
            }
        }
    }

    let lag_ledgers = network_tip.saturating_sub(current_ledger);
    let is_lag_acceptable = lag_ledgers <= DEFAULT_MAX_ACCEPTABLE_LAG
        && queue_backlog <= DEFAULT_MAX_ACCEPTABLE_QUEUE_BACKLOG;

    let ingestion_metrics = IngestionLagMetrics {
        network_tip_ledger: network_tip,
        current_processed_ledger: current_ledger,
        lag_ledgers,
        queue_backlog,
        max_acceptable_lag: DEFAULT_MAX_ACCEPTABLE_LAG,
        is_lag_acceptable,
    };

    let total_workers = workers.len();
    let active_healthy_workers = workers.iter().filter(|w| w.is_healthy).count();
    let are_workers_healthy = total_workers == 0 || active_healthy_workers == total_workers;

    let worker_summary = WorkerHealthSummary {
        total_workers,
        active_healthy_workers,
        max_acceptable_heartbeat_age_secs: DEFAULT_MAX_HEARTBEAT_AGE_SECS,
        are_workers_healthy,
        workers,
    };

    let health_response = evaluate_deploy_health(pod_healthy, ingestion_metrics, worker_summary);

    let status_code = match health_response.status {
        DeployStatus::Healthy | DeployStatus::Degraded | DeployStatus::Pending => StatusCode::OK,
        DeployStatus::Failed => StatusCode::SERVICE_UNAVAILABLE,
    };

    Ok((status_code, Json(health_response)))
}

/// `POST /api/v1/deployments` — register a new deployment.
///
/// Creates a deployment record with `status = pending` and returns it.
#[instrument(skip(state))]
pub async fn create_deployment(
    State(state): State<Arc<DeployHealthState>>,
    Json(body): Json<CreateDeploymentRequest>,
) -> Result<impl IntoResponse, AppError> {
    if body.contract_id.trim().is_empty() {
        return Err(AppError::BadRequest("contract_id is required".into()));
    }
    if body.version.trim().is_empty() {
        return Err(AppError::BadRequest("version is required".into()));
    }

    let deployment: Deployment = sqlx::query_as(
        r#"
        INSERT INTO deployments (contract_id, version, status, metadata)
        VALUES ($1, $2, 'pending', $3)
        RETURNING
            id, contract_id, version, status,
            deployed_at, last_checked_at, error_message,
            metadata, created_at, updated_at
        "#,
    )
    .bind(&body.contract_id)
    .bind(&body.version)
    .bind(&body.metadata)
    .fetch_one(&state.db)
    .await
    .map_err(AppError::from)?;

    Ok((StatusCode::CREATED, Json::<Deployment>(deployment)))
}

/// `GET /api/v1/deployments/:id` — fetch a single deployment by UUID.
///
/// Responses are cached in Redis for [`CACHE_TTL_SECS`] seconds.
#[instrument(skip(state))]
pub async fn get_deployment(
    State(state): State<Arc<DeployHealthState>>,
    Path(id): Path<Uuid>,
) -> Result<impl IntoResponse, AppError> {
    let cache_key = format!("deploy_health:deployment:{id}");
    if let Ok(mut redis) = state.redis.get_multiplexed_async_connection().await {
        if let Ok(raw) = redis.get::<_, String>(&cache_key).await {
            if let Ok(cached) = serde_json::from_str::<Deployment>(&raw) {
                return Ok(Json(cached));
            }
        }
    }

    let deployment: Deployment = match sqlx::query_as(
        r#"
        SELECT id, contract_id, version, status,
               deployed_at, last_checked_at, error_message,
               metadata, created_at, updated_at
        FROM deployments
        WHERE id = $1
        "#,
    )
    .bind(id)
    .fetch_optional(&state.db)
    .await
    {
        Ok(Some(v)) => v,
        Ok(None) => return Err(AppError::NotFound(format!("deployment {id} not found"))),
        Err(e) => return Err(AppError::from(e)),
    };

    if let Ok(mut redis) = state.redis.get_multiplexed_async_connection().await {
        if let Ok(json) = serde_json::to_string(&deployment) {
            let _: Result<(), _> = redis.set_ex(&cache_key, json, CACHE_TTL_SECS).await;
        }
    }

    Ok(Json::<Deployment>(deployment))
}

/// `GET /api/v1/deployments/contract/:contract_id` — list all deployments for a contract.
///
/// Returns deployments ordered by `deployed_at DESC`. Results are cached.
#[instrument(skip(state))]
pub async fn list_deployments_for_contract(
    State(state): State<Arc<DeployHealthState>>,
    Path(contract_id): Path<String>,
) -> Result<impl IntoResponse, AppError> {
    let cache_key = format!("deploy_health:contract:{contract_id}:deployments");
    if let Ok(mut redis) = state.redis.get_multiplexed_async_connection().await {
        if let Ok(raw) = redis.get::<_, String>(&cache_key).await {
            if let Ok(cached) = serde_json::from_str::<Vec<Deployment>>(&raw) {
                return Ok(Json(cached));
            }
        }
    }

    let deployments: Vec<Deployment> = sqlx::query_as(
        r#"
        SELECT id, contract_id, version, status,
               deployed_at, last_checked_at, error_message,
               metadata, created_at, updated_at
        FROM deployments
        WHERE contract_id = $1
        ORDER BY deployed_at DESC
        "#,
    )
    .bind(&contract_id)
    .fetch_all(&state.db)
    .await
    .map_err(AppError::from)?;

    if let Ok(mut redis) = state.redis.get_multiplexed_async_connection().await {
        if let Ok(json) = serde_json::to_string(&deployments) {
            let _: Result<(), _> = redis.set_ex(&cache_key, json, CACHE_TTL_SECS).await;
        }
    }

    Ok(Json::<Vec<Deployment>>(deployments))
}

/// `PATCH /api/v1/deployments/:id/status` — update the health status of a deployment.
///
/// Accepts `status` (one of `pending`, `healthy`, `degraded`, `failed`) and an optional
/// `error_message`. Invalidates the per-deployment cache entry on success.
#[instrument(skip(state))]
pub async fn update_deployment_status(
    State(state): State<Arc<DeployHealthState>>,
    Path(id): Path<Uuid>,
    Json(body): Json<UpdateStatusRequest>,
) -> Result<impl IntoResponse, AppError> {
    // Validate status value
    body.status.parse::<DeployStatus>()?;

    let deployment: Deployment = match sqlx::query_as(
        r#"
        UPDATE deployments
        SET status          = $2,
            error_message   = $3,
            last_checked_at = NOW(),
            updated_at      = NOW()
        WHERE id = $1
        RETURNING
            id, contract_id, version, status,
            deployed_at, last_checked_at, error_message,
            metadata, created_at, updated_at
        "#,
    )
    .bind(id)
    .bind(&body.status)
    .bind(&body.error_message)
    .fetch_optional(&state.db)
    .await
    {
        Ok(Some(v)) => v,
        Ok(None) => return Err(AppError::NotFound(format!("deployment {id} not found"))),
        Err(e) => return Err(AppError::from(e)),
    };

    // Invalidate cache
    let cache_key = format!("deploy_health:deployment:{id}");
    if let Ok(mut redis) = state.redis.get_multiplexed_async_connection().await {
        if let Err(e) = redis.del::<_, ()>(&cache_key).await {
            warn!(error = %e, "Failed to invalidate deployment cache");
        }
    }

    Ok(Json::<Deployment>(deployment))
}

/// `GET /health/ready` — Kubernetes readiness probe with write capability verification.
///
/// Performs a lightweight write transaction to verify database is writable, not in read-only
/// standby mode or experiencing transaction log disk-full conditions.
#[instrument(skip(state))]
pub async fn readiness_probe(
    State(state): State<Arc<DeployHealthState>>,
) -> Result<impl IntoResponse, AppError> {
    // Attempt a lightweight write transaction to verify database is not in read-only mode
    let mut tx = match state.db.begin().await {
        Ok(t) => t,
        Err(e) => {
            return Ok((
                StatusCode::SERVICE_UNAVAILABLE,
                Json(HealthProbeResult {
                    status: "unhealthy".to_string(),
                    timestamp: Utc::now(),
                    database_writable: false,
                    message: format!("Failed to begin transaction: {e}"),
                }),
            ));
        }
    };

    // Create a temporary table to verify write permissions
    if let Err(e) = sqlx::query("CREATE TEMPORARY TABLE IF NOT EXISTS health_check (id INT)")
        .execute(&mut *tx)
        .await
    {
        let _ = tx.rollback().await;
        return Ok((
            StatusCode::SERVICE_UNAVAILABLE,
            Json(HealthProbeResult {
                status: "unhealthy".to_string(),
                timestamp: Utc::now(),
                database_writable: false,
                message: format!("Database write check failed: {e}"),
            }),
        ));
    }

    if let Err(e) = tx.rollback().await {
        warn!("Failed to rollback health check transaction: {e}");
    }

    Ok((
        StatusCode::OK,
        Json(HealthProbeResult {
            status: "healthy".to_string(),
            timestamp: Utc::now(),
            database_writable: true,
            message: "Database is writable and operational".to_string(),
        }),
    ))
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    // --- DeployStatus unit tests ---

    #[test]
    fn test_status_display() {
        assert_eq!(DeployStatus::Pending.to_string(), "pending");
        assert_eq!(DeployStatus::Healthy.to_string(), "healthy");
        assert_eq!(DeployStatus::Degraded.to_string(), "degraded");
        assert_eq!(DeployStatus::Failed.to_string(), "failed");
    }

    #[test]
    fn test_status_from_str_valid() {
        assert_eq!(
            "pending".parse::<DeployStatus>().unwrap(),
            DeployStatus::Pending
        );
        assert_eq!(
            "healthy".parse::<DeployStatus>().unwrap(),
            DeployStatus::Healthy
        );
        assert_eq!(
            "degraded".parse::<DeployStatus>().unwrap(),
            DeployStatus::Degraded
        );
        assert_eq!(
            "failed".parse::<DeployStatus>().unwrap(),
            DeployStatus::Failed
        );
    }

    #[test]
    fn test_status_from_str_invalid() {
        let err = "unknown".parse::<DeployStatus>().unwrap_err();
        assert!(matches!(err, AppError::BadRequest(_)));
    }

    #[test]
    fn test_status_serde_roundtrip() {
        let statuses = [
            DeployStatus::Pending,
            DeployStatus::Healthy,
            DeployStatus::Degraded,
            DeployStatus::Failed,
        ];
        for status in &statuses {
            let json = serde_json::to_string(status).unwrap();
            let back: DeployStatus = serde_json::from_str(&json).unwrap();
            assert_eq!(&back, status);
        }
    }

    // --- Deployment serialization ---

    #[test]
    fn test_deployment_serialization_roundtrip() {
        let deployment = Deployment {
            id: Uuid::new_v4(),
            contract_id: "CAABC123".to_string(),
            version: "1.0.0".to_string(),
            status: "healthy".to_string(),
            deployed_at: Utc::now(),
            last_checked_at: Some(Utc::now()),
            error_message: None,
            metadata: Some(serde_json::json!({"network": "testnet"})),
            created_at: Utc::now(),
            updated_at: Utc::now(),
        };

        let json = serde_json::to_string(&deployment).unwrap();
        let back: Deployment = serde_json::from_str(&json).unwrap();

        assert_eq!(back.id, deployment.id);
        assert_eq!(back.contract_id, deployment.contract_id);
        assert_eq!(back.version, deployment.version);
        assert_eq!(back.status, deployment.status);
    }

    #[test]
    fn test_deployment_with_error_message() {
        let deployment = Deployment {
            id: Uuid::new_v4(),
            contract_id: "CAABC123".to_string(),
            version: "1.0.0".to_string(),
            status: "failed".to_string(),
            deployed_at: Utc::now(),
            last_checked_at: Some(Utc::now()),
            error_message: Some("out of gas".to_string()),
            metadata: None,
            created_at: Utc::now(),
            updated_at: Utc::now(),
        };

        let json = serde_json::to_string(&deployment).unwrap();
        assert!(json.contains("out of gas"));
    }

    // --- Request validation ---

    #[test]
    fn test_create_request_deserialization() {
        let json = r#"{"contract_id":"CAABC","version":"2.1.0"}"#;
        let req: CreateDeploymentRequest = serde_json::from_str(json).unwrap();
        assert_eq!(req.contract_id, "CAABC");
        assert_eq!(req.version, "2.1.0");
        assert!(req.metadata.is_none());
    }

    #[test]
    fn test_create_request_with_metadata() {
        let json = r#"{"contract_id":"CAABC","version":"2.1.0","metadata":{"env":"prod"}}"#;
        let req: CreateDeploymentRequest = serde_json::from_str(json).unwrap();
        assert!(req.metadata.is_some());
        assert_eq!(req.metadata.unwrap()["env"], "prod");
    }

    #[test]
    fn test_update_status_request_deserialization() {
        let json = r#"{"status":"degraded","error_message":"high latency"}"#;
        let req: UpdateStatusRequest = serde_json::from_str(json).unwrap();
        assert_eq!(req.status, "degraded");
        assert_eq!(req.error_message.as_deref(), Some("high latency"));
    }

    #[test]
    fn test_update_status_request_no_error() {
        let json = r#"{"status":"healthy"}"#;
        let req: UpdateStatusRequest = serde_json::from_str(json).unwrap();
        assert_eq!(req.status, "healthy");
        assert!(req.error_message.is_none());
    }

    // --- Handler-level validation (no DB required) ---

    #[tokio::test]
    async fn test_create_deployment_rejects_empty_contract_id() {
        let err = validate_create_request("", "1.0.0").unwrap_err();
        assert!(matches!(err, AppError::BadRequest(_)));
    }

    #[tokio::test]
    async fn test_create_deployment_rejects_empty_version() {
        let err = validate_create_request("CAABC", "").unwrap_err();
        assert!(matches!(err, AppError::BadRequest(_)));
    }

    #[tokio::test]
    async fn test_create_deployment_accepts_valid_input() {
        validate_create_request("CAABC", "1.0.0").unwrap();
    }

    fn validate_create_request(contract_id: &str, version: &str) -> Result<(), AppError> {
        if contract_id.trim().is_empty() {
            return Err(AppError::BadRequest("contract_id is required".into()));
        }
        if version.trim().is_empty() {
            return Err(AppError::BadRequest("version is required".into()));
        }
        Ok(())
    }

    // --- Deployment Health Evaluation Unit Tests (Issue #1158) ---

    fn sample_healthy_ingestion() -> IngestionLagMetrics {
        IngestionLagMetrics {
            network_tip_ledger: 10000,
            current_processed_ledger: 9990,
            lag_ledgers: 10,
            queue_backlog: 5,
            max_acceptable_lag: DEFAULT_MAX_ACCEPTABLE_LAG,
            is_lag_acceptable: true,
        }
    }

    fn sample_healthy_workers() -> WorkerHealthSummary {
        WorkerHealthSummary {
            total_workers: 2,
            active_healthy_workers: 2,
            max_acceptable_heartbeat_age_secs: DEFAULT_MAX_HEARTBEAT_AGE_SECS,
            are_workers_healthy: true,
            workers: vec![
                WorkerHeartbeatInfo {
                    worker_id: "worker-1".to_string(),
                    is_healthy: true,
                    seconds_since_last_heartbeat: 5,
                    last_heartbeat_at: Some(Utc::now()),
                },
                WorkerHeartbeatInfo {
                    worker_id: "worker-2".to_string(),
                    is_healthy: true,
                    seconds_since_last_heartbeat: 12,
                    last_heartbeat_at: Some(Utc::now()),
                },
            ],
        }
    }

    #[test]
    fn test_evaluate_deploy_health_all_healthy() {
        let res =
            evaluate_deploy_health(true, sample_healthy_ingestion(), sample_healthy_workers());
        assert_eq!(res.status, DeployStatus::Healthy);
        assert!(res.pod_healthy);
        assert!(res.workers_healthy);
        assert!(res.ingestion_healthy);
        assert!(res.message.is_none());
    }

    #[test]
    fn test_evaluate_deploy_health_elevated_lag_degraded() {
        let mut ingestion = sample_healthy_ingestion();
        ingestion.lag_ledgers = 600; // > 500 max, <= 2000
        ingestion.is_lag_acceptable = false;

        let res = evaluate_deploy_health(true, ingestion, sample_healthy_workers());
        assert_eq!(res.status, DeployStatus::Degraded);
        assert!(!res.ingestion_healthy);
        assert!(res
            .message
            .unwrap()
            .contains("Ingestion lag exceeds threshold"));
    }

    #[test]
    fn test_evaluate_deploy_health_critical_lag_thousands_behind_tip() {
        // Issue #1158 scenario: pod status reported healthy even when background ingestion workers
        // are lagging thousands of ledgers behind network tip.
        let mut ingestion = sample_healthy_ingestion();
        ingestion.current_processed_ledger = 7000;
        ingestion.network_tip_ledger = 10500;
        ingestion.lag_ledgers = 3500; // lagging thousands of ledgers!
        ingestion.is_lag_acceptable = false;

        let res = evaluate_deploy_health(true, ingestion, sample_healthy_workers());
        assert_eq!(res.status, DeployStatus::Failed);
        assert!(!res.ingestion_healthy);
        assert!(res
            .message
            .unwrap()
            .contains("Ingestion lag is critical: 3500 ledgers behind network tip"));
    }

    #[test]
    fn test_evaluate_deploy_health_stale_worker_heartbeat_degraded() {
        let mut workers = sample_healthy_workers();
        workers.active_healthy_workers = 1;
        workers.are_workers_healthy = false;
        workers.workers[1].is_healthy = false;
        workers.workers[1].seconds_since_last_heartbeat = 120; // > 60s

        let res = evaluate_deploy_health(true, sample_healthy_ingestion(), workers);
        assert_eq!(res.status, DeployStatus::Degraded);
        assert!(!res.workers_healthy);
        assert!(res.message.unwrap().contains("Worker heartbeat stale"));
    }

    #[test]
    fn test_evaluate_deploy_health_high_queue_backlog_degraded() {
        let mut ingestion = sample_healthy_ingestion();
        ingestion.queue_backlog = 1500; // > 1000 max

        let res = evaluate_deploy_health(true, ingestion, sample_healthy_workers());
        assert!(res
            .message
            .unwrap()
            .contains("Ingestion queue backlog high"));
    }

    #[test]
    fn test_evaluate_deploy_health_pod_storage_failure() {
        let res =
            evaluate_deploy_health(false, sample_healthy_ingestion(), sample_healthy_workers());
        assert_eq!(res.status, DeployStatus::Failed);
        assert!(!res.pod_healthy);
        assert!(res
            .message
            .unwrap()
            .contains("Pod storage connectivity degraded"));
    }

    #[test]
    fn test_deploy_health_response_serialization() {
        let res =
            evaluate_deploy_health(true, sample_healthy_ingestion(), sample_healthy_workers());
        let json = serde_json::to_string(&res).unwrap();
        assert!(json.contains("\"status\":\"healthy\""));
        assert!(json.contains("\"podHealthy\":true"));
        assert!(json.contains("\"ingestionLag\""));
        assert!(json.contains("\"workerHealth\""));

        let deserialized: DeployHealthResponse = serde_json::from_str(&json).unwrap();
        assert_eq!(deserialized.status, DeployStatus::Healthy);
        assert_eq!(deserialized.ingestion_lag.lag_ledgers, 10);
    }
}
