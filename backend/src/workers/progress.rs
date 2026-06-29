use redis::{AsyncCommands, Client};
use serde::{Deserialize, Serialize};
use sqlx::PgPool;
use std::sync::Arc;
use std::time::{Duration, Instant};
use tracing::{debug, error, info, warn};

/// Job progress tracker that monitors and reports job execution status
#[derive(Debug, Clone)]
pub struct JobProgressTracker {
    db_pool: PgPool,
    redis_client: Client,
    update_interval: Duration,
}

impl JobProgressTracker {
    /// Create a new job progress tracker
    pub fn new(db_pool: PgPool, redis_client: Client, update_interval: Duration) -> Self {
        Self {
            db_pool,
            redis_client,
            update_interval,
        }
    }

    /// Start tracking job progress
    pub async fn start(&self) -> Result<(), Box<dyn std::error::Error>> {
        info!("Starting job progress tracker...");

        loop {
            if let Err(e) = self.update_progress().await {
                error!("Failed to update job progress: {}", e);
            }

            // Wait for the configured interval before next update
            tokio::time::sleep(self.update_interval).await;
        }
    }

    /// Update progress for running jobs
    async fn update_progress(&self) -> Result<(), Box<dyn std::error::Error>> {
        info!("Updating job progress...");

        // Get Redis connection
        let mut redis_conn = self.redis_client.get_async_connection().await?;

        // Get list of running jobs from Redis (assuming they're stored with "job:" prefix)
        let job_keys: Vec<String> = redis_conn.keys("job:*:progress").await?;

        for key in job_keys {
            // Get current progress data
            let progress_data: Option<String> = redis_conn.get(&key).await?;

            if let Some(json) = progress_data {
                match serde_json::from_str::<JobProgress>(&json) {
                    Ok(mut progress) => {
                        // Update timestamp and potentially other metrics
                        progress.last_updated = Instant::now();

                        // Calculate progress percentage if applicable
                        if let Some(total_steps) = progress.total_steps {
                            progress.progress_percentage =
                                (progress.completed_steps as f64 / total_steps as f64 * 100.0)
                                    as u8;
                        }

                        // Store updated progress
                        redis_conn
                            .set_ex::<_, _, ()>(
                                &key,
                                serde_json::to_string(&progress)?,
                                3600_u64, // 1 hour TTL
                            )
                            .await?;

                        debug!("Updated progress for job: {}", progress.job_id);
                    }
                    Err(e) => {
                        error!("Failed to parse progress data for {}: {}", key, e);
                    }
                }
            }
        }

        info!("Job progress update completed");
        Ok(())
    }

    /// Get progress for a specific job
    pub async fn get_job_progress(
        &self,
        job_id: &str,
    ) -> Result<Option<JobProgress>, Box<dyn std::error::Error>> {
        let mut redis_conn = self.redis_client.get_async_connection().await?;
        let key = format!("job:{}:progress", job_id);

        let progress_data: Option<String> = redis_conn.get(&key).await?;

        match progress_data {
            Some(json) => {
                let progress: JobProgress = serde_json::from_str(&json)?;
                Ok(Some(progress))
            }
            None => Ok(None),
        }
    }

    /// Update progress for a specific job
    pub async fn update_job_progress(
        &self,
        job_id: &str,
        completed_steps: u64,
        total_steps: Option<u64>,
    ) -> Result<(), Box<dyn std::error::Error>> {
        let mut redis_conn = self.redis_client.get_async_connection().await?;
        let key = format!("job:{}:progress", job_id);

        let progress = JobProgress {
            job_id: job_id.to_string(),
            completed_steps,
            total_steps,
            progress_percentage: 0,
            last_updated: Instant::now(),
            started_at: Instant::now(),
        };

        redis_conn
            .set_ex::<_, _, ()>(
                &key,
                serde_json::to_string(&progress)?,
                3600_u64, // 1 hour TTL
            )
            .await?;

        Ok(())
    }
}

/// Job progress structure
#[derive(Serialize, Deserialize, Debug, Clone)]
pub struct JobProgress {
    pub job_id: String,
    pub completed_steps: u64,
    pub total_steps: Option<u64>,
    pub progress_percentage: u8,
    #[serde(with = "instant_serde")]
    pub last_updated: std::time::Instant,
    #[serde(with = "instant_serde")]
    pub started_at: std::time::Instant,
}

mod instant_serde {
    use serde::{self, Deserialize, Deserializer, Serializer};
    use std::time::{Duration, Instant};

    pub fn serialize<S>(instant: &Instant, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: Serializer,
    {
        serializer.serialize_str(&instant.elapsed().as_secs().to_string())
    }

    pub fn deserialize<'de, D>(deserializer: D) -> Result<Instant, D::Error>
    where
        D: Deserializer<'de>,
    {
        #[derive(Deserialize)]
        #[serde(untagged)]
        enum StringOrU64 {
            Str(String),
            Num(u64),
        }

        let val = StringOrU64::deserialize(deserializer)?;
        let secs = match val {
            StringOrU64::Str(s) => s.parse::<u64>().map_err(serde::de::Error::custom)?,
            StringOrU64::Num(n) => n,
        };
        Ok(Instant::now() - Duration::from_secs(secs))
    }
}
