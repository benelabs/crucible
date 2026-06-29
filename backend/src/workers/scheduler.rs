//! SCHEDULER APPROACH: Option B — Custom Tokio interval loop with cron crate
//! Rationale: We require strict control over execution guarantees such as distributed locking, timeouts, retries, and history logging. A custom loop using `tokio::time::sleep_until` and the `cron` crate allows us to wrap the execution precisely in a Redis lock and accurately handle timeouts using `tokio::time::timeout`.

use std::str::FromStr;
use std::sync::Arc;
use std::time::Duration;

use async_trait::async_trait;
use chrono::Utc;
use cron::Schedule;
use sqlx::PgPool;
use tokio::sync::Mutex;
use tokio::task::JoinHandle;
use tokio_util::sync::CancellationToken;
use tracing::{debug, error, info, instrument, warn};

use crate::workers::error::{JobError, SchedulerError};
use crate::workers::job_history::{record_failure, record_start, record_success, record_timeout};

/// Context passed to a job handler during execution.
#[derive(Clone)]
pub struct JobContext {
    pub job_name: String,
    pub scheduled_at: chrono::DateTime<Utc>,
    pub pool: PgPool,
    pub redis: redis::Client,
}

/// Trait implemented by all schedulable jobs.
#[async_trait]
pub trait JobHandler: Send + Sync {
    async fn run(&self, ctx: JobContext) -> Result<(), JobError>;
}

/// Definition of a single job.
#[derive(Clone)]
pub struct JobDefinition {
    pub name: String,
    pub cron_expr: String,
    pub handler: Arc<dyn JobHandler>,
    pub timeout_secs: u64,
    pub max_retries: u8,
}

/// The main job scheduler.
pub struct Scheduler {
    jobs: Vec<JobDefinition>,
    pool: PgPool,
    redis: redis::Client,
    started: bool,
}

impl Scheduler {
    pub fn new(pool: PgPool, redis: redis::Client) -> Self {
        Self {
            jobs: Vec::new(),
            pool,
            redis,
            started: false,
        }
    }

    /// Registers a job before starting the scheduler.
    pub fn register(&mut self, job: JobDefinition) -> Result<(), SchedulerError> {
        if self.started {
            return Err(SchedulerError::AlreadyStarted);
        }
        // Verify the cron expression is valid
        Schedule::from_str(&job.cron_expr)
            .map_err(|e| SchedulerError::JobRegistrationFailed(e.to_string()))?;

        self.jobs.push(job);
        Ok(())
    }

    /// Starts the scheduler, spawning all job loops as Tokio tasks.
    /// Returns a `SchedulerHandle` to manage graceful shutdown.
    pub fn start(mut self) -> Result<SchedulerHandle, SchedulerError> {
        if self.started {
            return Err(SchedulerError::AlreadyStarted);
        }
        self.started = true;

        let cancel_token = CancellationToken::new();
        let mut task_handles = Vec::new();

        for job in self.jobs {
            let cancel_token = cancel_token.clone();
            let pool = self.pool.clone();
            let redis = self.redis.clone();

            let handle = tokio::spawn(async move {
                run_job_loop(job, pool, redis, cancel_token).await;
            });
            task_handles.push(handle);
        }

        Ok(SchedulerHandle {
            cancel_token,
            task_handles: Arc::new(Mutex::new(task_handles)),
        })
    }
}

/// A handle to the running scheduler, allowing for graceful shutdown.
#[derive(Clone)]
pub struct SchedulerHandle {
    cancel_token: CancellationToken,
    task_handles: Arc<Mutex<Vec<JoinHandle<()>>>>,
}

impl SchedulerHandle {
    /// Signals all job loops to stop and waits for them to finish cleanly.
    pub async fn shutdown(self) -> Result<(), SchedulerError> {
        info!("Initiating scheduler graceful shutdown...");
        self.cancel_token.cancel();

        let mut handles = self.task_handles.lock().await;
        for handle in handles.drain(..) {
            // Wait for the task to finish, ignoring cancel errors
            let _ = handle.await;
        }

        info!("Scheduler stopped successfully");
        Ok(())
    }
}

/// The core loop for a single registered job.
#[instrument(skip(job, pool, redis, cancel_token), fields(job_name = %job.name))]
async fn run_job_loop(
    job: JobDefinition,
    pool: PgPool,
    redis: redis::Client,
    cancel_token: CancellationToken,
) {
    let schedule =
        Schedule::from_str(&job.cron_expr).expect("cron expression must be pre-validated");

    loop {
        // Calculate the next execution time
        let now = Utc::now();
        let next_tick = match schedule.upcoming(Utc).next() {
            Some(time) => time,
            None => {
                warn!("No upcoming execution time found for job");
                break;
            }
        };

        let duration_until_next = match (next_tick - now).to_std() {
            Ok(duration) => duration,
            Err(_) => {
                // Next tick is in the past, meaning we're falling behind. Tick immediately.
                Duration::from_millis(0)
            }
        };

        debug!("Next execution scheduled in {:?}", duration_until_next);

        // Sleep until the next tick, or break if cancelled
        tokio::select! {
            _ = tokio::time::sleep(duration_until_next) => {}
            _ = cancel_token.cancelled() => {
                info!("Job loop cancelled, shutting down");
                break;
            }
        }

        // We've woken up for the tick. Try to acquire the distributed lock.
        // Lock TTL is timeout_secs + 5.
        // WHY: If the lock TTL exactly matched the job timeout, they could expire at the same millisecond.
        // If the lock expires automatically while the job is timing out, another instance could
        // acquire the lock and start the job before the current instance finishes logging the timeout
        // to PostgreSQL, potentially causing overlapping state or duplicate runs.
        let lock_key = format!("{}:lock", job.name);
        let lock_ttl_ms = (job.timeout_secs + 5) * 1000;

        let mut conn = match redis.get_async_connection().await {
            Ok(conn) => conn,
            Err(e) => {
                error!("Failed to connect to Redis for lock acquisition: {}", e);
                continue; // Try again next tick
            }
        };

        let acquired: Option<String> = match redis::cmd("SET")
            .arg(&lock_key)
            .arg("1")
            .arg("NX")
            .arg("PX")
            .arg(lock_ttl_ms)
            .query_async::<Option<String>>(&mut conn)
            .await
        {
            Ok(v) => v,
            Ok(Some(v)) if v == "OK" => true,
            Ok(_) => false,
            Err(e) => {
                error!("Failed to acquire distributed lock: {}", e);
                continue;
            }
        };

        if acquired.as_deref() != Some("OK") {
            debug!("Another instance is running this tick, skipping");
            continue;
        }

        let ctx = JobContext {
            job_name: job.name.clone(),
            scheduled_at: next_tick,
            pool: pool.clone(),
            redis: redis.clone(),
        };

        // Execute the job
        execute_job_with_retries(&job, ctx).await;

        // Release the lock manually (optional, since it has a TTL, but good practice if it finishes early)
        let _: redis::RedisResult<()> = redis::cmd("DEL")
            .arg(&lock_key)
            .query_async(&mut conn)
            .await;
    }
}

async fn execute_job_with_retries(job: &JobDefinition, ctx: JobContext) {
    let mut attempt = 0;
    let max_attempts = job.max_retries + 1;
    let start_time = tokio::time::Instant::now();

    // Record start in db
    let run_id = match record_start(&ctx.pool, &job.name).await {
        Ok(id) => id,
        Err(e) => {
            error!("Failed to record job start in DB: {}", e);
            return;
        }
    };

    let handler_clone = job.handler.clone();

    loop {
        attempt += 1;
        let ctx_clone = ctx.clone();

        // Spawn a blocking task to handle panics safely.
        // Or rather, we spawn a standard tokio task to catch unwinds.
        let job_name = job.name.clone();
        let handler = handler_clone.clone();

        let task = tokio::spawn(async move { handler.run(ctx_clone).await });

        let timeout_duration = Duration::from_secs(job.timeout_secs);
        let result = tokio::time::timeout(timeout_duration, task).await;

        let duration_ms = start_time.elapsed().as_millis() as i64;

        match result {
            Ok(Ok(Ok(()))) => {
                info!("Job completed successfully on attempt {}", attempt);
                if let Err(e) = record_success(&ctx.pool, run_id, duration_ms).await {
                    error!("Failed to record job success: {}", e);
                }
                break;
            }
            Ok(Ok(Err(e))) => {
                warn!("Job failed on attempt {}/{}: {}", attempt, max_attempts, e);
                if attempt >= max_attempts {
                    error!("Job failed permanently after {} attempts", max_attempts);
                    if let Err(db_e) =
                        record_failure(&ctx.pool, run_id, &e.to_string(), duration_ms).await
                    {
                        error!("Failed to record job failure: {}", db_e);
                    }
                    break;
                }
            }
            Ok(Err(join_err)) => {
                let err_msg = if join_err.is_panic() {
                    "Job handler panicked".to_string()
                } else {
                    format!("Job task cancelled or failed: {}", join_err)
                };
                warn!(
                    "Job panicked/cancelled on attempt {}/{}: {}",
                    attempt, max_attempts, err_msg
                );

                if attempt >= max_attempts {
                    error!(
                        "Job failed permanently after panicking {} times",
                        max_attempts
                    );
                    if let Err(db_e) =
                        record_failure(&ctx.pool, run_id, &err_msg, duration_ms).await
                    {
                        error!("Failed to record job failure: {}", db_e);
                    }
                    break;
                }
            }
            Err(_) => {
                warn!("Job timed out on attempt {}/{}", attempt, max_attempts);

                if attempt >= max_attempts {
                    error!("Job timed out permanently after {} attempts", max_attempts);
                    if let Err(db_e) = record_timeout(&ctx.pool, run_id, duration_ms).await {
                        error!("Failed to record job timeout: {}", db_e);
                    }
                    break;
                }
            }
        }

        // Backoff slightly before retry (optional, but good practice)
        tokio::time::sleep(Duration::from_secs(2)).await;
    }
}
