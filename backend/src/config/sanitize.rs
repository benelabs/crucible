//! Configuration sanitization for safe exposure via diagnostics endpoints.
//!
//! This module provides utilities to sanitize sensitive configuration fields
//! (secrets, credentials, DSNs, private endpoints) before exposing config to
//! operators or logging systems.

use crate::config::{CorsConfig, DatabaseConfig, ObservabilityConfig, RedisConfig, ServerConfig};
use serde::{Deserialize, Serialize};
use std::fmt;

/// A sanitized, secret-redacted version of the application configuration.
///
/// This struct mirrors [`crate::config::AppConfig`] but redacts all sensitive fields
/// at serialization time. It's safe to expose through admin endpoints or include in logs.
#[derive(Clone, Debug, Deserialize, Serialize)]
pub struct SanitizedConfig {
    pub server: SanitizedServerConfig,
    pub database: SanitizedDatabaseConfig,
    pub redis: SanitizedRedisConfig,
    pub observability: ObservabilityConfig,
    #[serde(default)]
    pub cors: CorsConfig,
}

/// Sanitized server configuration. Redacts TLS private key paths.
#[derive(Clone, Debug, Deserialize, Serialize)]
pub struct SanitizedServerConfig {
    pub host: String,
    pub port: u16,
    pub request_timeout_ms: u64,
    pub max_connections: usize,
    pub tls: Option<SanitizedTlsConfig>,
}

/// Sanitized TLS configuration. Redacts private key path.
#[derive(Clone, Debug, Deserialize, Serialize)]
pub struct SanitizedTlsConfig {
    pub cert_path: String,
    /// Private key path is redacted for security.
    pub key_path: String,
}

impl SanitizedTlsConfig {
    fn from_original(tls: &crate::config::server::TlsConfig) -> Self {
        Self {
            cert_path: tls.cert_path.clone(),
            key_path: "[REDACTED]".to_string(),
        }
    }
}

/// Sanitized database configuration. Redacts connection URL.
#[derive(Clone, Debug, Deserialize, Serialize)]
pub struct SanitizedDatabaseConfig {
    /// Database connection URL is redacted for security.
    pub url: String,
    pub max_connections: u32,
    pub min_connections: u32,
    pub connect_timeout_secs: u64,
    pub idle_timeout_secs: u64,
}

impl SanitizedDatabaseConfig {
    fn from_original(db: &DatabaseConfig) -> Self {
        Self {
            url: "[REDACTED]".to_string(),
            max_connections: db.max_connections,
            min_connections: db.min_connections,
            connect_timeout_secs: db.connect_timeout_secs,
            idle_timeout_secs: db.idle_timeout_secs,
        }
    }
}

/// Sanitized Redis configuration. Redacts connection URLs.
#[derive(Clone, Debug, Deserialize, Serialize)]
pub struct SanitizedRedisConfig {
    /// Primary Redis connection URL is redacted for security.
    pub url: String,
    /// Job queue Redis URL is redacted for security (if present).
    pub job_queue_url: Option<String>,
    pub pool_size: u32,
    pub connection_timeout_ms: u64,
    pub max_retries: u32,
}

impl SanitizedRedisConfig {
    fn from_original(redis: &RedisConfig) -> Self {
        Self {
            url: "[REDACTED]".to_string(),
            job_queue_url: redis.job_queue_url.as_ref().map(|_| "[REDACTED]".to_string()),
            pool_size: redis.pool_size,
            connection_timeout_ms: redis.connection_timeout_ms,
            max_retries: redis.max_retries,
        }
    }
}

impl SanitizedServerConfig {
    fn from_original(server: &ServerConfig) -> Self {
        Self {
            host: server.host.clone(),
            port: server.port,
            request_timeout_ms: server.request_timeout_ms,
            max_connections: server.max_connections,
            tls: server.tls.as_ref().map(SanitizedTlsConfig::from_original),
        }
    }
}

/// Converts an [`crate::config::AppConfig`] to a sanitized representation,
/// redacting all sensitive fields.
///
/// # Example
/// ```ignore
/// let config = AppConfig::load(Environment::Production)?;
/// let sanitized = sanitize(&config);
/// let json = serde_json::to_string_pretty(&sanitized)?;
/// println!("{}", json);
/// ```
pub fn sanitize(config: &crate::config::AppConfig) -> SanitizedConfig {
    SanitizedConfig {
        server: SanitizedServerConfig::from_original(&config.server),
        database: SanitizedDatabaseConfig::from_original(&config.database),
        redis: SanitizedRedisConfig::from_original(&config.redis),
        observability: config.observability.clone(),
        cors: config.cors.clone(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::Environment;

    #[test]
    fn test_sanitize_redacts_database_url() {
        let config = crate::config::AppConfig::load(Environment::Development)
            .expect("Failed to load test config");
        let sanitized = sanitize(&config);

        assert_eq!(sanitized.database.url, "[REDACTED]");
        assert_ne!(sanitized.database.url, config.database.url);
    }

    #[test]
    fn test_sanitize_redacts_redis_url() {
        let config = crate::config::AppConfig::load(Environment::Development)
            .expect("Failed to load test config");
        let sanitized = sanitize(&config);

        assert_eq!(sanitized.redis.url, "[REDACTED]");
        assert_ne!(sanitized.redis.url, config.redis.url);
    }

    #[test]
    fn test_sanitize_redacts_tls_key_path() {
        let config = crate::config::AppConfig::load(Environment::Development)
            .expect("Failed to load test config");
        let sanitized = sanitize(&config);

        if let Some(tls) = &sanitized.server.tls {
            assert_eq!(tls.key_path, "[REDACTED]");
        }
    }

    #[test]
    fn test_sanitize_preserves_non_secret_fields() {
        let config = crate::config::AppConfig::load(Environment::Development)
            .expect("Failed to load test config");
        let sanitized = sanitize(&config);

        // Verify non-sensitive fields are preserved
        assert_eq!(sanitized.server.host, config.server.host);
        assert_eq!(sanitized.server.port, config.server.port);
        assert_eq!(
            sanitized.database.max_connections,
            config.database.max_connections
        );
        assert_eq!(sanitized.redis.pool_size, config.redis.pool_size);
        assert_eq!(sanitized.cors.allowed_origins, config.cors.allowed_origins);
    }

    #[test]
    fn test_sanitize_serializes_without_secrets() {
        let config = crate::config::AppConfig::load(Environment::Development)
            .expect("Failed to load test config");
        let sanitized = sanitize(&config);

        let json = serde_json::to_string(&sanitized).expect("Serialization failed");

        // Verify no actual secrets appear in JSON
        assert!(!json.contains(&config.database.url));
        assert!(!json.contains(&config.redis.url));
        assert!(json.contains("[REDACTED]"));
    }

    #[test]
    fn test_sanitize_optional_redis_job_queue_url() {
        let config = crate::config::AppConfig::load(Environment::Development)
            .expect("Failed to load test config");
        let sanitized = sanitize(&config);

        if sanitized.redis.job_queue_url.is_some() {
            assert_eq!(sanitized.redis.job_queue_url, Some("[REDACTED]".to_string()));
        } else {
            assert_eq!(sanitized.redis.job_queue_url, None);
        }
    }

    #[test]
    fn test_sanitize_preserves_tls_cert_path() {
        let config = crate::config::AppConfig::load(Environment::Development)
            .expect("Failed to load test config");
        let sanitized = sanitize(&config);

        if let Some(original_tls) = &config.server.tls {
            if let Some(sanitized_tls) = &sanitized.server.tls {
                assert_eq!(sanitized_tls.cert_path, original_tls.cert_path);
                assert_eq!(sanitized_tls.key_path, "[REDACTED]");
            }
        }
    }

    #[test]
    fn test_sanitize_produces_valid_json() {
        let config = crate::config::AppConfig::load(Environment::Development)
            .expect("Failed to load test config");
        let sanitized = sanitize(&config);

        let json_str = serde_json::to_string_pretty(&sanitized)
            .expect("Failed to serialize sanitized config");

        // Verify we can parse it back
        let _: serde_json::Value =
            serde_json::from_str(&json_str).expect("Invalid JSON produced");
    }
}
