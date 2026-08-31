use chrono::{DateTime, Utc};
use redis::AsyncCommands;
use serde::{Deserialize, Serialize};
use std::sync::Arc;
use tracing::{info, warn};
use utoipa::ToSchema;

use crate::error::AppError;

/// Dead Letter Job payload representing a permanently failed background task.
#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
pub struct DeadLetterJob {
    pub id: String,
    pub job_name: String,
    pub payload: serde_json::Value,
    pub failure_reason: String,
    pub attempts: u32,
    pub first_failed_at: DateTime<Utc>,
    pub failed_at: DateTime<Utc>,
}

/// Dead Letter Queue (DLQ) manager for storing, inspecting, replaying, and purging failed jobs.
#[derive(Clone)]
pub struct DeadLetterQueue {
    redis: Arc<redis::Client>,
    queue_key: String,
}

impl DeadLetterQueue {
    pub fn new(redis: Arc<redis::Client>) -> Self {
        Self {
            redis,
            queue_key: "dlq:failed_jobs".to_string(),
        }
    }

    pub fn with_queue_key(redis: Arc<redis::Client>, queue_key: impl Into<String>) -> Self {
        Self {
            redis,
            queue_key: queue_key.into(),
        }
    }

    /// Enqueues a permanently failed job into the Dead Letter Queue.
    pub async fn enqueue(&self, job: DeadLetterJob) -> Result<(), AppError> {
        warn!(
            job_id = %job.id,
            job_name = %job.job_name,
            attempts = job.attempts,
            reason = %job.failure_reason,
            "Routing permanently failed job to Dead Letter Queue (DLQ)"
        );

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

        let mut jobs = Vec::new();
        let hash_key = format!("{}:map", self.queue_key);

        for id in job_ids {
            let json_str: Option<String> = conn.hget(&hash_key, &id).await.map_err(AppError::Redis)?;
            if let Some(data) = json_str {
                if let Ok(job) = serde_json::from_str::<DeadLetterJob>(&data) {
                    jobs.push(job);
                }
            }
        }

        Ok(jobs)
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
    pub async fn replay(&self, job_id: &str) -> Result<DeadLetterJob, AppError> {
        let job = self
            .get(job_id)
            .await?
            .ok_or_else(|| AppError::NotFound(format!("Dead letter job {job_id} not found")))?;

        let mut conn = self.redis.get_multiplexed_async_connection().await.map_err(AppError::Redis)?;
        let hash_key = format!("{}:map", self.queue_key);

        let _: () = conn.hdel(&hash_key, job_id).await.map_err(AppError::Redis)?;
        let _: () = conn.lrem(&self.queue_key, 0, job_id).await.map_err(AppError::Redis)?;

        info!(job_id = %job_id, job_name = %job.job_name, "Replaying dead letter job");
        Ok(job)
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
