use redis::{AsyncCommands, Client};
use serde::{Deserialize, Serialize};
use sqlx::PgPool;
use std::time::{Duration, Instant};
use tracing::{debug, error, info};

/// Redis channel suffix used for job progress events. WebSocket handlers can
/// subscribe to `job:<job_id>:events` and forward these messages to clients.
const JOB_EVENTS_SUFFIX: &str = "events";
const JOB_PROGRESS_TTL_SECONDS: u64 = 3600;

/// Job progress tracker that monitors and reports job execution status.
///
/// Percentage updates are stored in Redis, while phase transitions are also
/// published as events. Phase events are deliberately independent of any
/// percentage-delta threshold, so a short phase remains observable even when
/// it starts and finishes between polling ticks.
#[derive(Debug, Clone)]
pub struct JobProgressTracker {
    db_pool: PgPool,
    redis_client: Client,
    update_interval: Duration,
}

impl JobProgressTracker {
    /// Create a new job progress tracker.
    pub fn new(db_pool: PgPool, redis_client: Client, update_interval: Duration) -> Self {
        Self {
            db_pool,
            redis_client,
            update_interval,
        }
    }

    /// Start tracking job progress.
    pub async fn start(&self) -> Result<(), Box<dyn std::error::Error>> {
        info!("Starting job progress tracker...");

        loop {
            if let Err(e) = self.update_progress().await {
                error!(error = %e, "Failed to update job progress");
            }

            tokio::time::sleep(self.update_interval).await;
        }
    }

    /// Update progress for running jobs.
    async fn update_progress(&self) -> Result<(), Box<dyn std::error::Error>> {
        info!("Updating job progress...");
        let mut redis_conn = self.redis_client.get_multiplexed_async_connection().await?;
        let job_keys: Vec<String> = redis_conn.keys("job:*:progress").await?;

        for key in job_keys {
            let progress_data: Option<String> = redis_conn.get(&key).await?;
            let Some(json) = progress_data else {
                continue;
            };

            match serde_json::from_str::<JobProgress>(&json) {
                Ok(mut progress) => {
                    progress.last_updated = Instant::now();
                    progress.recalculate_percentage();

                    if progress.progress_percentage >= 100 {
                        // Emit completion before removing the active progress key.
                        // This ensures consumers can observe a terminal transition.
                        if progress.phase != JobPhase::Completed {
                            progress.phase = JobPhase::Completed;
                            self.publish_phase_transition(&mut redis_conn, &progress).await?;
                        }
                        redis_conn.del::<_, ()>(&key).await?;
                        debug!(job_id = %progress.job_id, "Evicted completed job progress entry");
                    } else {
                        redis_conn
                            .set_ex::<_, _, ()>(
                                &key,
                                serde_json::to_string(&progress)?,
                                JOB_PROGRESS_TTL_SECONDS,
                            )
                            .await?;
                        debug!(job_id = %progress.job_id, "Updated job progress");
                    }
                }
                Err(e) => error!(key = %key, error = %e, "Failed to parse job progress data"),
            }
        }

        info!("Job progress update completed");
        Ok(())
    }

    /// Get progress for a specific job.
    pub async fn get_job_progress(
        &self,
        job_id: &str,
    ) -> Result<Option<JobProgress>, Box<dyn std::error::Error>> {
        let mut redis_conn = self.redis_client.get_multiplexed_async_connection().await?;
        let key = progress_key(job_id);
        let progress_data: Option<String> = redis_conn.get(key).await?;
        progress_data
            .map(|json| serde_json::from_str::<JobProgress>(&json).map_err(Into::into))
            .transpose()
    }

    /// Update a job's progress without changing its explicit phase.
    ///
    /// This backward-compatible method uses `Running` for non-terminal progress
    /// and `Completed` for 100%. Call [`Self::update_job_phase`] at each meaningful
    /// worker phase boundary to emit notifications for short-running phases.
    pub async fn update_job_progress(
        &self,
        job_id: &str,
        completed_steps: u64,
        total_steps: Option<u64>,
    ) -> Result<(), Box<dyn std::error::Error>> {
        let phase = if total_steps.is_some_and(|total| total > 0 && completed_steps >= total) {
            JobPhase::Completed
        } else {
            JobPhase::Running
        };
        self.update_job_phase(job_id, phase, completed_steps, total_steps)
            .await
    }

    /// Update progress and explicitly emit an event when the job phase changes.
    ///
    /// The event is published to `job:<job_id>:events` as JSON. WebSocket code
    /// should subscribe to this Redis channel and forward the event to connected
    /// clients. Events are published regardless of percentage delta.
    pub async fn update_job_phase(
        &self,
        job_id: &str,
        phase: JobPhase,
        completed_steps: u64,
        total_steps: Option<u64>,
    ) -> Result<(), Box<dyn std::error::Error>> {
        let mut redis_conn = self.redis_client.get_multiplexed_async_connection().await?;
        let key = progress_key(job_id);
        let previous_json: Option<String> = redis_conn.get(&key).await?;

        let mut progress = match previous_json {
            Some(json) => serde_json::from_str::<JobProgress>(&json)?,
            None => JobProgress::new(job_id, completed_steps, total_steps, phase),
        };

        let phase_changed = progress.phase != phase;
        progress.completed_steps = completed_steps;
        progress.total_steps = total_steps;
        progress.last_updated = Instant::now();
        progress.phase = phase;
        progress.recalculate_percentage();

        if phase_changed {
            self.publish_phase_transition(&mut redis_conn, &progress).await?;
        }

        if phase.is_terminal() || progress.progress_percentage >= 100 {
            // Keep terminal state in the event stream; the active key can be removed.
            if !phase.is_terminal() {
                progress.phase = JobPhase::Completed;
                self.publish_phase_transition(&mut redis_conn, &progress).await?;
            }
            redis_conn.del::<_, ()>(&key).await?;
        } else {
            redis_conn
                .set_ex::<_, _, ()>(
                    &key,
                    serde_json::to_string(&progress)?,
                    JOB_PROGRESS_TTL_SECONDS,
                )
                .await?;
        }

        Ok(())
    }

    async fn publish_phase_transition<C>(
        &self,
        redis_conn: &mut C,
        progress: &JobProgress,
    ) -> Result<(), Box<dyn std::error::Error>>
    where
        C: redis::aio::ConnectionLike + Send,
    {
        let event = JobProgressEvent {
            event_type: "phase_transition".to_owned(),
            job_id: progress.job_id.clone(),
            phase: progress.phase,
            completed_steps: progress.completed_steps,
            total_steps: progress.total_steps,
            progress_percentage: progress.progress_percentage,
            occurred_at_unix_ms: unix_time_ms(),
        };
        let channel = events_channel(&progress.job_id);
        let payload = serde_json::to_string(&event)?;
        let receivers: i64 = redis_conn.publish(channel, payload).await?;
        debug!(
            job_id = %progress.job_id,
            phase = %progress.phase,
            receivers,
            "Published job phase transition"
        );
        Ok(())
    }
}

/// Observable lifecycle phase for a job.
#[derive(Serialize, Deserialize, Debug, Clone, Copy, PartialEq, Eq, Default)]
#[serde(rename_all = "snake_case")]
pub enum JobPhase {
    #[default]
    Queued,
    Running,
    Processing,
    Finalizing,
    Completed,
    Failed,
    Cancelled,
}

impl JobPhase {
    fn is_terminal(self) -> bool {
        matches!(self, Self::Completed | Self::Failed | Self::Cancelled)
    }
}

impl std::fmt::Display for JobPhase {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(match self {
            Self::Queued => "queued",
            Self::Running => "running",
            Self::Processing => "processing",
            Self::Finalizing => "finalizing",
            Self::Completed => "completed",
            Self::Failed => "failed",
            Self::Cancelled => "cancelled",
        })
    }
}

/// Job progress structure stored in Redis.
#[derive(Serialize, Deserialize, Debug, Clone)]
pub struct JobProgress {
    pub job_id: String,
    pub completed_steps: u64,
    pub total_steps: Option<u64>,
    pub progress_percentage: u8,
    /// Phase is defaulted for compatibility with progress records created before this field existed.
    #[serde(default)]
    pub phase: JobPhase,
    #[serde(with = "instant_serde")]
    pub last_updated: Instant,
    #[serde(with = "instant_serde")]
    pub started_at: Instant,
}

impl JobProgress {
    fn new(
        job_id: &str,
        completed_steps: u64,
        total_steps: Option<u64>,
        phase: JobPhase,
    ) -> Self {
        let now = Instant::now();
        let mut progress = Self {
            job_id: job_id.to_owned(),
            completed_steps,
            total_steps,
            progress_percentage: 0,
            phase,
            last_updated: now,
            started_at: now,
        };
        progress.recalculate_percentage();
        progress
    }

    fn recalculate_percentage(&mut self) {
        self.progress_percentage = match self.total_steps {
            Some(total) if total > 0 => {
                ((self.completed_steps.min(total) as f64 / total as f64) * 100.0).round() as u8
            }
            _ => 0,
        };
    }
}

/// Message published to the Redis job event channel for each phase transition.
#[derive(Serialize, Deserialize, Debug, Clone, PartialEq, Eq)]
pub struct JobProgressEvent {
    pub event_type: String,
    pub job_id: String,
    pub phase: JobPhase,
    pub completed_steps: u64,
    pub total_steps: Option<u64>,
    pub progress_percentage: u8,
    pub occurred_at_unix_ms: u128,
}

fn progress_key(job_id: &str) -> String {
    format!("job:{job_id}:progress")
}

fn events_channel(job_id: &str) -> String {
    format!("job:{job_id}:{JOB_EVENTS_SUFFIX}")
}

fn unix_time_ms() -> u128 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis()
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

        let value = StringOrU64::deserialize(deserializer)?;
        let secs = match value {
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
    fn phase_transition_event_is_independent_of_percentage_delta() {
        let progress = JobProgress::new("job-123", 1, Some(100), JobPhase::Processing);
        let event = JobProgressEvent {
            event_type: "phase_transition".to_owned(),
            job_id: progress.job_id.clone(),
            phase: progress.phase,
            completed_steps: progress.completed_steps,
            total_steps: progress.total_steps,
            progress_percentage: progress.progress_percentage,
            occurred_at_unix_ms: 1_800_000_000_000,
        };

        let json = serde_json::to_value(event).expect("event should serialize");
        assert_eq!(json["event_type"], "phase_transition");
        assert_eq!(json["phase"], "processing");
        assert_eq!(json["progress_percentage"], 1);
    }

    #[test]
    fn progress_percentage_handles_missing_or_zero_total() {
        let mut progress = JobProgress::new("job-123", 3, None, JobPhase::Running);
        assert_eq!(progress.progress_percentage, 0);

        progress.total_steps = Some(0);
        progress.recalculate_percentage();
        assert_eq!(progress.progress_percentage, 0);
    }

    #[test]
    fn progress_percentage_is_bounded_to_one_hundred() {
        let progress = JobProgress::new("job-123", 12, Some(10), JobPhase::Running);
        assert_eq!(progress.progress_percentage, 100);
    }

    #[test]
    fn terminal_phases_are_identified() {
        assert!(JobPhase::Completed.is_terminal());
        assert!(JobPhase::Failed.is_terminal());
        assert!(JobPhase::Cancelled.is_terminal());
        assert!(!JobPhase::Processing.is_terminal());
    }

    #[test]
    fn redis_keys_are_consistent() {
        assert_eq!(progress_key("abc"), "job:abc:progress");
        assert_eq!(events_channel("abc"), "job:abc:events");
    }
}
