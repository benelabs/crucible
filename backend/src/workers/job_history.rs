//! SCHEDULER APPROACH: Option B — Custom Tokio interval loop with cron crate
//! Rationale: We require strict control over execution guarantees such as distributed locking, timeouts, retries, and history logging. A custom loop using `tokio::time::sleep_until` and the `cron` crate allows us to wrap the execution precisely in a Redis lock and accurately handle timeouts using `tokio::time::timeout`.

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use sqlx::{PgPool, Row, Type};
use uuid::Uuid;

use crate::workers::error::JobError;

/// Status of a job run execution.
#[derive(Debug, Clone, Serialize, Deserialize, Type, PartialEq, Eq)]
#[sqlx(type_name = "job_run_status", rename_all = "snake_case")]
pub enum JobRunStatus {
    Running,
    Succeeded,
    Failed,
    TimedOut,
}

impl JobRunStatus {
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::Running => "running",
            Self::Succeeded => "succeeded",
            Self::Failed => "failed",
            Self::TimedOut => "timed_out",
        }
    }

    pub fn from_str(s: &str) -> Self {
        match s {
            "running" => Self::Running,
            "succeeded" => Self::Succeeded,
            "failed" => Self::Failed,
            _ => Self::TimedOut,
        }
    }
}

/// Represents a single execution of a scheduled job.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct JobRun {
    pub id: Uuid,
    pub job_name: String,
    pub started_at: DateTime<Utc>,
    pub finished_at: Option<DateTime<Utc>>,
    pub status: JobRunStatus,
    pub error_message: Option<String>,
    pub duration_ms: Option<i64>,
}

impl JobRun {
    pub fn from_row(row: &sqlx::postgres::PgRow) -> Result<Self, sqlx::Error> {
        let id: Uuid = row.try_get("id")?;
        let job_name: String = row.try_get("job_name")?;
        let started_at: DateTime<Utc> = row.try_get("started_at")?;
        let finished_at: Option<DateTime<Utc>> = row.try_get("finished_at")?;
        let status_str: String = row.try_get("status")?;
        let error_message: Option<String> = row.try_get("error_message")?;
        let duration_ms: Option<i64> = row.try_get("duration_ms")?;

        Ok(Self {
            id,
            job_name,
            started_at,
            finished_at,
            status: JobRunStatus::from_str(&status_str),
            error_message,
            duration_ms,
        })
    }
}

/// Records the start of a job execution, inserting a new record with `Running` status.
/// Returns the unique `Uuid` for this job run.
pub async fn record_start(pool: &PgPool, job_name: &str) -> Result<Uuid, JobError> {
    let id = Uuid::new_v4();
    sqlx::query(
        r#"
        INSERT INTO job_runs (id, job_name, started_at, status)
        VALUES ($1, $2, NOW(), 'running')
        "#,
    )
    .bind(id)
    .bind(job_name)
    .execute(pool)
    .await?;

    Ok(id)
}

/// Records the successful completion of a job run.
pub async fn record_success(pool: &PgPool, run_id: Uuid, duration_ms: i64) -> Result<(), JobError> {
    sqlx::query(
        r#"
        UPDATE job_runs
        SET status = 'succeeded', finished_at = NOW(), duration_ms = $2
        WHERE id = $1
        "#,
    )
    .bind(run_id)
    .bind(duration_ms)
    .execute(pool)
    .await?;

    Ok(())
}

/// Records the failure of a job run, storing the error message.
pub async fn record_failure(
    pool: &PgPool,
    run_id: Uuid,
    error: &str,
    duration_ms: i64,
) -> Result<(), JobError> {
    sqlx::query(
        r#"
        UPDATE job_runs
        SET status = 'failed', finished_at = NOW(), duration_ms = $2, error_message = $3
        WHERE id = $1
        "#,
    )
    .bind(run_id)
    .bind(duration_ms)
    .bind(error)
    .execute(pool)
    .await?;

    Ok(())
}

/// Records that a job timed out.
pub async fn record_timeout(pool: &PgPool, run_id: Uuid, duration_ms: i64) -> Result<(), JobError> {
    sqlx::query(
        r#"
        UPDATE job_runs
        SET status = 'timed_out', finished_at = NOW(), duration_ms = $2, error_message = 'Job execution timed out'
        WHERE id = $1
        "#,
    )
    .bind(run_id)
    .bind(duration_ms)
    .execute(pool)
    .await?;

    Ok(())
}

/// Retrieves the most recent runs for a given job, ordered chronologically descending.
pub async fn get_recent_runs(
    pool: &PgPool,
    job_name: &str,
    limit: i64,
) -> Result<Vec<JobRun>, JobError> {
    let rows = sqlx::query(
        r#"
        SELECT id, job_name, started_at, finished_at, status, error_message, duration_ms
        FROM job_runs
        WHERE job_name = $1
        ORDER BY started_at DESC
        LIMIT $2
        "#,
    )
    .bind(job_name)
    .bind(limit)
    .fetch_all(pool)
    .await?;

    let mut runs = Vec::with_capacity(rows.len());
    for row in rows {
        runs.push(JobRun::from_row(&row)?);
    }

    Ok(runs)
}

/// Maximum number of rows removed per `DELETE` statement during cleanup.
const CLEANUP_BATCH_SIZE: i64 = 1_000;

/// Pause between cleanup batches so concurrent inserts and updates are not starved.
const CLEANUP_BATCH_PAUSE: std::time::Duration = std::time::Duration::from_millis(50);

/// Cleans up job run records older than the specified retention period.
/// Returns the number of deleted records.
///
/// Rows are deleted in bounded batches of [`CLEANUP_BATCH_SIZE`] so each
/// statement holds row locks briefly instead of locking the whole backlog in a
/// single long-running transaction. `SKIP LOCKED` ensures rows touched by an
/// in-flight job update are left for the next run rather than blocking it.
pub async fn cleanup_old_runs(pool: &PgPool, retain_days: i64) -> Result<u64, JobError> {
    let mut total_deleted = 0u64;

    loop {
        let result = sqlx::query(
            r#"
            DELETE FROM job_runs
            WHERE id IN (
                SELECT id FROM job_runs
                WHERE started_at < NOW() - INTERVAL '1 day' * $1
                LIMIT $2
                FOR UPDATE SKIP LOCKED
            )
            "#,
        )
        .bind(retain_days as f64)
        .bind(CLEANUP_BATCH_SIZE)
        .execute(pool)
        .await?;

        let deleted = result.rows_affected();
        total_deleted += deleted;

        if deleted < CLEANUP_BATCH_SIZE as u64 {
            break;
        }

        tokio::time::sleep(CLEANUP_BATCH_PAUSE).await;
    }

    Ok(total_deleted)
}
