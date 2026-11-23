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

/// Default batch size for pruning expired job runs (1,000 records).
pub const DEFAULT_PRUNE_BATCH_SIZE: i64 = 1_000;
/// Default pause between deletion batches in milliseconds (50ms) to release locks.
pub const DEFAULT_PRUNE_BATCH_SLEEP_MS: u64 = 50;

/// Cleans up job run records older than the specified retention period using bounded batches
/// to prevent exclusive table locks and starvation of active job insertions.
/// Returns the total number of deleted records.
pub async fn cleanup_old_runs(pool: &PgPool, retain_days: i64) -> Result<u64, JobError> {
    cleanup_old_runs_batched(
        pool,
        retain_days,
        DEFAULT_PRUNE_BATCH_SIZE,
        std::time::Duration::from_millis(DEFAULT_PRUNE_BATCH_SLEEP_MS),
    )
    .await
}

/// Cleans up job run records older than the retention period in custom-sized batches
/// with an optional pause between batches.
pub async fn cleanup_old_runs_batched(
    pool: &PgPool,
    retain_days: i64,
    batch_size: i64,
    batch_pause: std::time::Duration,
) -> Result<u64, JobError> {
    let mut total_deleted: u64 = 0;
    let batch_size = if batch_size <= 0 {
        DEFAULT_PRUNE_BATCH_SIZE
    } else {
        batch_size
    };

    loop {
        let result = sqlx::query(
            r#"
            DELETE FROM job_runs
            WHERE id IN (
                SELECT id FROM job_runs
                WHERE started_at < NOW() - INTERVAL '1 day' * $1
                LIMIT $2
            )
            "#,
        )
        .bind(retain_days as f64)
        .bind(batch_size)
        .execute(pool)
        .await?;

        let rows_affected = result.rows_affected();
        total_deleted += rows_affected;

        // If fewer rows were deleted than the batch limit, all expired records have been pruned
        if (rows_affected as i64) < batch_size {
            break;
        }

        // Pause briefly between batches to yield table locks to concurrent job run insertions/updates
        if !batch_pause.is_zero() {
            tokio::time::sleep(batch_pause).await;
        }
    }

    Ok(total_deleted)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_job_run_status_roundtrip() {
        assert_eq!(JobRunStatus::Running.as_str(), "running");
        assert_eq!(JobRunStatus::Succeeded.as_str(), "succeeded");
        assert_eq!(JobRunStatus::Failed.as_str(), "failed");
        assert_eq!(JobRunStatus::TimedOut.as_str(), "timed_out");

        assert_eq!(JobRunStatus::from_str("running"), JobRunStatus::Running);
        assert_eq!(JobRunStatus::from_str("succeeded"), JobRunStatus::Succeeded);
        assert_eq!(JobRunStatus::from_str("failed"), JobRunStatus::Failed);
        assert_eq!(JobRunStatus::from_str("unknown"), JobRunStatus::TimedOut);
    }

    #[test]
    fn test_batch_constants() {
        assert_eq!(DEFAULT_PRUNE_BATCH_SIZE, 1_000);
        const { assert!(DEFAULT_PRUNE_BATCH_SIZE > 0 && DEFAULT_PRUNE_BATCH_SIZE <= 5_000) };
        assert_eq!(DEFAULT_PRUNE_BATCH_SLEEP_MS, 50);
    }
}
