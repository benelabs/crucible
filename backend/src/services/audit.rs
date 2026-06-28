//! Audit logging service and HTTP routes.

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
use tracing::instrument;
use utoipa::ToSchema;

use crate::error::AppError;

#[derive(Debug, Serialize, Deserialize, Clone, sqlx::FromRow, ToSchema)]
pub struct AuditEventRecord {
    pub id: i64,
    pub event_type: String,
    pub user_id: Option<String>,
    pub details: serde_json::Value,
    pub timestamp: chrono::DateTime<chrono::Utc>,
}

#[derive(Debug, Serialize, Deserialize, Clone, ToSchema)]
pub struct AuditEvent {
    pub event_type: String,
    pub user_id: Option<String>,
    pub details: serde_json::Value,
    pub timestamp: chrono::DateTime<chrono::Utc>,
}

#[derive(Clone)]
pub struct AuditService {
    pub db: PgPool,
    pub redis: Arc<redis::Client>,
}

impl AuditService {
    pub fn new(db: PgPool, redis: Arc<redis::Client>) -> Self {
        Self { db, redis }
    }

    pub async fn log_event(&self, event: AuditEvent) -> Result<(), AppError> {
        sqlx::query(
            "INSERT INTO audit_logs (event_type, user_id, details, timestamp) VALUES ($1, $2, $3, $4)",
        )
        .bind(&event.event_type)
        .bind(&event.user_id)
        .bind(&event.details)
        .bind(event.timestamp)
        .execute(&self.db)
        .await
        .map_err(AppError::Database)?;

        let mut conn = self.redis.get_async_connection().await.map_err(AppError::Redis)?;
        let event_json = serde_json::to_string(&event).map_err(AppError::Serialization)?;
        conn.lpush("audit_queue", event_json).await.map_err(AppError::Redis)?;
        Ok(())
    }

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

#[derive(Debug, Deserialize, ToSchema)]
pub struct AuditEventRequest {
    pub event_type: String,
    pub user_id: Option<String>,
    pub details: serde_json::Value,
}

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
    service
        .log_event(AuditEvent {
            event_type: payload.event_type,
            user_id: payload.user_id,
            details: payload.details,
            timestamp: chrono::Utc::now(),
        })
        .await?;
    Ok(axum::http::StatusCode::CREATED)
}

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
