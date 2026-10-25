use chrono::{DateTime, Utc};
use redis::AsyncCommands;
use serde::{Deserialize, Serialize};
use std::sync::Arc;
use tracing::{info, warn};
use utoipa::ToSchema;

use crate::error::AppError;

pub const DEFAULT_MAX_REPLAY_ATTEMPTS: u32 = 3;

/// Dead Letter Job payload representing a permanently failed background task.
#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
pub struct DeadLetterJob {
    pub id: String,
    pub job_name: String,
    pub payload: serde_json::Value,
    pub failure_reason: String,
    pub attempts: u32,
    #[serde(default)]
    pub replay_count: u32,
    pub first_failed_at: DateTime<Utc>,
    pub failed_at: DateTime<Utc>,
}

/// Dead Letter Queue (DLQ) manager for storing, inspecting, replaying, and purging failed jobs.
#[derive(Clone)]
pub struct DeadLetterQueue {
    redis: Arc<redis::Client>,
    queue_key: String,
    quarantine_key: String,
    max_replay_attempts: u32,
}

impl DeadLetterQueue {
    pub fn new(redis: Arc<redis::Client>) -> Self {
        Self {
            redis,
            queue_key: "dlq:failed_jobs".to_string(),
            quarantine_key: "dlq:quarantined_jobs".to_string(),
            max_replay_attempts: DEFAULT_MAX_REPLAY_ATTEMPTS,
        }
    }

    pub fn with_queue_key(redis: Arc<redis::Client>, queue_key: impl Into<String>) -> Self {
        let key = queue_key.into();
        let quarantine_key = format!("{}:quarantine", key);
        Self {
            redis,
            queue_key: key,
            quarantine_key,
            max_replay_attempts: DEFAULT_MAX_REPLAY_ATTEMPTS,
        }
    }

    pub fn with_config(
        redis: Arc<redis::Client>,
        queue_key: impl Into<String>,
        quarantine_key: impl Into<String>,
        max_replay_attempts: u32,
    ) -> Self {
        Self {
            redis,
            queue_key: queue_key.into(),
            quarantine_key: quarantine_key.into(),
            max_replay_attempts,
        }
    }

    pub fn max_replay_attempts(&self) -> u32 {
        self.max_replay_attempts
    }

    pub fn quarantine_key(&self) -> &str {
        &self.quarantine_key
    }

    /// Enqueues a permanently failed job into the Dead Letter Queue.
    /// If the job has reached or exceeded max_replay_attempts, it is diverted to the quarantine archive.
    pub async fn enqueue(&self, job: DeadLetterJob) -> Result<(), AppError> {
        if job.replay_count >= self.max_replay_attempts {
            warn!(
                job_id = %job.id,
                job_name = %job.job_name,
                replay_count = job.replay_count,
                max = self.max_replay_attempts,
                "Job has reached or exceeded max replay attempts; routing to quarantine archive"
            );
            return self.quarantine(job).await;
        }

        warn!(
            job_id = %job.id,
            job_name = %job.job_name,
            attempts = job.attempts,
            replay_count = job.replay_count,
            reason = %job.failure_reason,
            "Routing permanently failed job to Dead Letter Queue (DLQ)"
        );
        crate::services::metrics::inc_worker_job_dropped_metric();

        let json_data = serde_json::to_string(&job).map_err(AppError::Serialization)?;
        let hash_key = format!("{}:map", self.queue_key);

        let mut conn = self.redis.get_multiplexed_async_connection().await.map_err(AppError::Redis)?;

        let _: () = conn
            .hset(&hash_key, &job.id, &json_data)
            .await
            .map_err(AppError::Redis)?;

        let _: () = conn
            .lpush(&self.queue_key, &job.id)
            .await
            .map_err(AppError::Redis)?;

        Ok(())
    }

    /// Fetches all dead-lettered jobs up to `limit`.
    pub async fn list(&self, limit: isize) -> Result<Vec<DeadLetterJob>, AppError> {
        let mut conn = self.redis.get_multiplexed_async_connection().await.map_err(AppError::Redis)?;

        let job_ids: Vec<String> = conn
            .lrange(&self.queue_key, 0, limit - 1)
            .await
            .map_err(AppError::Redis)?;

        if job_ids.is_empty() {
            return Ok(Vec::new());
        }

        let hash_key = format!("{}:map", self.queue_key);
        let job_data: Vec<Option<String>> = conn
            .hget(&hash_key, &job_ids)
            .await
            .map_err(AppError::Redis)?;

        Ok(deserialize_jobs(job_data))
    }

    /// Retrieves a single dead letter job by ID.
    pub async fn get(&self, job_id: &str) -> Result<Option<DeadLetterJob>, AppError> {
        let mut conn = self.redis.get_multiplexed_async_connection().await.map_err(AppError::Redis)?;
        let hash_key = format!("{}:map", self.queue_key);

        let json_str: Option<String> = conn.hget(&hash_key, job_id).await.map_err(AppError::Redis)?;
        match json_str {
            Some(data) => Ok(Some(serde_json::from_str(&data).map_err(AppError::Serialization)?)),
            None => Ok(None),
        }
    }

    /// Replays a dead letter job by removing it from DLQ and returning it for worker reprocessing.
    /// If the job has reached or exceeded max_replay_attempts, it is quarantined to prevent
    /// poison pill infinite loops and an error is returned.
    pub async fn replay(&self, job_id: &str) -> Result<DeadLetterJob, AppError> {
        let mut job = self
            .get(job_id)
            .await?
            .ok_or_else(|| AppError::NotFound(format!("Dead letter job {job_id} not found")))?;

        if job.replay_count >= self.max_replay_attempts {
            warn!(
                job_id = %job_id,
                job_name = %job.job_name,
                replay_count = job.replay_count,
                max_replays = self.max_replay_attempts,
                "Poison pill detected: job exceeded maximum replay attempts, moving to quarantine"
            );
            self.quarantine(job.clone()).await?;
            let _ = self.purge(job_id).await;
            return Err(AppError::ValidationError(format!(
                "Job {job_id} exceeded maximum replay attempts ({}) and was quarantined",
                self.max_replay_attempts
            )));
        }

        let mut conn = self.redis.get_multiplexed_async_connection().await.map_err(AppError::Redis)?;
        let hash_key = format!("{}:map", self.queue_key);

        let _: () = conn.hdel(&hash_key, job_id).await.map_err(AppError::Redis)?;
        let _: () = conn.lrem(&self.queue_key, 0, job_id).await.map_err(AppError::Redis)?;

        job.replay_count += 1;
        info!(
            job_id = %job_id,
            job_name = %job.job_name,
            replay_count = job.replay_count,
            "Replaying dead letter job"
        );
        Ok(job)
    }

    /// Moves a failed job into the permanent quarantine failure archive.
    pub async fn quarantine(&self, job: DeadLetterJob) -> Result<(), AppError> {
        let json_data = serde_json::to_string(&job).map_err(AppError::Serialization)?;
        let quarantine_hash = format!("{}:map", self.quarantine_key);

        let mut conn = self.redis.get_multiplexed_async_connection().await.map_err(AppError::Redis)?;

        let _: () = conn
            .hset(&quarantine_hash, &job.id, &json_data)
            .await
            .map_err(AppError::Redis)?;

        let _: () = conn
            .lpush(&self.quarantine_key, &job.id)
            .await
            .map_err(AppError::Redis)?;

        let dlq_hash = format!("{}:map", self.queue_key);
        let _: () = conn.hdel(&dlq_hash, &job.id).await.map_err(AppError::Redis)?;
        let _: () = conn.lrem(&self.queue_key, 0, &job.id).await.map_err(AppError::Redis)?;

        warn!(
            job_id = %job.id,
            job_name = %job.job_name,
            replay_count = job.replay_count,
            "Job quarantined in permanent failure archive"
        );
        Ok(())
    }

    /// Fetches all quarantined jobs up to `limit`.
    pub async fn list_quarantine(&self, limit: isize) -> Result<Vec<DeadLetterJob>, AppError> {
        let mut conn = self.redis.get_multiplexed_async_connection().await.map_err(AppError::Redis)?;

        let job_ids: Vec<String> = conn
            .lrange(&self.quarantine_key, 0, limit - 1)
            .await
            .map_err(AppError::Redis)?;

        if job_ids.is_empty() {
            return Ok(Vec::new());
        }

        let hash_key = format!("{}:map", self.quarantine_key);
        let job_data: Vec<Option<String>> = redis::cmd("HMGET")
            .arg(&hash_key)
            .arg(&job_ids)
            .query_async(&mut conn)
            .await
            .map_err(AppError::Redis)?;

        Ok(deserialize_jobs(job_data))
    }

    /// Retrieves a single quarantined dead letter job by ID.
    pub async fn get_quarantined(&self, job_id: &str) -> Result<Option<DeadLetterJob>, AppError> {
        let mut conn = self.redis.get_multiplexed_async_connection().await.map_err(AppError::Redis)?;
        let hash_key = format!("{}:map", self.quarantine_key);

        let json_str: Option<String> = conn.hget(&hash_key, job_id).await.map_err(AppError::Redis)?;
        match json_str {
            Some(data) => Ok(Some(serde_json::from_str(&data).map_err(AppError::Serialization)?)),
            None => Ok(None),
        }
    }

    /// Permanently deletes a job from the DLQ.
    pub async fn purge(&self, job_id: &str) -> Result<(), AppError> {
        let mut conn = self.redis.get_multiplexed_async_connection().await.map_err(AppError::Redis)?;
        let hash_key = format!("{}:map", self.queue_key);

        let _: () = conn.hdel(&hash_key, job_id).await.map_err(AppError::Redis)?;
        let _: () = conn.lrem(&self.queue_key, 0, job_id).await.map_err(AppError::Redis)?;

        info!(job_id = %job_id, "Purged job from DLQ");
        Ok(())
    }
}

fn deserialize_jobs(job_data: Vec<Option<String>>) -> Vec<DeadLetterJob> {
    job_data
        .into_iter()
        .flatten()
        .filter_map(|data| serde_json::from_str::<DeadLetterJob>(&data).ok())
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn deserialize_jobs_preserves_order_and_skips_missing_or_invalid_entries() {
        let now = Utc::now();
        let first = DeadLetterJob {
            id: "first".to_string(),
            job_name: "task".to_string(),
            payload: serde_json::json!({}),
            failure_reason: "failed".to_string(),
            attempts: 1,
            replay_count: 0,
            first_failed_at: now,
            failed_at: now,
        };
        let last = DeadLetterJob {
            id: "last".to_string(),
            ..first.clone()
        };

        let jobs = deserialize_jobs(vec![
            Some(serde_json::to_string(&first).unwrap()),
            None,
            Some("not valid JSON".to_string()),
            Some(serde_json::to_string(&last).unwrap()),
        ]);

        assert_eq!(
            jobs.iter().map(|job| job.id.as_str()).collect::<Vec<_>>(),
            vec!["first", "last"]
        );
    }

    #[test]
    fn deserialize_handles_missing_replay_count_gracefully() {
        let json_without_replay_count = r#"{
            "id": "legacy-job-1",
            "job_name": "legacy_task",
            "payload": {},
            "failure_reason": "timeout",
            "attempts": 3,
            "first_failed_at": "2026-10-21T00:00:00Z",
            "failed_at": "2026-10-21T00:00:00Z"
        }"#;

        let job: DeadLetterJob = serde_json::from_str(json_without_replay_count).unwrap();
        assert_eq!(job.id, "legacy-job-1");
        assert_eq!(job.replay_count, 0);
    }
}
