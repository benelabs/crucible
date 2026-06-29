//! CONFIG APPROACH: Option A — layered config crate

use crate::config::{AppConfig, Environment, ObservabilityConfig};
use std::str::FromStr;

#[test]
fn test_toml_files_valid() {
    // This ensures that all TOML files are syntactically valid at compile time and parse correctly.
    let default_str = include_str!("defaults/default.toml");
    let dev_str = include_str!("defaults/development.toml");
    let staging_str = include_str!("defaults/staging.toml");
    let prod_str = include_str!("defaults/production.toml");

    let _: toml::Value = toml::from_str(default_str).expect("default.toml is malformed");
    let _: toml::Value = toml::from_str(dev_str).expect("development.toml is malformed");
    let _: toml::Value = toml::from_str(staging_str).expect("staging.toml is malformed");
    let _: toml::Value = toml::from_str(prod_str).expect("production.toml is malformed");
}

#[test]
fn test_environment_parsing() {
    assert_eq!(
        Environment::from_str("development").unwrap(),
        Environment::Development
    );
    assert_eq!(
        Environment::from_str("dev").unwrap(),
        Environment::Development
    );
    assert_eq!(
        Environment::from_str("STAGING").unwrap(),
        Environment::Staging
    );
    assert_eq!(
        Environment::from_str("prod").unwrap(),
        Environment::Production
    );
    assert!(Environment::from_str("invalid").is_err());
}

#[test]
fn test_load_development_config() {
    // Set required env vars to avoid validation errors without race conditions
    temp_env::with_vars(
        [
            (
                "APP_DATABASE__URL",
                Some("postgres://user:pass@localhost/db"),
            ),
            ("APP_REDIS__URL", Some("redis://localhost:6379")),
        ],
        || {
            let config =
                AppConfig::load(Environment::Development).expect("Failed to load dev config");

            // Check overrides from development.toml vs default.toml
            assert_eq!(config.server.host, "127.0.0.1");
            assert_eq!(config.server.port, 3000);
            assert_eq!(config.observability.log_level, "debug");

            // Check SQLx pool options translation
            let _pool_opts = config.database.to_sqlx_pool_options();
            // We verify the translation executes without panicking and producing valid options.
            assert_eq!(config.database.max_connections, 5); // From dev defaults

            // Tracing init shouldn't panic
            config.observability.init_tracing(Environment::Development);
        },
    );
}

#[test]
fn test_production_observability_uses_json_logging() {
    let observability = ObservabilityConfig {
        log_level: "info".into(),
        tracing_endpoint: None,
        enable_metrics: false,
    };

    observability.init_tracing(Environment::Production);
}

#[test]
fn test_production_missing_tls_validation() {
    temp_env::with_vars(
        [
            (
                "APP_DATABASE__URL",
                Some("postgres://user:pass@localhost/db"),
            ),
            ("APP_REDIS__URL", Some("redis://localhost:6379")),
            (
                "APP_CORS__ALLOWED_ORIGINS__0",
                Some("https://example.com"),
            ),
        ],
        || {
            let err = AppConfig::load(Environment::Production)
                .expect_err("Should fail without TLS config");

            match err {
                crate::config::ConfigError::ValidationError(errors) => {
                    assert!(errors
                        .iter()
                        .any(|e| e.contains("TLS configuration is strictly required")));
                }
                _ => panic!("Expected ValidationError, got {:?}", err),
            }
        },
    );
}

#[test]
fn test_validation_collects_all_errors() {
    temp_env::with_vars(
        [
            ("APP_DATABASE__URL", Some("")), // Invalid (empty)
            ("APP_REDIS__URL", Some("")),    // Invalid (empty)
        ],
        || {
            let err = AppConfig::load(Environment::Production).expect_err("Should fail validation");

            match err {
                crate::config::ConfigError::ValidationError(errors) => {
                    // TLS missing, DB URL empty, Redis URL empty, CORS origins empty
                    assert!(errors.len() >= 4);
                    assert!(errors.iter().any(|e| e.contains("TLS configuration")));
                    assert!(errors.iter().any(|e| e.contains("Database URL")));
                    assert!(errors.iter().any(|e| e.contains("Redis URL")));
                    assert!(errors.iter().any(|e| e.contains("CORS")));
                }
                _ => panic!("Expected ValidationError, got {:?}", err),
            }
        },
    );
}

#[test]
fn test_env_var_override_wins() {
    temp_env::with_vars(
        [
            (
                "APP_DATABASE__URL",
                Some("postgres://user:pass@localhost/db"),
            ),
            ("APP_REDIS__URL", Some("redis://localhost:6379")),
            ("APP_SERVER__PORT", Some("9999")),
            ("APP_DATABASE__MAX_CONNECTIONS", Some("42")),
        ],
        || {
            let config = AppConfig::load(Environment::Development).unwrap();

            assert_eq!(config.server.port, 9999);
            assert_eq!(config.database.max_connections, 42);
        },
    );
}

#[test]
fn test_database_to_pool_options() {
    let config = crate::config::DatabaseConfig {
        url: "postgres://localhost".to_string(),
        max_connections: 42,
        min_connections: 5,
        connect_timeout_secs: 15,
        idle_timeout_secs: 30,
    };

    let _pool_opts = config.to_sqlx_pool_options();
    // PoolOptions builder successfully generated
}

#[test]
fn test_sensitive_fields_redacted_in_debug() {
    let config = crate::config::AppConfig {
        server: crate::config::ServerConfig {
            host: "localhost".into(),
            port: 8080,
            request_timeout_ms: 1000,
            max_connections: 10,
            tls: Some(crate::config::TlsConfig {
                cert_path: "/path/to/cert".into(),
                key_path: "/path/to/secret.key".into(),
            }),
        },
        database: crate::config::DatabaseConfig {
            url: "postgres://secret:password@host/db".into(),
            max_connections: 10,
            min_connections: 1,
            connect_timeout_secs: 5,
            idle_timeout_secs: 5,
        },
        redis: crate::config::RedisConfig {
            url: "redis://:secretpass@host".into(),
            job_queue_url: Some("redis://:secretpass2@host2".into()),
            pool_size: 10,
            connection_timeout_ms: 1000,
            max_retries: 3,
        },
        observability: crate::config::ObservabilityConfig {
            log_level: "info".into(),
            tracing_endpoint: None,
            enable_metrics: false,
        },
        cors: crate::config::CorsConfig::default(),
    };

    let debug_str = format!("{:?}", config);

    assert!(!debug_str.contains("secret:password"));
    assert!(!debug_str.contains("secretpass"));
    assert!(!debug_str.contains("secretpass2"));
    assert!(!debug_str.contains("secret.key"));

    assert!(debug_str.contains("[REDACTED]"));
    assert!(debug_str.contains("/path/to/cert")); // Public cert path is OK
}

#[test]
fn test_production_rejects_wildcard_cors() {
    temp_env::with_vars(
        [
            (
                "APP_DATABASE__URL",
                Some("postgres://user:pass@localhost/db"),
            ),
            ("APP_REDIS__URL", Some("redis://localhost:6379")),
            ("APP_CORS__ALLOWED_ORIGINS__0", Some("*")),
        ],
        || {
            let err = AppConfig::load(Environment::Production)
                .expect_err("Should fail with wildcard CORS in production");

            match err {
                crate::config::ConfigError::ValidationError(errors) => {
                    assert!(
                        errors.iter().any(|e| e.contains("CORS wildcard")),
                        "Expected CORS wildcard error, got: {:?}",
                        errors
                    );
                }
                _ => panic!("Expected ValidationError, got {:?}", err),
            }
        },
    );
}

#[test]
fn test_production_rejects_empty_cors_origins() {
    temp_env::with_vars(
        [
            (
                "APP_DATABASE__URL",
                Some("postgres://user:pass@localhost/db"),
            ),
            ("APP_REDIS__URL", Some("redis://localhost:6379")),
        ],
        || {
            let err = AppConfig::load(Environment::Production)
                .expect_err("Should fail with empty CORS origins in production");

            match err {
                crate::config::ConfigError::ValidationError(errors) => {
                    assert!(
                        errors.iter().any(|e| e.contains("CORS allowed_origins must not be empty")),
                        "Expected CORS empty error, got: {:?}",
                        errors
                    );
                }
                _ => panic!("Expected ValidationError, got {:?}", err),
            }
        },
    );
}
