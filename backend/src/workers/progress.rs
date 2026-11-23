use redis::{AsyncCommands, Client};
use serde::{Deserialize, Serialize};
use sqlx::PgPool;
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};
use tracing::{debug, error, info};

/// Phase transition event broadcast when a job transitions between stages.
#[derive(Serialize, Deserialize, Debug, Clone, PartialEq, Eq)]
pub struct PhaseTransitionEvent {
    pub job_id: String,
    pub previous_phase: Option<String>,
    pub new_phase: String,
    pub timestamp_ms: u64,
}

/// Job progress tracker that monitors and reports job execution status
#[derive(Debug, Clone)]
pub struct JobProgressTracker {
    #[allow(dead_code)]
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
        let mut redis_conn = self.redis_client.get_multiplexed_async_connection().await?;

        // Get list of running jobs from Redis (assuming they're stored with "job:*:progress" prefix)
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
                            if total_steps > 0 {
                                progress.progress_percentage =
                                    (progress.completed_steps as f64 / total_steps as f64 * 100.0)
                                        as u8;
                            }
                        }

                        // Evict completed jobs (100% progress) from active tracking to prevent memory accumulation
                        if progress.progress_percentage >= 100 {
                            redis_conn.del::<_, ()>(&key).await?;
                            debug!("Evicted completed job progress entry: {}", progress.job_id);
                        } else {
                            // Store updated progress with TTL expiration
                            redis_conn
                                .set_ex::<_, _, ()>(
                                    &key,
                                    serde_json::to_string(&progress)?,
                                    3600_u64, // 1 hour TTL
                                )
                                .await?;
                            debug!("Updated progress for job: {}", progress.job_id);
                        }
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
        let mut redis_conn = self.redis_client.get_multiplexed_async_connection().await?;
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
        let mut redis_conn = self.redis_client.get_multiplexed_async_connection().await?;
        let key = format!("job:{}:progress", job_id);

        let current = self.get_job_progress(job_id).await?;
        let started_at = current
            .as_ref()
            .map(|p| p.started_at)
            .unwrap_or_else(Instant::now);
        let current_phase = current.as_ref().and_then(|p| p.current_phase.clone());
        let previous_phase = current.as_ref().and_then(|p| p.previous_phase.clone());

        let progress_percentage = if let Some(total) = total_steps {
            if total > 0 {
                (completed_steps as f64 / total as f64 * 100.0) as u8
            } else {
                0
            }
        } else {
            0
        };

        let progress = JobProgress {
            job_id: job_id.to_string(),
            completed_steps,
            total_steps,
            progress_percentage,
            current_phase,
            previous_phase,
            last_updated: Instant::now(),
            started_at,
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

    /// Update job execution phase, emitting an explicit PhaseTransitionEvent even for sub-second stages
    pub async fn update_job_phase(
        &self,
        job_id: &str,
        new_phase: &str,
        completed_steps: u64,
        total_steps: Option<u64>,
    ) -> Result<Option<PhaseTransitionEvent>, Box<dyn std::error::Error>> {
        let mut redis_conn = self.redis_client.get_multiplexed_async_connection().await?;
        let key = format!("job:{}:progress", job_id);

        let current = self.get_job_progress(job_id).await?;
        let started_at = current
            .as_ref()
            .map(|p| p.started_at)
            .unwrap_or_else(Instant::now);
        let previous_phase = current.as_ref().and_then(|p| p.current_phase.clone());

        let transition_event = if previous_phase.as_deref() != Some(new_phase) {
            let now_ms = SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .unwrap_or_default()
                .as_millis() as u64;

            let event = PhaseTransitionEvent {
                job_id: job_id.to_string(),
                previous_phase: previous_phase.clone(),
                new_phase: new_phase.to_string(),
                timestamp_ms: now_ms,
            };

            // Publish phase transition event to Redis channel so WebSocket consumers receive immediate notice
            let event_json = serde_json::to_string(&event)?;
            let _: () = redis_conn.publish("job:events", &event_json).await?;
            info!(job_id = %job_id, phase = %new_phase, "Emitted phase transition event");
            Some(event)
        } else {
            None
        };

        let progress_percentage = if let Some(total) = total_steps {
            if total > 0 {
                (completed_steps as f64 / total as f64 * 100.0) as u8
            } else {
                0
            }
        } else {
            0
        };

        let progress = JobProgress {
            job_id: job_id.to_string(),
            completed_steps,
            total_steps,
            progress_percentage,
            current_phase: Some(new_phase.to_string()),
            previous_phase,
            last_updated: Instant::now(),
            started_at,
        };

        redis_conn
            .set_ex::<_, _, ()>(&key, serde_json::to_string(&progress)?, 3600_u64)
            .await?;

        Ok(transition_event)
    }
}

/// Job progress structure
#[derive(Serialize, Deserialize, Debug, Clone)]
pub struct JobProgress {
    pub job_id: String,
    pub completed_steps: u64,
    pub total_steps: Option<u64>,
    pub progress_percentage: u8,
    #[serde(default)]
    pub current_phase: Option<String>,
    #[serde(default)]
    pub previous_phase: Option<String>,
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_phase_transition_event_serialization() {
        let event = PhaseTransitionEvent {
            job_id: "build-123".to_string(),
            previous_phase: Some("compiling".to_string()),
            new_phase: "linking".to_string(),
            timestamp_ms: 1700000000000,
        };

        let json = serde_json::to_string(&event).unwrap();
        let parsed: PhaseTransitionEvent = serde_json::from_str(&json).unwrap();
        assert_eq!(event, parsed);
        assert_eq!(parsed.job_id, "build-123");
        assert_eq!(parsed.previous_phase.as_deref(), Some("compiling"));
        assert_eq!(parsed.new_phase, "linking");
    }

    #[test]
    fn test_job_progress_with_phase_information() {
        let progress = JobProgress {
            job_id: "test-job".to_string(),
            completed_steps: 1,
            total_steps: Some(10),
            progress_percentage: 10,
            current_phase: Some("init".to_string()),
            previous_phase: None,
            last_updated: Instant::now(),
            started_at: Instant::now(),
        };

        let json = serde_json::to_string(&progress).unwrap();
        let deserialized: JobProgress = serde_json::from_str(&json).unwrap();
        assert_eq!(deserialized.job_id, "test-job");
        assert_eq!(deserialized.current_phase.as_deref(), Some("init"));
        assert_eq!(deserialized.previous_phase, None);
    }
}
