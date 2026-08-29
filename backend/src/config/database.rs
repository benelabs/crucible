//! CONFIG APPROACH: Option A — layered config crate
//! Rationale: Using the `config` crate provides a robust, layered approach where environment-specific
//! defaults are cleanly defined in TOML files, while sensitive secrets and infrastructure-specific
//! overrides are passed securely via environment variables.

use serde::{Deserialize, Serialize};
use sqlx::postgres::PgPoolOptions;
use std::fmt;
use std::time::Duration;

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
    pub connect_timeout_secs: u64,
    /// Maximum time in seconds a connection can remain idle before being closed.
    pub idle_timeout_secs: u64,
    /// Duration in seconds for Retry-After header when pool is exhausted.
    #[serde(default = "default_pool_retry_after_secs")]
    pub pool_retry_after_secs: u64,
}

fn default_pool_retry_after_secs() -> u64 {
    5
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
            .finish()
    }
}

impl DatabaseConfig {
    /// Converts the configuration into a ready-to-use SQLx Postgres pool builder.
    pub fn to_sqlx_pool_options(&self) -> PgPoolOptions {
        PgPoolOptions::new()
            .max_connections(self.max_connections)
            .min_connections(self.min_connections)
            .acquire_timeout(Duration::from_secs(self.connect_timeout_secs))
            .idle_timeout(Duration::from_secs(self.idle_timeout_secs))
    }
}
