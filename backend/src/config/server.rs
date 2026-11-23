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
    /// Grace period in seconds to wait for background workers and in-flight jobs to drain before terminating (default: 30s)
    #[serde(default = "default_worker_grace_period_secs")]
    pub worker_grace_period_secs: u64,
    /// Maximum concurrent HTTP/2 streams per connection (prevents Rapid Reset DoS attacks)
    #[serde(default = "default_http2_max_concurrent_streams")]
    pub http2_max_concurrent_streams: u32,
    /// TLS configuration (required in production, optional elsewhere)
    pub tls: Option<TlsConfig>,
}

fn default_max_body_size() -> usize {
    10 * 1024 * 1024 // 10MB
}

fn default_compile_max_size() -> usize {
    1024 * 1024 // 1MB
}

fn default_sandbox_max_size() -> usize {
    5 * 1024 * 1024 // 5MB
}

fn default_worker_grace_period_secs() -> u64 {
    30
}

fn default_http2_max_concurrent_streams() -> u32 {
    100
}

impl ServerConfig {
    /// Returns the maximum concurrent HTTP/2 streams allowed per connection.
    pub fn http2_max_concurrent_streams(&self) -> u32 {
        self.http2_max_concurrent_streams
    }

    /// Configures a Hyper HTTP/2 server builder with the max concurrent streams bound.
    pub fn configure_http2_builder<E>(&self, builder: &mut hyper::server::conn::http2::Builder<E>) {
        builder.max_concurrent_streams(self.http2_max_concurrent_streams);
    }

    /// Returns the maximum concurrent HTTP/2 streams bound for the server builder.
    pub fn http2_max_concurrent_streams_bound(&self) -> Option<u32> {
        Some(self.http2_max_concurrent_streams)
    }
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
            .field("worker_grace_period_secs", &self.worker_grace_period_secs)
            .field(
                "http2_max_concurrent_streams",
                &self.http2_max_concurrent_streams,
            )
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn server_config_defaults_include_worker_grace_period() {
        let json_str = r#"{
            "host": "127.0.0.1",
            "port": 8080,
            "request_timeout_ms": 30000,
            "max_connections": 1000
        }"#;

        let config: ServerConfig = serde_json::from_str(json_str).unwrap();
        assert_eq!(config.worker_grace_period_secs, 30);
        assert_eq!(config.max_body_size, 10 * 1024 * 1024);
        assert_eq!(config.compile_max_size, 1024 * 1024);
        assert_eq!(config.sandbox_max_size, 5 * 1024 * 1024);
    }

    #[test]
    fn server_config_custom_worker_grace_period() {
        let json_str = r#"{
            "host": "0.0.0.0",
            "port": 9000,
            "request_timeout_ms": 15000,
            "max_connections": 500,
            "worker_grace_period_secs": 45
        }"#;

        let config: ServerConfig = serde_json::from_str(json_str).unwrap();
        assert_eq!(config.worker_grace_period_secs, 45);
    }

    #[test]
    fn test_server_config_http2_max_concurrent_streams() {
        let config = ServerConfig {
            host: "0.0.0.0".to_string(),
            port: 3000,
            request_timeout_ms: 30_000,
            max_connections: 10_000,
            max_body_size: default_max_body_size(),
            compile_max_size: default_compile_max_size(),
            sandbox_max_size: default_sandbox_max_size(),
            worker_grace_period_secs: default_worker_grace_period_secs(),
            http2_max_concurrent_streams: default_http2_max_concurrent_streams(),
            tls: None,
        };

        assert_eq!(config.http2_max_concurrent_streams(), 100);

        #[derive(Clone)]
        struct MockExecutor;
        impl<F> hyper::rt::Executor<F> for MockExecutor
        where
            F: std::future::Future + Send + 'static,
            F::Output: Send + 'static,
        {
            fn execute(&self, _fut: F) {}
        }

        let mut builder = hyper::server::conn::http2::Builder::new(MockExecutor);
        config.configure_http2_builder(&mut builder);
    }

    #[test]
    fn server_config_defaults_include_http2_max_concurrent_streams() {
        let json_str = r#"{
            "host": "127.0.0.1",
            "port": 8080,
            "request_timeout_ms": 30000,
            "max_connections": 1000
        }"#;

        let config: ServerConfig = serde_json::from_str(json_str).unwrap();
        assert_eq!(config.http2_max_concurrent_streams, 100);
        assert_eq!(config.http2_max_concurrent_streams_bound(), Some(100));
        assert_eq!(config.max_body_size, 10 * 1024 * 1024);
        assert_eq!(config.compile_max_size, 1024 * 1024);
        assert_eq!(config.sandbox_max_size, 5 * 1024 * 1024);
    }

    #[test]
    fn server_config_custom_http2_max_concurrent_streams() {
        let json_str = r#"{
            "host": "0.0.0.0",
            "port": 9000,
            "request_timeout_ms": 15000,
            "max_connections": 500,
            "http2_max_concurrent_streams": 250
        }"#;

        let config: ServerConfig = serde_json::from_str(json_str).unwrap();
        assert_eq!(config.http2_max_concurrent_streams, 250);
        assert_eq!(config.http2_max_concurrent_streams_bound(), Some(250));
    }

    #[test]
    fn server_config_debug_includes_http2_bound() {
        let json_str = r#"{
            "host": "127.0.0.1",
            "port": 8080,
            "request_timeout_ms": 30000,
            "max_connections": 1000
        }"#;

        let config: ServerConfig = serde_json::from_str(json_str).unwrap();
        let debug_str = format!("{:?}", config);
        assert!(debug_str.contains("http2_max_concurrent_streams: 100"));
    }
}
