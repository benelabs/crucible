//! # Configuration Dump Utility
//!
//! Standalone CLI binary that loads the effective application configuration
//! and outputs it in sanitized (secrets-redacted) JSON format.
//!
//! This tool is useful for operators to inspect configuration without exposing
//! secrets, for diagnostics, debugging, and configuration verification.
//!
//! ## Usage
//!
//! ```bash
//! # Default format (JSON, pretty-printed)
//! cargo run --bin config-dump
//!
//! # Output to file
//! cargo run --bin config-dump > config.json
//!
//! # Use with jq to inspect specific fields
//! cargo run --bin config-dump | jq '.server'
//! ```
//!
//! ## Configuration
//!
//! The utility respects the same configuration loading as the main application:
//! 1. Embedded default configuration (`defaults/default.toml`)
//! 2. Environment-specific overrides (`defaults/development.toml`, etc.)
//! 3. Environment variable overrides (prefix: `APP_`, separator: `__`)
//!
//! Set `APP_ENV` to control the environment:
//! - `development` (default)
//! - `staging`
//! - `production`
//!
//! ## Output
//!
//! The output is a JSON document with the following top-level keys:
//! - `server`: HTTP server configuration
//! - `database`: PostgreSQL database configuration
//! - `redis`: Redis cache configuration
//! - `observability`: Tracing and logging configuration
//! - `cors`: CORS configuration
//!
//! All sensitive fields (URLs, private keys, credentials) are redacted to `"[REDACTED]"`.
//!
//! ## Examples
//!
//! **Inspect database configuration:**
//! ```bash
//! cargo run --bin config-dump | jq '.database'
//! ```
//! Output:
//! ```json
//! {
//!   "url": "[REDACTED]",
//!   "max_connections": 20,
//!   "min_connections": 5,
//!   "connect_timeout_secs": 30,
//!   "idle_timeout_secs": 600
//! }
//! ```
//!
//! **Check if TLS is configured:**
//! ```bash
//! cargo run --bin config-dump | jq '.server.tls'
//! ```
//!
//! **Verify CORS origins:**
//! ```bash
//! cargo run --bin config-dump | jq '.cors.allowed_origins'
//! ```

use backend::config::{sanitize, AppConfig, Environment};
use std::io::Write;

fn main() -> anyhow::Result<()> {
    // Load environment
    let env = Environment::from_env();

    // Load configuration (respects embedded defaults, env overrides, and env vars)
    let config = AppConfig::load(env).expect("Failed to load configuration");

    // Sanitize to remove secrets
    let sanitized = sanitize(&config);

    // Serialize to JSON with pretty printing
    let json = serde_json::to_string_pretty(&sanitized)?;

    // Output to stdout
    writeln!(std::io::stdout(), "{}", json)?;

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_config_dump_produces_valid_json() {
        let config = AppConfig::load(Environment::Development)
            .expect("Failed to load test config");
        let sanitized = sanitize(&config);
        let json_str = serde_json::to_string_pretty(&sanitized)
            .expect("Failed to serialize config");

        // Verify it can be parsed back
        let _: serde_json::Value =
            serde_json::from_str(&json_str).expect("Invalid JSON produced");
    }

    #[test]
    fn test_config_dump_redacts_secrets() {
        let config = AppConfig::load(Environment::Development)
            .expect("Failed to load test config");
        let sanitized = sanitize(&config);
        let json_str = serde_json::to_string_pretty(&sanitized)
            .expect("Failed to serialize config");

        // Ensure no real secrets in output
        assert!(!json_str.contains(&config.database.url));
        assert!(!json_str.contains(&config.redis.url));

        // Ensure redaction markers are present
        assert!(json_str.contains("[REDACTED]"));
    }

    #[test]
    fn test_config_dump_preserves_non_secrets() {
        let config = AppConfig::load(Environment::Development)
            .expect("Failed to load test config");
        let sanitized = sanitize(&config);
        let json_str = serde_json::to_string_pretty(&sanitized)
            .expect("Failed to serialize config");

        // Ensure non-sensitive values are preserved in JSON
        assert!(json_str.contains(&config.server.host.to_string()));
        assert!(json_str.contains(&config.server.port.to_string()));
    }
}
