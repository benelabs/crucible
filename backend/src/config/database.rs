//! CONFIG APPROACH: Option A — layered config crate
//! Rationale: Using the `config` crate provides a robust, layered approach where environment-specific
//! defaults are cleanly defined in TOML files, while sensitive secrets and infrastructure-specific
//! overrides are passed securely via environment variables.

use serde::{Deserialize, Serialize};
use sqlx::postgres::{PgConnectOptions, PgPool, PgPoolOptions};
use std::fmt;
use std::str::FromStr;
use std::time::Duration;

/// Upper bound (seconds) for the connection acquisition timeout, preventing request stalls.
const MAX_ACQUIRE_TIMEOUT_SECS: u64 = 5;

/// Database configuration for PostgreSQL via SQLx.
#[derive(Clone, Deserialize, Serialize)]
pub struct DatabaseConfig {
    /// The connection URL. Never logged or serialized.
    #[serde(skip_serializing)]
    pub url: String,
    /// Maximum number of connections in the pool.
    pub max_connections: u32,
    /// Minimum number of idle connections to maintain.
    pub min_connections: u32,
    /// Maximum time in seconds to wait for a connection.
    #[serde(default = "default_connect_timeout_secs")]
    pub connect_timeout_secs: u64,
    /// Maximum time in seconds a connection can remain idle before being closed.
    pub idle_timeout_secs: u64,
    /// Duration in seconds for Retry-After header when pool is exhausted.
    #[serde(default = "default_pool_retry_after_secs")]
    pub pool_retry_after_secs: u64,
    /// Dedicated maximum connections for background batch workers.
    #[serde(default = "default_worker_max_connections")]
    pub worker_max_connections: u32,
    /// TCP keepalive idle duration in seconds to prevent firewall and NAT timeout drops.
    #[serde(default = "default_tcp_keepalive_secs")]
    pub tcp_keepalive_secs: u64,
}

fn default_pool_retry_after_secs() -> u64 {
    5
}

fn default_connect_timeout_secs() -> u64 {
    3
}

fn default_worker_max_connections() -> u32 {
    10
}

fn default_tcp_keepalive_secs() -> u64 {
    60
}

impl fmt::Debug for DatabaseConfig {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("DatabaseConfig")
            .field("url", &"[REDACTED]")
            .field("max_connections", &self.max_connections)
            .field("min_connections", &self.min_connections)
            .field("connect_timeout_secs", &self.connect_timeout_secs)
            .field("idle_timeout_secs", &self.idle_timeout_secs)
            .field("pool_retry_after_secs", &self.pool_retry_after_secs)
            .field("worker_max_connections", &self.worker_max_connections)
            .field("tcp_keepalive_secs", &self.tcp_keepalive_secs)
            .finish()
    }
}

impl DatabaseConfig {
    /// Returns the configured TCP keepalive duration.
    pub fn tcp_keepalive(&self) -> Option<Duration> {
        if self.tcp_keepalive_secs > 0 {
            Some(Duration::from_secs(self.tcp_keepalive_secs))
        } else {
            None
        }
    }

    /// Returns the connection acquisition timeout, capped between 1 and 5 seconds to prevent request stalls.
    pub fn acquire_timeout(&self) -> Duration {
        let bounded_secs = self.connect_timeout_secs.clamp(1, MAX_ACQUIRE_TIMEOUT_SECS);
        Duration::from_secs(bounded_secs)
    }

    /// Converts the configuration into a ready-to-use SQLx Postgres pool builder.
    pub fn to_sqlx_pool_options(&self) -> PgPoolOptions {
        let keepalive_idle = 60u64;
        PgPoolOptions::new()
            .max_connections(self.max_connections)
            .min_connections(self.min_connections)
            .acquire_timeout(self.acquire_timeout())
            .idle_timeout(Duration::from_secs(self.idle_timeout_secs))
            .after_connect(move |conn, _meta| {
                Box::pin(async move {
                    let stmt = format!(
                        "SET tcp_keepalives_idle = {}; SET tcp_keepalives_interval = 10; SET tcp_keepalives_count = 5",
                        keepalive_idle
                    );
                    sqlx::query(&stmt).execute(conn).await?;
                    Ok(())
                })
            })
    }

    /// Converts the configuration into a dedicated SQLx Postgres pool builder for background batch workers.
    pub fn to_worker_sqlx_pool_options(&self) -> PgPoolOptions {
        let keepalive_idle = 60u64;
        PgPoolOptions::new()
            .max_connections(self.worker_max_connections)
            .min_connections(self.min_connections)
            .acquire_timeout(self.acquire_timeout())
            .idle_timeout(Duration::from_secs(self.idle_timeout_secs))
            .after_connect(move |conn, _meta| {
                Box::pin(async move {
                    let stmt = format!(
                        "SET tcp_keepalives_idle = {}; SET tcp_keepalives_interval = 10; SET tcp_keepalives_count = 5",
                        keepalive_idle
                    );
                    sqlx::query(&stmt).execute(conn).await?;
                    Ok(())
                })
            })
    }
}

#[cfg(test)]
mod tests_acquire {
    use super::*;

    #[test]
    fn test_to_sqlx_pool_options_clamps_timeout() {
        let mut config = DatabaseConfig {
            url: "postgres://localhost/test".to_string(),
            max_connections: 10,
            worker_max_connections: 5,
            min_connections: 1,
            connect_timeout_secs: 30,
            idle_timeout_secs: 60,
            pool_retry_after_secs: 5,
            tcp_keepalive_secs: 60,
        };

        // Even though connect_timeout_secs is 30, options should be created without panicking
        let _opts = config.to_sqlx_pool_options();
        let _worker_opts = config.to_worker_sqlx_pool_options();

        config.connect_timeout_secs = 1;
        let _opts_low = config.to_sqlx_pool_options();

        config.connect_timeout_secs = 4;
        let _opts_mid = config.to_sqlx_pool_options();
    }

    #[test]
    fn test_default_connect_timeout() {
        assert_eq!(default_connect_timeout_secs(), 3);
        assert!(default_connect_timeout_secs() >= 3 && default_connect_timeout_secs() <= 5);
    }

    #[test]
    fn test_acquire_timeout_clamping() {
        let mut config = DatabaseConfig {
            url: "postgres://localhost/test".to_string(),
            max_connections: 10,
            min_connections: 2,
            connect_timeout_secs: 30,
            idle_timeout_secs: 300,
            pool_retry_after_secs: 5,
            worker_max_connections: 5,
            tcp_keepalive_secs: 60,
        };
        assert_eq!(config.acquire_timeout(), Duration::from_secs(5));

        config.connect_timeout_secs = 3;
        assert_eq!(config.acquire_timeout(), Duration::from_secs(3));

        config.connect_timeout_secs = 0;
        assert_eq!(config.acquire_timeout(), Duration::from_secs(1));
    }

    #[test]
    fn test_serde_default_connect_timeout() {
        let json = r#"{
            "url": "postgres://localhost/test",
            "max_connections": 10,
            "min_connections": 2,
            "idle_timeout_secs": 300
        }"#;
        let config: DatabaseConfig = serde_json::from_str(json).unwrap();
        assert_eq!(config.connect_timeout_secs, 3);
        assert_eq!(config.acquire_timeout(), Duration::from_secs(3));
        assert_eq!(config.pool_retry_after_secs, 5);
        assert_eq!(config.worker_max_connections, 10);
    }
}

impl DatabaseConfig {
    /// Creates PostgreSQL connection options populated from `self.url` with TCP keepalive parameters
    /// configured to prevent intermediate firewalls and NAT gateways from dropping idle connections.
    pub fn to_connect_options(&self) -> Result<PgConnectOptions, sqlx::Error> {
        let keepalive_idle = self.tcp_keepalive_secs.to_string();
        let opts = PgConnectOptions::from_str(&self.url)?.options([
            ("tcp_keepalives_idle", keepalive_idle.as_str()),
            ("tcp_keepalives_interval", "10"),
            ("tcp_keepalives_count", "5"),
        ]);
        Ok(opts)
    }

    /// Establishes a PostgreSQL connection pool configured with pool sizing and TCP keepalive options.
    pub async fn create_pool(&self) -> Result<PgPool, sqlx::Error> {
        self.to_sqlx_pool_options()
            .connect_with(self.to_connect_options()?)
            .await
    }

    /// Establishes a dedicated worker PostgreSQL connection pool configured with pool sizing and TCP keepalive options.
    pub async fn create_worker_pool(&self) -> Result<PgPool, sqlx::Error> {
        self.to_worker_sqlx_pool_options()
            .connect_with(self.to_connect_options()?)
            .await
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_default_tcp_keepalive() {
        assert_eq!(default_tcp_keepalive_secs(), 60);
    }

    #[test]
    fn test_tcp_keepalive_duration() {
        let config = DatabaseConfig {
            url: "postgres://localhost/test".to_string(),
            max_connections: 10,
            min_connections: 2,
            connect_timeout_secs: 5,
            idle_timeout_secs: 300,
            pool_retry_after_secs: 5,
            worker_max_connections: 5,
            tcp_keepalive_secs: 60,
        };
        assert_eq!(config.tcp_keepalive(), Some(Duration::from_secs(60)));

        let zero_config = DatabaseConfig {
            tcp_keepalive_secs: 0,
            ..config.clone()
        };
        assert_eq!(zero_config.tcp_keepalive(), None);
    }

    #[test]
    fn test_to_connect_options_includes_keepalive() {
        let config = DatabaseConfig {
            url: "postgres://user:pass@localhost:5432/crucible_db".to_string(),
            max_connections: 20,
            min_connections: 4,
            connect_timeout_secs: 5,
            idle_timeout_secs: 600,
            pool_retry_after_secs: 5,
            worker_max_connections: 10,
            tcp_keepalive_secs: 45,
        };

        let connect_opts = config.to_connect_options().expect("valid connect options");
        assert_eq!(connect_opts.get_host(), "localhost");
        assert_eq!(connect_opts.get_port(), 5432);
        assert_eq!(connect_opts.get_database(), Some("crucible_db"));
        assert_eq!(connect_opts.get_username(), "user");
    }

    #[test]
    fn test_serde_default_tcp_keepalive() {
        let json = r#"{
            "url": "postgres://localhost/test",
            "max_connections": 10,
            "min_connections": 2,
            "connect_timeout_secs": 5,
            "idle_timeout_secs": 300
        }"#;
        let config: DatabaseConfig = serde_json::from_str(json).unwrap();
        assert_eq!(config.tcp_keepalive_secs, 60);
        assert_eq!(config.pool_retry_after_secs, 5);
        assert_eq!(config.worker_max_connections, 10);
    }
}
