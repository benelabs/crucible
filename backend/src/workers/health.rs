use redis::{AsyncCommands, Client};
use serde::{Deserialize, Serialize};
use sqlx::PgPool;
use std::sync::Arc;
use std::time::{Duration, Instant};
use tracing::{debug, error, info, warn};

/// Worker health monitor that tracks and reports worker status
#[derive(Debug, Clone)]
pub struct WorkerHealthMonitor {
    db_pool: PgPool,
    redis_client: Client,
    check_interval: Duration,
    health_ttl: Duration,
}

impl WorkerHealthMonitor {
    /// Create a new worker health monitor
    pub fn new(
        db_pool: PgPool,
        redis_client: Client,
        check_interval: Duration,
        health_ttl: Duration,
    ) -> Self {
        Self {
            db_pool,
            redis_client,
            check_interval,
            health_ttl,
        }
    }

    /// Start the health monitoring process
    pub async fn start(&self) -> Result<(), Box<dyn std::error::Error>> {
        info!("Starting worker health monitor...");

        loop {
            if let Err(e) = self.check_health().await {
                error!("Failed to check worker health: {}", e);
            }

            // Wait for the configured interval before next health check
            tokio::time::sleep(self.check_interval).await;
        }
    }

    /// Check health of all workers
    async fn check_health(&self) -> Result<(), Box<dyn std::error::Error>> {
        info!("Checking worker health...");

        // Get Redis connection
        let mut redis_conn = self.redis_client.get_async_connection().await?;

        // Get list of worker health keys
        let health_keys: Vec<String> = redis_conn.keys("worker:*:health").await?;

        for key in health_keys {
            // Get current health data
            let health_data: Option<String> = redis_conn.get(&key).await?;

            if let Some(json) = health_data {
                match serde_json::from_str::<WorkerHealth>(&json) {
                    Ok(mut health) => {
                        // Update last checked timestamp
                        health.last_checked = Instant::now();

                        // Calculate health status based on last heartbeat
                        let elapsed = health.last_heartbeat.elapsed();
                        health.is_healthy = elapsed < Duration::from_secs(30); // 30 seconds threshold

                        // Store updated health status
                        redis_conn
                            .set_ex::<_, _, ()>(
                                &key,
                                serde_json::to_string(&health)?,
                                self.health_ttl.as_secs(),
                            )
                            .await?;

                        debug!(
                            "Updated health for worker: {} (healthy: {})",
                            health.worker_id, health.is_healthy
                        );
                    }
                    Err(e) => {
                        error!("Failed to parse health data for {}: {}", key, e);
                    }
                }
            }
        }

        info!("Worker health check completed");
        Ok(())
    }

    /// Report worker heartbeat
    pub async fn report_heartbeat(
        &self,
        worker_id: &str,
    ) -> Result<(), Box<dyn std::error::Error>> {
        let mut redis_conn = self.redis_client.get_async_connection().await?;
        let key = format!("worker:{}:health", worker_id);

        let health = WorkerHealth {
            worker_id: worker_id.to_string(),
            last_heartbeat: Instant::now(),
            last_checked: Instant::now(),
            is_healthy: true,
            uptime_seconds: 0,
        };

        redis_conn
            .set_ex::<_, _, ()>(
                &key,
                serde_json::to_string(&health)?,
                self.health_ttl.as_secs(),
            )
            .await?;

        Ok(())
    }

    /// Get health status for a specific worker
    pub async fn get_worker_health(
        &self,
        worker_id: &str,
    ) -> Result<Option<WorkerHealth>, Box<dyn std::error::Error>> {
        let mut redis_conn = self.redis_client.get_async_connection().await?;
        let key = format!("worker:{}:health", worker_id);

        let health_data: Option<String> = redis_conn.get(&key).await?;

        match health_data {
            Some(json) => {
                let health: WorkerHealth = serde_json::from_str(&json)?;
                Ok(Some(health))
            }
            None => Ok(None),
        }
    }
}

/// Worker health structure
#[derive(Serialize, Deserialize, Debug, Clone)]
pub struct WorkerHealth {
    pub worker_id: String,
    #[serde(with = "instant_serde")]
    pub last_heartbeat: std::time::Instant,
    #[serde(with = "instant_serde")]
    pub last_checked: std::time::Instant,
    pub is_healthy: bool,
    pub uptime_seconds: u64,
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
