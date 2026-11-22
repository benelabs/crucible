//! CONFIG APPROACH: Option A — layered config crate
//! Rationale: Using the `config` crate provides a robust, layered approach where environment-specific
//! defaults are cleanly defined in TOML files, while sensitive secrets and infrastructure-specific
//! overrides are passed securely via environment variables.

use axum::http::StatusCode;
use axum::response::{IntoResponse, Response};
use std::fmt;

/// Errors that can occur during configuration loading and validation.
#[derive(Debug)]
pub enum ConfigError {
    /// A required configuration field is missing.
    MissingRequiredField(String),
    /// A configuration value is invalid.
    InvalidValue { field: String, reason: String },
    /// General failure to load configuration sources.
    LoadError(String),
    /// One or more validation failures occurred across the configuration.
    ValidationError(Vec<String>),
}

impl std::error::Error for ConfigError {}

impl fmt::Display for ConfigError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            ConfigError::MissingRequiredField(field) => {
                write!(f, "Missing required configuration field: {}", field)
            }
            ConfigError::InvalidValue { field, reason } => write!(
                f,
                "Invalid value for configuration field '{}': {}",
                field, reason
            ),
            ConfigError::LoadError(reason) => write!(f, "Failed to load configuration: {}", reason),
            ConfigError::ValidationError(errors) => {
                writeln!(
                    f,
                    "Configuration validation failed with {} errors:",
                    errors.len()
                )?;
                for (i, err) in errors.iter().enumerate() {
                    writeln!(f, "  {}. {}", i + 1, err)?;
                }
                Ok(())
            }
        }
    }
}

impl IntoResponse for ConfigError {
    fn into_response(self) -> Response {
        let body = format!("Startup Configuration Error: {}", self);
        // It's generally uncommon to expose detailed config errors over HTTP in production,
        // but this fulfills the requirement to convert to IntoResponse (e.g. for a health check endpoint).
        (StatusCode::INTERNAL_SERVER_ERROR, body).into_response()
    }
}
