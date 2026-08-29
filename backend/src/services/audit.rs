//! CQRS and Event Sourcing Architecture for Audit Logging Service.
//!
//! Audit entries are cryptographically hash-chained using SHA-256 to provide
//! tamper detection.  Each entry stores ``previous_hash`` (the hash of the
//! preceding entry in the chain) and its own ``hash`` computed as::
//!
//!     hash = SHA-256(previous_hash || serialized_entry)
//!
//! Use :meth:`AuditService.verify_chain` to detect unauthorized insertion,
//! deletion, or modification of any entry in the chain.

use axum::{
    extract::{Path, Query, State},
    http::HeaderMap,
    response::IntoResponse,
    routing::{get, post},
    Json, Router,
};
use redis::AsyncCommands;
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use sqlx::PgPool;
use std::collections::HashMap;
use std::sync::Arc;
use std::time::{Duration, Instant};
use tokio::sync::mpsc;
use tracing::{instrument, info, error};
use utoipa::ToSchema;
use uuid::Uuid;

use crate::error::AppError;

// ---------------------------------------------------------------------------
// Domain Events & Event Store (Event Sourcing Write Model)
// ---------------------------------------------------------------------------

/// Immutable domain event representing an audited action.
///
/// Each event carries a ``hash`` and ``previous_hash`` forming a SHA-256 chain
/// that can be verified with :meth:`AuditService.verify_chain`.
#[derive(Debug, Serialize, Deserialize, Clone, ToSchema)]
pub struct AuditDomainEvent {
    pub event_id: Uuid,
    pub aggregate_id: String,
    pub event_type: String,
    pub user_id: Option<String>,
    pub details: serde_json::Value,
    pub sequence_number: u64,
    pub timestamp: chrono::DateTime<chrono::Utc>,
    /// SHA-256 hash of the serialized canonical representation.
    pub hash: String,
    /// Hash of the preceding event in the chain, or empty string for genesis.
    pub previous_hash: String,
}

/// Command payload for recording a new audit event.
#[derive(Debug, Serialize, Deserialize, Clone, ToSchema)]
pub struct AuditEventRequest {
    pub aggregate_id: Option<String>,
    pub event_type: String,
    pub user_id: Option<String>,
    pub details: serde_json::Value,
}

/// The canonical data that gets hashed for chain verification.
/// Only fields that define the audit entry are included — metadata
/// like `event_id` and `sequence_number` is excluded so that
/// re-computation across replicas yields the same hash.
#[derive(Debug, Serialize)]
struct CanonicalAuditData<'a> {
    previous_hash: &'a str,
    event_type: &'a str,
    user_id: Option<&'a str>,
    details: &'a serde_json::Value,
    timestamp: &'a chrono::DateTime<chrono::Utc>,
}

pub fn compute_hash(previous_hash: &str, event: &AuditDomainEvent) -> String {
    let canonical = CanonicalAuditData {
        previous_hash,
        event_type: &event.event_type,
        user_id: event.user_id.as_deref(),
        details: &event.details,
        timestamp: &event.timestamp,
    };
    let json = serde_json::to_string(&canonical).unwrap_or_default();
    let mut hasher = Sha256::new();
    hasher.update(json.as_bytes());
    let result = hasher.finalize();
    result.iter().map(|b| format!("{:02x}", b)).collect()
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
    pub hash: String,
    pub previous_hash: String,
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
    pub hash: String,
    pub previous_hash: String,
}

// ---------------------------------------------------------------------------
// Audit Service with CQRS & Event Sourcing
// ---------------------------------------------------------------------------

#[derive(Clone)]
pub struct AuditService {
    pub db: PgPool,
    pub redis: Arc<redis::Client>,
    event_tx: mpsc::UnboundedSender<AuditDomainEvent>,
    max_export_days: u32,
    export_rate_limiter: Arc<std::sync::Mutex<HashMap<String, (u32, Instant)>>>,
}

impl AuditService {
    pub fn new(db: PgPool, redis: Arc<redis::Client>) -> Self {
        let (tx, mut rx) = mpsc::unbounded_channel::<AuditDomainEvent>();

        let max_export_days = std::env::var("AUDIT_MAX_EXPORT_DAYS")
            .ok()
            .and_then(|v| v.parse().ok())
            .unwrap_or(90);

        let export_rate_limiter = Arc::new(std::sync::Mutex::new(HashMap::new()));

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
            max_export_days,
            export_rate_limiter,
        }
    }

    /// Event Sourcing Write Path: Appends event to event stream without blocking on DB read-table locks.
    ///
    /// Computes the SHA-256 hash chain:
    /// 1. Reads the ``audit:last_hash`` key from Redis (previous hash).
    /// 2. Computes ``hash = SHA-256(previous_hash || canonical_json)``.
    /// 3. Writes the new hash back to ``audit:last_hash``.
    /// 4. Publishes the event (with hash fields) to the event stream.
    pub async fn log_event(&self, req: AuditEventRequest) -> Result<Uuid, AppError> {
        let event_id = Uuid::new_v4();
        let aggregate_id = req.aggregate_id.unwrap_or_else(|| Uuid::new_v4().to_string());
        let now = chrono::Utc::now();

        let mut conn = self.redis.get_multiplexed_async_connection().await.map_err(AppError::Redis)?;

        // Retrieve the previous hash from Redis (empty string for genesis).
        let previous_hash: String = conn
            .get("audit:last_hash")
            .await
            .unwrap_or_default();

        let mut domain_event = AuditDomainEvent {
            event_id,
            aggregate_id,
            event_type: req.event_type,
            user_id: req.user_id,
            details: req.details,
            sequence_number: now.timestamp_millis() as u64,
            timestamp: now,
            hash: String::new(),
            previous_hash: previous_hash.clone(),
        };

        // Compute chain hash = SHA-256(previous_hash || canonical_data)
        domain_event.hash = compute_hash(&previous_hash, &domain_event);

        // Store the new hash as the chain tip for the next event.
        let _: () = conn
            .set("audit:last_hash", &domain_event.hash)
            .await
            .map_err(AppError::Redis)?;

        // Publish to Redis Event Stream
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
            "INSERT INTO audit_logs (event_type, user_id, details, timestamp, hash, previous_hash) VALUES ($1, $2, $3, $4, $5, $6)",
        )
        .bind(&event.event_type)
        .bind(&event.user_id)
        .bind(&event.details)
        .bind(event.timestamp)
        .bind(&event.hash)
        .bind(&event.previous_hash)
        .execute(db)
        .await
        .map_err(AppError::Database)?;

        Ok(())
    }

    /// CQRS Read Path: Queries read-optimized audit projection store.
    pub async fn list_events(
        &self,
        event_type: Option<String>,
        tenant_id: Option<String>,
        start_date: Option<chrono::DateTime<chrono::Utc>>,
        end_date: Option<chrono::DateTime<chrono::Utc>>,
        offset: Option<u32>,
        limit: u32,
    ) -> Result<Vec<AuditEventRecord>, AppError> {
        let mut query_builder = sqlx::QueryBuilder::new(
            "SELECT id, event_type, user_id, details, timestamp, hash, previous_hash FROM audit_logs"
        );

        let mut has_where = false;

        if let Some(ref et) = event_type {
            query_builder.push(" WHERE event_type = ");
            query_builder.push_bind(et);
            has_where = true;
        }

        if let Some(ref t_id) = tenant_id {
            if has_where {
                query_builder.push(" AND ");
            } else {
                query_builder.push(" WHERE ");
                has_where = true;
            }
            query_builder.push("details->>'tenant_id' = ");
            query_builder.push_bind(t_id);
        }

        if let Some(start) = start_date {
            if has_where {
                query_builder.push(" AND ");
            } else {
                query_builder.push(" WHERE ");
                has_where = true;
            }
            query_builder.push("timestamp >= ");
            query_builder.push_bind(start);
        }

        if let Some(end) = end_date {
            if has_where {
                query_builder.push(" AND ");
            } else {
                query_builder.push(" WHERE ");
                has_where = true;
            }
            query_builder.push("timestamp <= ");
            query_builder.push_bind(end);
        }

        query_builder.push(" ORDER BY timestamp DESC");

        let limit = limit.clamp(1, 200) as i64;
        query_builder.push(" LIMIT ");
        query_builder.push_bind(limit);

        if let Some(off) = offset {
            query_builder.push(" OFFSET ");
            query_builder.push_bind(off as i64);
        }

        let query = query_builder.build_query_as::<AuditEventRecord>();
        let rows = query.fetch_all(&self.db).await?;
        Ok(rows)
    }

    /// Verify the integrity of the hash chain from genesis to the latest entry.
    ///
    /// Iterates over all entries in order, recomputes each entry's hash from
    /// its ``previous_hash`` + canonical data, and checks it matches the
    /// stored ``hash``.  Returns a list of tampered entry IDs (empty = chain
    /// is intact).
    pub async fn verify_chain(&self) -> Result<Vec<i64>, AppError> {
        let rows: Vec<AuditEventRecord> = sqlx::query_as(
            "SELECT id, event_type, user_id, details, timestamp, hash, previous_hash \
             FROM audit_logs ORDER BY id ASC",
        )
        .fetch_all(&self.db)
        .await?;

        let mut tampered: Vec<i64> = Vec::new();
        let mut expected_prev = String::new();

        for row in &rows {
            // Recompute what the hash should be.
            let canonical = CanonicalAuditData {
                previous_hash: &expected_prev,
                event_type: &row.event_type,
                user_id: row.user_id.as_deref(),
                details: &row.details,
                timestamp: &row.timestamp,
            };
            let json = serde_json::to_string(&canonical).unwrap_or_default();
            let mut hasher = Sha256::new();
            hasher.update(json.as_bytes());
            let computed: String = hasher.finalize().iter().map(|b| format!("{:02x}", b)).collect();

            // Verify previous_hash link.
            if row.previous_hash != expected_prev || row.hash != computed {
                tampered.push(row.id);
            }

            expected_prev = row.hash.clone();
        }

        Ok(tampered)
    }

    /// CQRS Read Path: Fetches single audit record.
    pub async fn get_event(&self, id: i64) -> Result<AuditEventRecord, AppError> {
        let event = sqlx::query_as(
            "SELECT id, event_type, user_id, details, timestamp, hash, previous_hash FROM audit_logs WHERE id = $1",
        )
        .bind(id)
        .fetch_optional(&self.db)
        .await?;
        event.ok_or_else(|| AppError::NotFound(format!("Audit report {id} not found")))
    }

    /// CQRS Read Path: Export audit events with date filtering.
    pub async fn export_events(
        &self,
        event_type: Option<String>,
        since: chrono::DateTime<chrono::Utc>,
        limit: u32,
    ) -> Result<Vec<AuditEventRecord>, AppError> {
        let limit = limit.clamp(1, 1000) as i64;
        let cols = "id, event_type, user_id, details, timestamp, hash, previous_hash";
        let rows = if let Some(event_type) = event_type {
            sqlx::query_as(&format!("SELECT {cols} FROM audit_logs WHERE event_type = $1 AND timestamp >= $2 ORDER BY timestamp DESC LIMIT $3"))
                .bind(event_type)
                .bind(since)
                .bind(limit)
                .fetch_all(&self.db)
                .await?
        } else {
            sqlx::query_as(&format!("SELECT {cols} FROM audit_logs WHERE timestamp >= $1 ORDER BY timestamp DESC LIMIT $2"))
                .bind(since)
                .bind(limit)
                .fetch_all(&self.db)
                .await?
        };
        Ok(rows)
    }
}

// ---------------------------------------------------------------------------
// HTTP Router & Handlers
// ---------------------------------------------------------------------------

#[derive(Debug, Deserialize)]
pub struct AuditReportQuery {
    pub event_type: Option<String>,
    pub tenant_id: Option<String>,
    pub start_date: Option<chrono::DateTime<chrono::Utc>>,
    pub end_date: Option<chrono::DateTime<chrono::Utc>>,
    pub offset: Option<u32>,
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
    params(
        ("event_type" = Option<String>, Query, description = "Filter by event type"),
        ("tenant_id" = Option<String>, Query, description = "Filter by tenant ID"),
        ("start_date" = Option<DateTime<Utc>>, Query, description = "Start of date range filter"),
        ("end_date" = Option<DateTime<Utc>>, Query, description = "End of date range filter"),
        ("offset" = Option<u32>, Query, description = "Pagination offset"),
        ("limit" = Option<u32>, Query, description = "Pagination limit"),
    ),
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
            .list_events(
                query.event_type,
                query.tenant_id,
                query.start_date,
                query.end_date,
                query.offset,
                query.limit,
            )
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

/// CQRS Read Path: Export audit events in JSON or CSV format.
#[utoipa::path(
    get,
    path = "/api/v1/audit/export",
    params(
        ("event_type" = Option<String>, Query, description = "Filter by event type"),
        ("limit" = Option<u32>, Query, description = "Max number of records to export")
    ),
    responses(
        (status = 200, description = "Audit events exported in requested format"),
        (status = 415, description = "Unsupported media type - use application/json or text/csv"),
        (status = 429, description = "Export rate limit exceeded")
    ),
    tag = "audit"
)]
#[instrument(skip(service))]
pub async fn export_audit_logs(
    State(service): State<Arc<AuditService>>,
    Query(query): Query<AuditReportQuery>,
    headers: HeaderMap,
) -> Result<axum::response::Response, AppError> {
    let accept = headers
        .get(axum::http::header::ACCEPT)
        .and_then(|v| v.to_str().ok())
        .unwrap_or("application/json");

    let client_ip = headers
        .get("x-forwarded-for")
        .and_then(|v| v.to_str().ok())
        .unwrap_or("unknown")
        .split(',')
        .next()
        .unwrap_or("unknown")
        .trim()
        .to_string();

    {
        let mut limiter = service.export_rate_limiter.lock().unwrap();
        let now = Instant::now();
        let (count, last_reset) = limiter.entry(client_ip.clone()).or_insert((0, now));
        if now.duration_since(*last_reset) > Duration::from_secs(3600) {
            *count = 0;
            *last_reset = now;
        }
        if *count >= 5 {
            return Err(AppError::UnsupportedMediaType(
                "Export rate limit exceeded: 5 exports per hour".into(),
            ));
        }
        *count += 1;
    }

    let since = chrono::Utc::now() - chrono::Duration::days(service.max_export_days as i64);
    let records = service.export_events(query.event_type, since, query.limit).await?;

    if accept == "text/csv" {
        let mut wtr = csv::Writer::from_writer(Vec::new());
        wtr.write_record(&[
            "timestamp",
            "actor",
            "action",
            "resource_type",
            "resource_id",
            "ip_address",
            "outcome",
            "metadata",
        ])?;

        for record in &records {
            let details = record.details.as_object();
            let resource_type = details
                .and_then(|o| o.get("resource_type"))
                .and_then(|v| v.as_str())
                .unwrap_or("");
            let resource_id = details
                .and_then(|o| o.get("resource_id"))
                .and_then(|v| v.as_str())
                .unwrap_or("");
            let ip_address = details
                .and_then(|o| o.get("ip_address"))
                .and_then(|v| v.as_str())
                .unwrap_or("");
            let outcome = details
                .and_then(|o| o.get("outcome"))
                .and_then(|v| v.as_str())
                .unwrap_or("");
            let metadata = record.details.to_string();

            let ts = record.timestamp.to_rfc3339();
            wtr.write_record(&[
                ts.as_str(),
                record.user_id.as_deref().unwrap_or(""),
                &record.event_type,
                resource_type,
                resource_id,
                ip_address,
                outcome,
                &metadata,
            ])?;
        }
        wtr.flush()?;
        let data = wtr.into_inner().map_err(|e| AppError::InternalError(e.to_string()))?;
        Ok((
            [(axum::http::header::CONTENT_TYPE, "text/csv; charset=utf-8")],
            data,
        ).into_response())
    } else {
        Ok(Json(records).into_response())
    }
}

pub fn routes(service: Arc<AuditService>) -> Router {
    Router::new()
        .route("/log", post(log_audit_event))
        .route("/reports", get(list_audit_reports))
        .route("/reports/:id", get(get_audit_report))
        .route("/export", get(export_audit_logs))
        .with_state(service)
}
