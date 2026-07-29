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
