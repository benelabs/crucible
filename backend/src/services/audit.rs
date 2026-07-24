//! CQRS and Event Sourcing Architecture for Audit Logging Service.

use axum::{
    extract::{Path, Query, State},
    response::IntoResponse,
    routing::{get, post},
    Json, Router,
};
use redis::AsyncCommands;
use serde::{Deserialize, Serialize};
use sqlx::PgPool;
use std::sync::Arc;
use tokio::sync::mpsc;
use tracing::{instrument, info, error};
use utoipa::ToSchema;
use uuid::Uuid;

use crate::error::AppError;

// ---------------------------------------------------------------------------
// Domain Events & Event Store (Event Sourcing Write Model)
// ---------------------------------------------------------------------------

/// Immutable domain event representing an audited action.
#[derive(Debug, Serialize, Deserialize, Clone, ToSchema)]
pub struct AuditDomainEvent {
    pub event_id: Uuid,
    pub aggregate_id: String,
    pub event_type: String,
    pub user_id: Option<String>,
    pub details: serde_json::Value,
    pub sequence_number: u64,
    pub timestamp: chrono::DateTime<chrono::Utc>,
}

/// Command payload for recording a new audit event.
#[derive(Debug, Serialize, Deserialize, ToSchema)]
pub struct AuditEventRequest {
    pub aggregate_id: Option<String>,
    pub event_type: String,
    pub user_id: Option<String>,
    pub details: serde_json::Value,
}

// ---------------------------------------------------------------------------
// Read Model / Projections (CQRS Read Model)
// ---------------------------------------------------------------------------

#[derive(Debug, Serialize, Deserialize, Clone, sqlx::FromRow, ToSchema)]
pub struct AuditEventRecord {
    pub id: i64,
    pub event_type: String,
    pub user_id: Option<String>,
    pub details: serde_json::Value,
    pub timestamp: chrono::DateTime<chrono::Utc>,
}

#[derive(Debug, Serialize, Deserialize, Clone, ToSchema)]
pub struct AuditProjection {
    pub event_id: Uuid,
    pub aggregate_id: String,
    pub event_type: String,
    pub user_id: Option<String>,
    pub details: serde_json::Value,
    pub sequence_number: u64,
    pub timestamp: chrono::DateTime<chrono::Utc>,
}

// ---------------------------------------------------------------------------
// Audit Service with CQRS & Event Sourcing
// ---------------------------------------------------------------------------

#[derive(Clone)]
pub struct AuditService {
    pub db: PgPool,
    pub redis: Arc<redis::Client>,
    event_tx: mpsc::UnboundedSender<AuditDomainEvent>,
}

impl AuditService {
    pub fn new(db: PgPool, redis: Arc<redis::Client>) -> Self {
        let (tx, mut rx) = mpsc::unbounded_channel::<AuditDomainEvent>();

        // Spawn background projection engine to process event stream asynchronously
        let db_clone = db.clone();
        let redis_clone = redis.clone();
        tokio::spawn(async move {
            while let Some(event) = rx.recv().await {
                if let Err(err) = Self::project_event(&db_clone, &redis_clone, &event).await {
                    error!(event_id = %event.event_id, "Failed to project audit domain event: {err:?}");
                }
            }
        });

        Self {
            db,
            redis,
            event_tx: tx,
        }
    }

    /// Event Sourcing Write Path: Appends event to event stream without blocking on DB read-table locks.
    pub async fn log_event(&self, req: AuditEventRequest) -> Result<Uuid, AppError> {
        let event_id = Uuid::new_v4();
        let aggregate_id = req.aggregate_id.unwrap_or_else(|| Uuid::new_v4().to_string());

        let domain_event = AuditDomainEvent {
            event_id,
            aggregate_id,
            event_type: req.event_type,
            user_id: req.user_id,
            details: req.details,
            sequence_number: chrono::Utc::now().timestamp_millis() as u64,
            timestamp: chrono::Utc::now(),
        };

        // Publish to Redis Event Stream
        let mut conn = self.redis.get_multiplexed_async_connection().await.map_err(AppError::Redis)?;
        let event_json = serde_json::to_string(&domain_event).map_err(AppError::Serialization)?;
        let _: () = conn
            .lpush::<_, _, ()>("audit_event_stream", event_json)
            .await
            .map_err(AppError::Redis)?;

        // Send to projection pipeline non-blockingly
        let _ = self.event_tx.send(domain_event);

        Ok(event_id)
    }

    /// Asynchronous Projection Engine: Updates read-optimized database views.
    async fn project_event(
        db: &PgPool,
        _redis: &redis::Client,
        event: &AuditDomainEvent,
    ) -> Result<(), AppError> {
        info!(event_id = %event.event_id, event_type = %event.event_type, "Projecting audit event");

        sqlx::query(
            "INSERT INTO audit_logs (event_type, user_id, details, timestamp) VALUES ($1, $2, $3, $4)",
        )
        .bind(&event.event_type)
        .bind(&event.user_id)
        .bind(&event.details)
        .bind(event.timestamp)
        .execute(db)
        .await
        .map_err(AppError::Database)?;

        Ok(())
    }

    /// CQRS Read Path: Queries read-optimized audit projection store.
    pub async fn list_events(
        &self,
        event_type: Option<String>,
        limit: u32,
    ) -> Result<Vec<AuditEventRecord>, AppError> {
        let limit = limit.clamp(1, 200) as i64;
        let rows = if let Some(event_type) = event_type {
            sqlx::query_as(
                "SELECT id, event_type, user_id, details, timestamp FROM audit_logs
                 WHERE event_type = $1 ORDER BY timestamp DESC LIMIT $2",
            )
            .bind(event_type)
            .bind(limit)
            .fetch_all(&self.db)
            .await?
        } else {
            sqlx::query_as(
                "SELECT id, event_type, user_id, details, timestamp FROM audit_logs
                 ORDER BY timestamp DESC LIMIT $1",
            )
            .bind(limit)
            .fetch_all(&self.db)
            .await?
        };
        Ok(rows)
    }

    /// CQRS Read Path: Fetches single audit record.
    pub async fn get_event(&self, id: i64) -> Result<AuditEventRecord, AppError> {
        let event = sqlx::query_as(
            "SELECT id, event_type, user_id, details, timestamp FROM audit_logs WHERE id = $1",
        )
        .bind(id)
        .fetch_optional(&self.db)
        .await?;
        event.ok_or_else(|| AppError::NotFound(format!("Audit report {id} not found")))
    }
}

// ---------------------------------------------------------------------------
// HTTP Router & Handlers
// ---------------------------------------------------------------------------

#[derive(Debug, Deserialize)]
pub struct AuditReportQuery {
    pub event_type: Option<String>,
    #[serde(default = "default_limit")]
    pub limit: u32,
}

fn default_limit() -> u32 {
    50
}

#[instrument(skip(service))]
pub async fn log_audit_event(
    State(service): State<Arc<AuditService>>,
    Json(payload): Json<AuditEventRequest>,
) -> Result<impl IntoResponse, AppError> {
    let event_id = service.log_event(payload).await?;
    Ok((axum::http::StatusCode::CREATED, Json(serde_json::json!({ "event_id": event_id }))))
}

#[utoipa::path(
    get,
    path = "/api/v1/audit/reports",
    responses((status = 200, description = "List audit reports", body = [AuditEventRecord])),
    tag = "audit"
)]
#[instrument(skip(service))]
pub async fn list_audit_reports(
    State(service): State<Arc<AuditService>>,
    Query(query): Query<AuditReportQuery>,
) -> Result<impl IntoResponse, AppError> {
    Ok(Json(
        service
            .list_events(query.event_type, query.limit)
            .await?,
    ))
}

#[utoipa::path(
    get,
    path = "/api/v1/audit/reports/{id}",
    params(
        ("id" = i64, Path, description = "Report ID")
    ),
    responses(
        (status = 200, description = "Audit report details", body = AuditEventRecord),
        (status = 404, description = "Audit report not found")
    ),
    tag = "audit"
)]
#[instrument(skip(service))]
pub async fn get_audit_report(
    State(service): State<Arc<AuditService>>,
    Path(id): Path<i64>,
) -> Result<impl IntoResponse, AppError> {
    Ok(Json(service.get_event(id).await?))
}

pub fn routes(service: Arc<AuditService>) -> Router {
    Router::new()
        .route("/log", post(log_audit_event))
        .route("/reports", get(list_audit_reports))
        .route("/reports/:id", get(get_audit_report))
        .with_state(service)
}
