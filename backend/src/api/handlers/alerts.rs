use axum::{extract::State, Json};
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::sync::Arc;
use utoipa::ToSchema;
use uuid::Uuid;

use crate::api::handlers::profiling::AppState;
use crate::error::AppError;

/// Request body for `POST /api/alerts/ingest`.
#[derive(Debug, Deserialize, ToSchema)]
pub struct AlertIngestRequest {
    pub source: String,
    pub message: String,
    #[serde(default)]
    pub severity: Option<String>,
    #[serde(default)]
    pub metadata: Option<Value>,
}

/// Response body for `POST /api/alerts/ingest`.
#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
pub struct AlertIngestResponse {
    pub alert_id: String,
    pub accepted: bool,
    pub source: String,
    pub message: String,
    pub severity: String,
    pub metadata: Option<Value>,
    pub received_at: DateTime<Utc>,
}

/// Ingests one alert event.
#[utoipa::path(
    post,
    path = "/api/alerts/ingest",
    request_body = AlertIngestRequest,
    params(
        ("Idempotency-Key" = Option<String>, Header, description = "Optional key used to make retries idempotent for 24 hours")
    ),
    responses(
        (status = 200, description = "Alert ingested", body = AlertIngestResponse),
        (status = 400, description = "Invalid payload")
    ),
    tag = "alerts"
)]
pub async fn ingest_alert(
    State(_state): State<Arc<AppState>>,
    Json(payload): Json<AlertIngestRequest>,
) -> Result<Json<AlertIngestResponse>, AppError> {
    if payload.source.trim().is_empty() {
        return Err(AppError::BadRequest("source is required".to_string()));
    }
    if payload.message.trim().is_empty() {
        return Err(AppError::BadRequest("message is required".to_string()));
    }

    let response = AlertIngestResponse {
        alert_id: Uuid::new_v4().to_string(),
        accepted: true,
        source: payload.source,
        message: payload.message,
        severity: payload
            .severity
            .filter(|value| !value.trim().is_empty())
            .unwrap_or_else(|| "info".to_string()),
        metadata: payload.metadata,
        received_at: Utc::now(),
    };

    Ok(Json(response))
}
