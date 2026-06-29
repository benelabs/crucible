//! CONFIG APPROACH: Option A — layered config crate
//! Rationale: Using the `config` crate provides a robust, layered approach where environment-specific
//! defaults are cleanly defined in TOML files, while sensitive secrets and infrastructure-specific
//! overrides are passed securely via environment variables.

use serde::{Deserialize, Serialize};
use std::fmt;

/// Server configuration governing HTTP connections.
#[derive(Clone, Deserialize, Serialize)]
pub struct ServerConfig {
    /// The host address to bind to (e.g., "127.0.0.1" or "0.0.0.0")
    pub host: String,
    /// The port to listen on
    pub port: u16,
    /// Maximum time in milliseconds to wait for a request to complete
    pub request_timeout_ms: u64,
    /// Maximum number of concurrent connections
    pub max_connections: usize,
    /// Maximum request body size in bytes (default: 10MB)
    #[serde(default = "default_max_body_size")]
    pub max_body_size: usize,
    /// Maximum request body size for contract compilation in bytes (default: 1MB)
    #[serde(default = "default_compile_max_size")]
    pub compile_max_size: usize,
    /// Maximum request body size for sandbox execution in bytes (default: 5MB)
    #[serde(default = "default_sandbox_max_size")]
    pub sandbox_max_size: usize,
    /// TLS configuration (required in production, optional elsewhere)
    pub tls: Option<TlsConfig>,
}

fn default_max_body_size() -> usize {
    10 * 1024 * 1024 // 10MB
}

fn default_compile_max_size() -> usize {
    1 * 1024 * 1024 // 1MB
}

fn default_sandbox_max_size() -> usize {
    5 * 1024 * 1024 // 5MB
}

impl fmt::Debug for ServerConfig {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("ServerConfig")
            .field("host", &self.host)
            .field("port", &self.port)
            .field("request_timeout_ms", &self.request_timeout_ms)
            .field("max_connections", &self.max_connections)
            .field("max_body_size", &self.max_body_size)
            .field("compile_max_size", &self.compile_max_size)
            .field("sandbox_max_size", &self.sandbox_max_size)
            .field("tls", &self.tls)
            .finish()
    }
}

/// TLS certificates configuration.
#[derive(Clone, Deserialize, Serialize)]
pub struct TlsConfig {
    /// Path to the TLS certificate chain
    pub cert_path: String,
    /// Path to the TLS private key. Marked as skip_serializing to avoid accidental leaks.
    #[serde(skip_serializing)]
    pub key_path: String,
}

impl fmt::Debug for TlsConfig {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("TlsConfig")
            .field("cert_path", &self.cert_path)
            .field("key_path", &"[REDACTED]")
            .finish()
    }
}
