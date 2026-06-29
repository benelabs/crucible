//! Deploy health API handlers.
//!
//! Provides endpoints for tracking and querying the health of contract deployments.
//!
//! # Endpoints
//!
//! | Method | Path | Description |
//! |--------|------|-------------|
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

// ---------------------------------------------------------------------------
// State
// ---------------------------------------------------------------------------

/// Shared state for deploy-health handlers.
pub struct DeployHealthState {
    pub db: PgPool,
    pub redis: redis::aio::ConnectionManager,
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
// Handlers
// ---------------------------------------------------------------------------

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
    let mut redis = state.redis.clone();

    if let Ok(raw) = redis.get::<_, String>(&cache_key).await {
        if let Ok(cached) = serde_json::from_str::<Deployment>(&raw) {
            return Ok(Json(cached));
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
    .await {
        Ok(Some(v)) => v,
        Ok(None) => return Err(AppError::NotFound(format!("deployment {id} not found"))),
        Err(e) => return Err(AppError::from(e)),
    };

    if let Ok(json) = serde_json::to_string(&deployment) {
        let _: Result<(), _> = redis.set_ex(&cache_key, json, CACHE_TTL_SECS).await;
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
    let mut redis = state.redis.clone();

    if let Ok(raw) = redis.get::<_, String>(&cache_key).await {
        if let Ok(cached) = serde_json::from_str::<Vec<Deployment>>(&raw) {
            return Ok(Json(cached));
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

    if let Ok(json) = serde_json::to_string(&deployments) {
        let _: Result<(), _> = redis.set_ex(&cache_key, json, CACHE_TTL_SECS).await;
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
    .await {
        Ok(Some(v)) => v,
        Ok(None) => return Err(AppError::NotFound(format!("deployment {id} not found"))),
        Err(e) => return Err(AppError::from(e)),
    };

    // Invalidate cache
    let cache_key = format!("deploy_health:deployment:{id}");
    let mut redis = state.redis.clone();
    if let Err(e) = redis.del::<_, ()>(&cache_key).await {
        warn!(error = %e, "Failed to invalidate deployment cache");
    }

    Ok(Json::<Deployment>(deployment))
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
        assert_eq!("pending".parse::<DeployStatus>().unwrap(), DeployStatus::Pending);
        assert_eq!("healthy".parse::<DeployStatus>().unwrap(), DeployStatus::Healthy);
        assert_eq!("degraded".parse::<DeployStatus>().unwrap(), DeployStatus::Degraded);
        assert_eq!("failed".parse::<DeployStatus>().unwrap(), DeployStatus::Failed);
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

    // Helper that mirrors the validation logic in `create_deployment`.
    fn validate_create_request(contract_id: &str, version: &str) -> Result<(), AppError> {
        if contract_id.trim().is_empty() {
            return Err(AppError::BadRequest("contract_id is required".into()));
        }
        if version.trim().is_empty() {
            return Err(AppError::BadRequest("version is required".into()));
        }
        Ok(())
    }
}
