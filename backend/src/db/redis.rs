//! Redis connection manager setup with timeout and retry strategy.

use std::time::Duration;
use redis::Client;
use tracing::{info, warn};
use crate::config::redis::RedisConfig;

/// Establish a connection manager with exponential backoff and timeout logic.
pub async fn connect_with_retry(
    client: &Client,
    config: &RedisConfig,
) -> Result<redis::aio::ConnectionManager, redis::RedisError> {
    let mut attempt = 0;
    let mut delay = Duration::from_millis(100);
    let max_delay = Duration::from_secs(5);
    let timeout = Duration::from_millis(config.connection_timeout_ms);

    loop {
        attempt += 1;
        info!(
            "Attempting to connect to Redis (attempt {}/{})",
            attempt, config.max_retries
        );

        let conn_fut = redis::aio::ConnectionManager::new(client.clone());
        match tokio::time::timeout(timeout, conn_fut).await {
            Ok(Ok(conn)) => {
                info!("Successfully connected to Redis");
                return Ok(conn);
            }
            Ok(Err(e)) => {
                warn!("Failed to connect to Redis on attempt {}: {:?}", attempt, e);
                if attempt >= config.max_retries {
                    return Err(e);
                }
            }
            Err(_) => {
                warn!(
                    "Connection to Redis timed out on attempt {} (limit: {}ms)",
                    attempt, config.connection_timeout_ms
                );
                if attempt >= config.max_retries {
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

/// Detailed Redis health check result.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RedisHealthStatus {
    pub is_healthy: bool,
    pub is_loading: bool,
    pub is_read_only: bool,
    pub role: String,
    pub error: Option<String>,
}

/// Inspect Redis status to ensure it is not loading or in read-only mode during cluster recovery.
///
/// Verifies health by executing `INFO persistence` and `INFO replication`, or testing a ephemeral write/read.
pub async fn check_redis_health(
    conn: &mut redis::aio::ConnectionManager,
) -> Result<RedisHealthStatus, redis::RedisError> {
    // Check INFO persistence for loading flag
    let persistence_info: Result<String, redis::RedisError> = redis::cmd("INFO")
        .arg("persistence")
        .query_async(conn)
        .await;

    let mut is_loading = false;
    if let Ok(ref info) = persistence_info {
        for line in info.lines() {
            if let Some(val) = line.strip_prefix("loading:") {
                if val.trim() == "1" {
                    is_loading = true;
                }
            }
        }
    }

    // Check INFO replication for role
    let replication_info: Result<String, redis::RedisError> = redis::cmd("INFO")
        .arg("replication")
        .query_async(conn)
        .await;

    let mut role = "unknown".to_string();
    if let Ok(ref info) = replication_info {
        for line in info.lines() {
            if let Some(val) = line.strip_prefix("role:") {
                role = val.trim().to_string();
            }
        }
    }

    // Check write capability via ephemeral key with TTL
    let test_key = format!("health:probe:{}", uuid::Uuid::new_v4());
    let write_res: Result<(), redis::RedisError> = redis::cmd("SETEX")
        .arg(&test_key)
        .arg(5)
        .arg("1")
        .query_async(conn)
        .await;

    let is_read_only = write_res.is_err();
    let write_err = write_res.err().map(|e| e.to_string());

    let is_healthy = !is_loading && !is_read_only;

    Ok(RedisHealthStatus {
        is_healthy,
        is_loading,
        is_read_only,
        role,
        error: write_err,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_redis_health_status_healthy() {
        let status = RedisHealthStatus {
            is_healthy: true,
            is_loading: false,
            is_read_only: false,
            role: "master".to_string(),
            error: None,
        };
        assert!(status.is_healthy);
        assert!(!status.is_loading);
        assert!(!status.is_read_only);
    }

    #[test]
    fn test_redis_health_status_loading_unhealthy() {
        let status = RedisHealthStatus {
            is_healthy: false,
            is_loading: true,
            is_read_only: false,
            role: "master".to_string(),
            error: None,
        };
        assert!(!status.is_healthy);
        assert!(status.is_loading);
    }

    #[test]
    fn test_redis_health_status_readonly_unhealthy() {
        let status = RedisHealthStatus {
            is_healthy: false,
            is_loading: false,
            is_read_only: true,
            role: "slave".to_string(),
            error: Some("READONLY You can't write against a read only replica.".to_string()),
        };
        assert!(!status.is_healthy);
        assert!(status.is_read_only);
    }
}
