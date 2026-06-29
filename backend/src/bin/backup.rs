//! # Database Backup and Restore Service
//!
//! Standalone binary that exposes HTTP endpoints for triggering PostgreSQL
//! backups, listing existing backups, and restoring from a chosen snapshot.
//! Backup jobs are enqueued via Redis so they can be picked up by a separate
//! worker process if desired, and job status is tracked in a PostgreSQL table.
//!
//! ## Endpoints
//!
//! | Method | Path | Description |
//! |--------|------|-------------|
//! | `POST` | `/backups` | Enqueue a new backup job |
//! | `GET`  | `/backups` | List all backup records |
//! | `GET`  | `/backups/:id` | Get a single backup record |
//! | `POST` | `/backups/:id/restore` | Enqueue a restore job for a backup |
//! | `GET`  | `/health` | Liveness check |
//!
//! ## Configuration (environment variables)
//!
//! | Variable | Default | Description |
//! |----------|---------|-------------|
//! | `DATABASE_URL` | ΓÇö | PostgreSQL connection string (required) |
//! | `REDIS_URL` | `redis://127.0.0.1/` | Redis connection string |
//! | `BACKUP_QUEUE` | `backup_jobs` | Redis list key for backup jobs |
//! | `RESTORE_QUEUE` | `restore_jobs` | Redis list key for restore jobs |
//! | `BIND_ADDR` | `0.0.0.0:8080` | HTTP server bind address |
//! | `BACKUP_DIR` | `/var/backups/crucible` | Directory for `pg_dump` output files |

use axum::{
    extract::{Path, State},
    http::StatusCode,
    response::{IntoResponse, Response},
    routing::{get, post},
    Json, Router,
};
use chrono::{DateTime, Utc};
use redis::AsyncCommands;
use serde::{Deserialize, Serialize};
use sqlx::{postgres::PgPoolOptions, PgPool};
use std::{net::SocketAddr, sync::Arc};
use thiserror::Error;
use tower_http::trace::TraceLayer;
use tracing::{error, info, instrument};
use uuid::Uuid;

// ---------------------------------------------------------------------------
// Error types
// ---------------------------------------------------------------------------

/// Application-level error type.
#[derive(Debug, Error)]
pub enum AppError {
    #[error("database error: {0}")]
    Database(#[from] sqlx::Error),

    #[error("redis error: {0}")]
    Redis(#[from] redis::RedisError),

    #[error("not found")]
    NotFound,

    #[error("internal error: {0}")]
    Internal(String),
}

impl IntoResponse for AppError {
    fn into_response(self) -> Response {
        let (status, message) = match &self {
            AppError::NotFound => (StatusCode::NOT_FOUND, self.to_string()),
            AppError::Database(e) => {
                error!("database error: {e}");
                (
                    StatusCode::INTERNAL_SERVER_ERROR,
                    "database error".to_string(),
                )
            }
            AppError::Redis(e) => {
                error!("redis error: {e}");
                (StatusCode::INTERNAL_SERVER_ERROR, "queue error".to_string())
            }
            AppError::Internal(msg) => {
                error!("internal error: {msg}");
                (
                    StatusCode::INTERNAL_SERVER_ERROR,
                    "internal error".to_string(),
                )
            }
        };

        let body = Json(serde_json::json!({ "error": message }));
        (status, body).into_response()
    }
}

// ---------------------------------------------------------------------------
// Domain types
// ---------------------------------------------------------------------------

/// Status of a backup or restore job.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, sqlx::Type)]
#[sqlx(type_name = "text")]
#[serde(rename_all = "snake_case")]
pub enum JobStatus {
    Pending,
    Running,
    Completed,
    Failed,
}

impl std::fmt::Display for JobStatus {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let s = match self {
            JobStatus::Pending => "pending",
            JobStatus::Running => "running",
            JobStatus::Completed => "completed",
            JobStatus::Failed => "failed",
        };
        write!(f, "{s}")
    }
}

/// A backup record stored in PostgreSQL.
#[derive(Debug, Clone, Serialize, Deserialize, sqlx::FromRow)]
pub struct BackupRecord {
    pub id: Uuid,
    pub status: String,
    pub file_path: Option<String>,
    pub size_bytes: Option<i64>,
    pub error_message: Option<String>,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
}

/// Request body for creating a backup.
#[derive(Debug, Deserialize)]
pub struct CreateBackupRequest {
    /// Optional label / note for this backup.
    pub label: Option<String>,
}

/// Response body for a newly enqueued job.
#[derive(Debug, Serialize)]
pub struct JobEnqueued {
    pub id: Uuid,
    pub status: JobStatus,
    pub message: String,
}

// ---------------------------------------------------------------------------
// Shared application state
// ---------------------------------------------------------------------------

/// Configuration extracted from environment variables.
#[derive(Debug, Clone)]
pub struct Config {
    pub database_url: String,
    pub redis_url: String,
    pub backup_queue: String,
    pub restore_queue: String,
    pub bind_addr: SocketAddr,
    pub backup_dir: String,
}

impl Config {
    /// Load configuration from the process environment.
    ///
    /// # Panics
    /// Panics if `DATABASE_URL` is not set.
    pub fn from_env() -> Self {
        let database_url = std::env::var("DATABASE_URL").expect("DATABASE_URL must be set");
        let redis_url =
            std::env::var("REDIS_URL").unwrap_or_else(|_| "redis://127.0.0.1/".to_string());
        let backup_queue =
            std::env::var("BACKUP_QUEUE").unwrap_or_else(|_| "backup_jobs".to_string());
        let restore_queue =
            std::env::var("RESTORE_QUEUE").unwrap_or_else(|_| "restore_jobs".to_string());
        let bind_addr: SocketAddr = std::env::var("BIND_ADDR")
            .unwrap_or_else(|_| "0.0.0.0:8080".to_string())
            .parse()
            .expect("BIND_ADDR must be a valid socket address");
        let backup_dir =
            std::env::var("BACKUP_DIR").unwrap_or_else(|_| "/var/backups/crucible".to_string());

        Self {
            database_url,
            redis_url,
            backup_queue,
            restore_queue,
            bind_addr,
            backup_dir,
        }
    }
}

/// Shared state injected into every Axum handler.
#[derive(Clone)]
pub struct AppState {
    pub db: PgPool,
    pub redis: redis::Client,
    pub config: Arc<Config>,
}

// ---------------------------------------------------------------------------
// Database helpers
// ---------------------------------------------------------------------------

/// Ensure the `backups` table exists.
pub async fn run_migrations(pool: &PgPool) -> Result<(), sqlx::Error> {
    sqlx::query(
        r#"
        CREATE TABLE IF NOT EXISTS backups (
            id            UUID PRIMARY KEY DEFAULT gen_random_uuid(),
            status        TEXT NOT NULL DEFAULT 'pending',
            file_path     TEXT,
            size_bytes    BIGINT,
            error_message TEXT,
            created_at    TIMESTAMPTZ NOT NULL DEFAULT now(),
            updated_at    TIMESTAMPTZ NOT NULL DEFAULT now()
        )
        "#,
    )
    .execute(pool)
    .await?;

    Ok(())
}

/// Insert a new backup record in `pending` status and return it.
pub async fn create_backup_row(pool: &PgPool) -> Result<BackupRecord, sqlx::Error> {
    sqlx::query_as::<_, BackupRecord>(
        r#"
        INSERT INTO backups (id, status, created_at, updated_at)
        VALUES (gen_random_uuid(), 'pending', now(), now())
        RETURNING *
        "#,
    )
    .fetch_one(pool)
    .await
}

/// Fetch all backup records, newest first.
pub async fn list_backup_rows(pool: &PgPool) -> Result<Vec<BackupRecord>, sqlx::Error> {
    sqlx::query_as::<_, BackupRecord>("SELECT * FROM backups ORDER BY created_at DESC")
        .fetch_all(pool)
        .await
}

/// Fetch a single backup record by ID.
pub async fn get_backup_row(pool: &PgPool, id: Uuid) -> Result<Option<BackupRecord>, sqlx::Error> {
    sqlx::query_as::<_, BackupRecord>("SELECT * FROM backups WHERE id = $1")
        .bind(id)
        .fetch_optional(pool)
        .await
}

// ---------------------------------------------------------------------------
// Redis job queue helpers
// ---------------------------------------------------------------------------

/// Payload serialised onto the Redis job queue.
#[derive(Debug, Serialize, Deserialize)]
pub struct BackupJob {
    pub backup_id: Uuid,
    pub backup_dir: String,
}

/// Payload serialised onto the Redis restore queue.
#[derive(Debug, Serialize, Deserialize)]
pub struct RestoreJob {
    pub backup_id: Uuid,
    pub file_path: String,
}

/// Push a [`BackupJob`] onto the Redis list `queue`.
pub async fn enqueue_backup(
    client: &redis::Client,
    queue: &str,
    job: &BackupJob,
) -> Result<(), AppError> {
    let mut conn = client.get_async_connection().await?;
    let payload = serde_json::to_string(job).map_err(|e| AppError::Internal(e.to_string()))?;
    conn.rpush::<_, _, ()>(queue, payload).await?;
    Ok(())
}

/// Push a [`RestoreJob`] onto the Redis list `queue`.
pub async fn enqueue_restore(
    client: &redis::Client,
    queue: &str,
    job: &RestoreJob,
) -> Result<(), AppError> {
    let mut conn = client.get_async_connection().await?;
    let payload = serde_json::to_string(job).map_err(|e| AppError::Internal(e.to_string()))?;
    conn.rpush::<_, _, ()>(queue, payload).await?;
    Ok(())
}

// ---------------------------------------------------------------------------
// HTTP handlers
// ---------------------------------------------------------------------------

/// `GET /health` ΓÇö liveness probe.
pub async fn health() -> impl IntoResponse {
    Json(serde_json::json!({ "status": "ok" }))
}

/// `POST /backups` ΓÇö create a backup record and enqueue the job.
#[instrument(skip(state))]
pub async fn create_backup(
    State(state): State<AppState>,
    body: Option<Json<CreateBackupRequest>>,
) -> Result<impl IntoResponse, AppError> {
    let label = body.and_then(|b| b.label.clone());
    if let Some(ref l) = label {
        info!(label = %l, "backup requested");
    }

    let record = create_backup_row(&state.db).await?;

    let job = BackupJob {
        backup_id: record.id,
        backup_dir: state.config.backup_dir.clone(),
    };
    enqueue_backup(&state.redis, &state.config.backup_queue, &job).await?;

    info!(backup_id = %record.id, "backup job enqueued");

    let response = JobEnqueued {
        id: record.id,
        status: JobStatus::Pending,
        message: "Backup job enqueued".to_string(),
    };
    Ok((StatusCode::ACCEPTED, Json(response)))
}

/// `GET /backups` ΓÇö list all backups.
#[instrument(skip(state))]
pub async fn list_backups(State(state): State<AppState>) -> Result<impl IntoResponse, AppError> {
    let records = list_backup_rows(&state.db).await?;
    Ok(Json(records))
}

/// `GET /backups/:id` ΓÇö fetch a single backup.
#[instrument(skip(state))]
pub async fn get_backup(
    State(state): State<AppState>,
    Path(id): Path<Uuid>,
) -> Result<impl IntoResponse, AppError> {
    let record = get_backup_row(&state.db, id)
        .await?
        .ok_or(AppError::NotFound)?;
    Ok(Json(record))
}

/// `POST /backups/:id/restore` ΓÇö enqueue a restore job for the given backup.
#[instrument(skip(state))]
pub async fn restore_backup(
    State(state): State<AppState>,
    Path(id): Path<Uuid>,
) -> Result<impl IntoResponse, AppError> {
    let record = get_backup_row(&state.db, id)
        .await?
        .ok_or(AppError::NotFound)?;

    let file_path = record.file_path.ok_or_else(|| {
        AppError::Internal(format!(
            "backup {id} has no file_path; it may not be completed yet"
        ))
    })?;

    let job = RestoreJob {
        backup_id: id,
        file_path,
    };
    enqueue_restore(&state.redis, &state.config.restore_queue, &job).await?;

    info!(backup_id = %id, "restore job enqueued");

    let response = JobEnqueued {
        id,
        status: JobStatus::Pending,
        message: "Restore job enqueued".to_string(),
    };
    Ok((StatusCode::ACCEPTED, Json(response)))
}

// ---------------------------------------------------------------------------
// Router
// ---------------------------------------------------------------------------

/// Build the Axum router with all routes and middleware.
pub fn build_router(state: AppState) -> Router {
    Router::new()
        .route("/health", get(health))
        .route("/backups", post(create_backup).get(list_backups))
        .route("/backups/:id", get(get_backup))
        .route("/backups/:id/restore", post(restore_backup))
        .layer(TraceLayer::new_for_http())
        .with_state(state)
}

// ---------------------------------------------------------------------------
// Entry point
// ---------------------------------------------------------------------------

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::try_from_default_env().unwrap_or_else(|_| "info".into()),
        )
        .init();

    let config = Arc::new(Config::from_env());

    let db = PgPoolOptions::new()
        .max_connections(10)
        .connect(&config.database_url)
        .await?;

    run_migrations(&db).await?;

    let redis = redis::Client::open(config.redis_url.as_str())?;

    let state = AppState {
        db,
        redis,
        config: config.clone(),
    };

    let router = build_router(state);

    info!(addr = %config.bind_addr, "backup service listening");
    let listener = tokio::net::TcpListener::bind(config.bind_addr).await?;
    axum::serve(listener, router).await?;

    Ok(())
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use axum::{
        body::Body,
        http::{Request, StatusCode},
    };
    use tower::ServiceExt;

    // ------------------------------------------------------------------
    // Unit tests ΓÇö no I/O required
    // ------------------------------------------------------------------

    #[test]
    fn job_status_display() {
        assert_eq!(JobStatus::Pending.to_string(), "pending");
        assert_eq!(JobStatus::Running.to_string(), "running");
        assert_eq!(JobStatus::Completed.to_string(), "completed");
        assert_eq!(JobStatus::Failed.to_string(), "failed");
    }

    #[test]
    fn job_status_serde_roundtrip() {
        let statuses = [
            JobStatus::Pending,
            JobStatus::Running,
            JobStatus::Completed,
            JobStatus::Failed,
        ];
        for status in &statuses {
            let json = serde_json::to_string(status).unwrap();
            let decoded: JobStatus = serde_json::from_str(&json).unwrap();
            assert_eq!(status, &decoded);
        }
    }

    #[test]
    fn backup_job_serde_roundtrip() {
        let id = Uuid::new_v4();
        let job = BackupJob {
            backup_id: id,
            backup_dir: "/tmp/test".to_string(),
        };
        let json = serde_json::to_string(&job).unwrap();
        let decoded: BackupJob = serde_json::from_str(&json).unwrap();
        assert_eq!(decoded.backup_id, id);
        assert_eq!(decoded.backup_dir, "/tmp/test");
    }

    #[test]
    fn restore_job_serde_roundtrip() {
        let id = Uuid::new_v4();
        let job = RestoreJob {
            backup_id: id,
            file_path: "/var/backups/crucible/2024-01-01.dump".to_string(),
        };
        let json = serde_json::to_string(&job).unwrap();
        let decoded: RestoreJob = serde_json::from_str(&json).unwrap();
        assert_eq!(decoded.backup_id, id);
        assert_eq!(decoded.file_path, "/var/backups/crucible/2024-01-01.dump");
    }

    #[test]
    fn config_defaults_applied() {
        // Only DATABASE_URL is required; test the other defaults.
        // We set DATABASE_URL to an arbitrary string; we are only checking Config
        // field construction here, not an actual connection.
        std::env::set_var("DATABASE_URL", "postgres://user:pass@localhost/test");
        std::env::remove_var("REDIS_URL");
        std::env::remove_var("BACKUP_QUEUE");
        std::env::remove_var("RESTORE_QUEUE");
        std::env::remove_var("BIND_ADDR");
        std::env::remove_var("BACKUP_DIR");

        let cfg = Config::from_env();

        assert_eq!(cfg.redis_url, "redis://127.0.0.1/");
        assert_eq!(cfg.backup_queue, "backup_jobs");
        assert_eq!(cfg.restore_queue, "restore_jobs");
        assert_eq!(cfg.bind_addr.to_string(), "0.0.0.0:8080");
        assert_eq!(cfg.backup_dir, "/var/backups/crucible");

        std::env::remove_var("DATABASE_URL");
    }

    #[test]
    fn app_error_not_found_status() {
        let err = AppError::NotFound;
        let response = err.into_response();
        assert_eq!(response.status(), StatusCode::NOT_FOUND);
    }

    #[test]
    fn app_error_internal_status() {
        let err = AppError::Internal("oops".to_string());
        let response = err.into_response();
        assert_eq!(response.status(), StatusCode::INTERNAL_SERVER_ERROR);
    }

    // ------------------------------------------------------------------
    // Integration tests ΓÇö HTTP layer only (no real DB/Redis)
    // ------------------------------------------------------------------

    /// Build a minimal router wired to a mock state for HTTP-layer tests.
    ///
    /// These tests only exercise the `/health` endpoint, which has no I/O
    /// dependencies and can run without database or Redis connections.
    fn test_router_health_only() -> Router {
        Router::new().route("/health", get(health))
    }

    #[tokio::test]
    async fn health_endpoint_returns_200() {
        let app = test_router_health_only();

        let response = app
            .oneshot(
                Request::builder()
                    .uri("/health")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();

        assert_eq!(response.status(), StatusCode::OK);
    }

    #[tokio::test]
    async fn health_endpoint_returns_ok_json() {
        let app = test_router_health_only();

        let response = app
            .oneshot(
                Request::builder()
                    .uri("/health")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();

        let body = axum::body::to_bytes(response.into_body(), usize::MAX)
            .await
            .unwrap();
        let value: serde_json::Value = serde_json::from_slice(&body).unwrap();
        assert_eq!(value["status"], "ok");
    }

    #[tokio::test]
    async fn unknown_route_returns_404() {
        let app = test_router_health_only();

        let response = app
            .oneshot(
                Request::builder()
                    .uri("/nonexistent")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();

        assert_eq!(response.status(), StatusCode::NOT_FOUND);
    }
}
