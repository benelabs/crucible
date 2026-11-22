//! CONFIG APPROACH: Option A — layered config crate
//! Rationale: Using the `config` crate provides a robust, layered approach where environment-specific
//! defaults are cleanly defined in TOML files, while sensitive secrets and infrastructure-specific
//! overrides are passed securely via environment variables.

use redis::IntoConnectionInfo;
use serde::{Deserialize, Serialize};
use std::fmt;

/// Redis configuration for caching and optional job queues.
#[derive(Clone, Deserialize, Serialize)]
pub struct RedisConfig {
    /// The primary Redis connection URL (used for cache). Never logged or serialized.
    #[serde(skip_serializing)]
    pub url: String,
    /// Optional separate Redis URL for a job queue. Falls back to `url` if not provided.
    #[serde(skip_serializing)]
    pub job_queue_url: Option<String>,
    /// Number of connections in the Redis pool.
    pub pool_size: u32,
    /// Connection timeout in milliseconds.
    pub connection_timeout_ms: u64,
    /// Maximum number of connection retries.
    pub max_retries: u32,
    /// Startup timeout in milliseconds before giving up retrying connection manager creation.
    #[serde(default = "default_startup_timeout_ms")]
    pub startup_timeout_ms: u64,
}

fn default_startup_timeout_ms() -> u64 {
    30_000
}

impl fmt::Debug for RedisConfig {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("RedisConfig")
            .field("url", &"[REDACTED]")
            .field(
                "job_queue_url",
                if self.job_queue_url.is_some() {
                    &Some("[REDACTED]")
                } else {
                    &None::<&str>
                },
            )
            .field("pool_size", &self.pool_size)
            .field("connection_timeout_ms", &self.connection_timeout_ms)
            .field("max_retries", &self.max_retries)
            .field("startup_timeout_ms", &self.startup_timeout_ms)
            .finish()
    }
}

impl RedisConfig {
    /// Connects to Redis via a connection manager with exponential retry until startup timeout.
    pub async fn connect_manager_with_retry(
        &self,
        client: &redis::Client,
    ) -> Result<redis::aio::ConnectionManager, redis::RedisError> {
        let deadline =
            std::time::Instant::now() + std::time::Duration::from_millis(self.startup_timeout_ms);
        let mut delay = std::time::Duration::from_millis(100);
        let max_delay = std::time::Duration::from_secs(5);
        let timeout = std::time::Duration::from_millis(self.connection_timeout_ms);
        let mut attempt = 0;

        loop {
            attempt += 1;
            tracing::info!(
                "Attempting to connect to Redis connection manager (attempt {})",
                attempt
            );

            let conn_fut = redis::aio::ConnectionManager::new(client.clone());
            match tokio::time::timeout(timeout, conn_fut).await {
                Ok(Ok(conn)) => {
                    tracing::info!("Successfully connected to Redis connection manager");
                    return Ok(conn);
                }
                Ok(Err(e)) => {
                    tracing::warn!("Failed to connect to Redis on attempt {}: {:?}", attempt, e);
                    if std::time::Instant::now() >= deadline
                        || (self.max_retries > 0 && attempt >= self.max_retries)
                    {
                        return Err(e);
                    }
                }
                Err(_) => {
                    tracing::warn!(
                        "Connection to Redis timed out on attempt {} (limit: {}ms)",
                        attempt,
                        self.connection_timeout_ms
                    );
                    if std::time::Instant::now() >= deadline
                        || (self.max_retries > 0 && attempt >= self.max_retries)
                    {
                        return Err(redis::RedisError::from((
                            redis::ErrorKind::IoError,
                            "Connection timed out",
                        )));
                    }
                }
            }

            tokio::time::sleep(delay).await;
            delay = std::cmp::min(delay * 2, max_delay);
        }
    }

    /// Converts the primary config to Redis connection info.
    pub fn to_connection_info(
        &self,
    ) -> Result<redis::ConnectionInfo, crate::config::error::ConfigError> {
        self.url.as_str().into_connection_info().map_err(|e| {
            crate::config::error::ConfigError::InvalidValue {
                field: "redis.url".to_string(),
                reason: e.to_string(),
            }
        })
    }

    /// Converts the job queue config to Redis connection info.
    pub fn to_job_queue_connection_info(
        &self,
    ) -> Result<redis::ConnectionInfo, crate::config::error::ConfigError> {
        let url = self.job_queue_url.as_ref().unwrap_or(&self.url);
        url.as_str().into_connection_info().map_err(|e| {
            crate::config::error::ConfigError::InvalidValue {
                field: "redis.job_queue_url".to_string(),
                reason: e.to_string(),
            }
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_redis_config_default_startup_timeout() {
        let config = RedisConfig {
            url: "redis://127.0.0.1:6379".to_string(),
            job_queue_url: None,
            pool_size: 10,
            connection_timeout_ms: 500,
            max_retries: 2,
            startup_timeout_ms: default_startup_timeout_ms(),
        };

        assert_eq!(config.startup_timeout_ms, 30_000);
        let debug_str = format!("{:?}", config);
        assert!(debug_str.contains("[REDACTED]"));
        assert!(debug_str.contains("startup_timeout_ms: 30000"));
    }

    #[tokio::test]
    async fn test_connect_manager_with_retry_fails_gracefully_on_invalid_url() {
        let config = RedisConfig {
            url: "redis://127.0.0.1:1".to_string(),
            job_queue_url: None,
            pool_size: 1,
            connection_timeout_ms: 100,
            max_retries: 1,
            startup_timeout_ms: 200,
        };

        let client = redis::Client::open(config.url.clone()).unwrap();
        let res = config.connect_manager_with_retry(&client).await;
        assert!(res.is_err());
    }

    #[test]
    fn redis_config_custom_startup_timeout() {
        let json_str = r#"{
            "url": "redis://127.0.0.1:6379",
            "pool_size": 10,
            "connection_timeout_ms": 2000,
            "max_retries": 10,
            "startup_timeout_ms": 60000
        }"#;

        let config: RedisConfig = serde_json::from_str(json_str).unwrap();
        assert_eq!(config.startup_timeout_ms, 60_000);
    }
}
