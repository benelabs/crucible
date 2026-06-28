//! CONFIG APPROACH: Option A — layered config crate
//! Rationale: Using the `config` crate provides a robust, layered approach where environment-specific
//! defaults are cleanly defined in TOML files, while sensitive secrets and infrastructure-specific
//! overrides are passed securely via environment variables. This prevents environment variable sprawl,
//! ensures typed nested structures, and makes local development frictionless without compromising
//! production security.
//!
//! - [`AppConfig`] — hot-reloadable runtime configuration, loaded via the layered config crate.
//! - [`reload`] — [`ConfigManager`] and Axum handlers for live config updates.

use config::{Config as ConfigBuilder, Environment as ConfigEnvironment, File, FileFormat};
use serde::{Deserialize, Serialize};
use std::str::FromStr;

pub mod cors;
pub mod database;
pub mod error;
pub mod observability;
pub mod redis;
pub mod reload;
pub mod sanitize;
pub mod server;

#[cfg(test)]
mod tests;

pub use cors::CorsConfig;
pub use database::DatabaseConfig;
pub use error::ConfigError;
pub use observability::ObservabilityConfig;
pub use redis::RedisConfig;
pub use sanitize::{sanitize, SanitizedConfig};
pub use server::ServerConfig;



/// The execution environment of the application.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Deserialize, Serialize)]
pub enum Environment {
    Development,
    Staging,
    Production,
}

impl FromStr for Environment {
    type Err = String;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        match s.to_lowercase().as_str() {
            "development" | "dev" => Ok(Environment::Development),
            "staging" | "stg" => Ok(Environment::Staging),
            "production" | "prod" => Ok(Environment::Production),
            _ => Err(format!("Unknown environment: {}", s)),
        }
    }
}

impl Environment {
    /// Returns the string representation of the environment.
    pub fn as_str(&self) -> &'static str {
        match self {
            Environment::Development => "development",
            Environment::Staging => "staging",
            Environment::Production => "production",
        }
    }

    /// Loads the environment from the `APP_ENV` environment variable.
    /// Defaults to `Development` if the variable is missing or unparseable.
    pub fn from_env() -> Self {
        match std::env::var("APP_ENV") {
            Ok(val) => match Environment::from_str(&val) {
                Ok(env) => env,
                Err(e) => {
                    tracing::warn!("Failed to parse APP_ENV: {}. Defaulting to Development.", e);
                    Environment::Development
                }
            },
            Err(_) => {
                tracing::warn!("APP_ENV environment variable not set. Defaulting to Development.");
                Environment::Development
            }
        }
    }
}

/// The root configuration object.
///
/// **Usage Pattern:**
/// `AppConfig::load` should be called exactly once during application startup.
/// The resulting configuration should be wrapped in an `std::sync::Arc<AppConfig>` and
/// shared across the application state to ensure consistent configuration reads without
/// cloning large structs or repeatedly parsing environments.
#[derive(Clone, Debug, Deserialize, Serialize)]
pub struct AppConfig {
    pub server: ServerConfig,
    pub database: DatabaseConfig,
    pub redis: RedisConfig,
    pub observability: ObservabilityConfig,
    #[serde(default)]
    pub cors: CorsConfig,
}

// Temporary fallback for testing/reloading if missing in main.rs
impl Default for AppConfig {
    fn default() -> Self {
        Self::load(Environment::Development).expect("Failed to load default configuration")
    }
}

impl AppConfig {
    /// Loads the configuration by layering default settings, environment-specific
    /// settings, and finally environment variable overrides.
    pub fn load(env: Environment) -> Result<Self, ConfigError> {
        // Embed the TOML files into the binary to avoid distributing config files in production.
        let default_config = include_str!("defaults/default.toml");

        let env_config = match env {
            Environment::Development => include_str!("defaults/development.toml"),
            Environment::Staging => include_str!("defaults/staging.toml"),
            Environment::Production => include_str!("defaults/production.toml"),
        };

        let builder = ConfigBuilder::builder()
            // 1. Base configuration defaults
            .add_source(File::from_str(default_config, FileFormat::Toml))
            // 2. Environment-specific overrides
            .add_source(File::from_str(env_config, FileFormat::Toml))
            // 3. Environment variables (e.g., APP_DATABASE__URL overrides database.url)
            .add_source(
                ConfigEnvironment::with_prefix("APP")
                    .separator("__")
                    .ignore_empty(true),
            );

        let config_result = builder
            .build()
            .map_err(|e| ConfigError::LoadError(e.to_string()))?;
        let app_config: AppConfig = config_result
            .try_deserialize()
            .map_err(|e| ConfigError::LoadError(e.to_string()))?;

        app_config.validate(env)?;

        Ok(app_config)
    }

    /// Validates the loaded configuration according to the current environment's rules.
    fn validate(&self, env: Environment) -> Result<(), ConfigError> {
        let mut errors = Vec::new();

        if env == Environment::Production && self.server.tls.is_none() {
            errors.push(
                "TLS configuration is strictly required in the Production environment.".to_string(),
            );
        }

        if self.database.url.is_empty() {
            errors.push("Database URL cannot be empty.".to_string());
        }

        if self.redis.url.is_empty() {
            errors.push("Redis URL cannot be empty.".to_string());
        }

        if self.database.max_connections == 0 {
            errors.push("Database max_connections must be greater than 0.".to_string());
        }

        if self.redis.pool_size == 0 {
            errors.push("Redis pool_size must be greater than 0.".to_string());
        }

        if env == Environment::Production || env == Environment::Staging {
            if self.cors.allowed_origins.is_empty() {
                errors.push(
                    "CORS allowed_origins must not be empty in Production/Staging. \
                     Set explicit origins (e.g. APP_CORS__ALLOWED_ORIGINS__0=https://example.com)."
                        .to_string(),
                );
            } else if self.cors.allowed_origins.contains(&"*".to_string()) {
                errors.push(
                    "CORS wildcard '*' is not allowed in Production/Staging. \
                     Replace with explicit origins (e.g. APP_CORS__ALLOWED_ORIGINS__0=https://example.com)."
                        .to_string(),
                );
            }
        }

        if !errors.is_empty() {
            return Err(ConfigError::ValidationError(errors));
        }

        Ok(())
    }
}
