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
    /// Graceful shutdown timeout in seconds to drain in-flight worker jobs and HTTP requests (default: 30s)
    #[serde(default = "default_shutdown_timeout_secs")]
    pub shutdown_timeout_secs: u64,
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

fn default_shutdown_timeout_secs() -> u64 {
    30 // 30 seconds
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
            .field("shutdown_timeout_secs", &self.shutdown_timeout_secs)
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

use std::sync::Arc;
use tokio::sync::Mutex;
use tokio::task::JoinHandle;
use tokio_util::sync::CancellationToken;

/// Server shutdown coordinator that broadcasts a shutdown cancellation token
/// to all background workers and awaits worker termination before exiting the process.
#[derive(Clone, Debug)]
pub struct ServerShutdownCoordinator {
    cancellation_token: CancellationToken,
    worker_handles: Arc<Mutex<Vec<JoinHandle<()>>>>,
    timeout: std::time::Duration,
}

impl ServerShutdownCoordinator {
    /// Create a new shutdown coordinator with the specified timeout duration.
    pub fn new(timeout: std::time::Duration) -> Self {
        Self {
            cancellation_token: CancellationToken::new(),
            worker_handles: Arc::new(Mutex::new(Vec::new())),
            timeout,
        }
    }

    /// Access the cancellation token to propagate to background workers and ingestion tasks.
    pub fn cancellation_token(&self) -> CancellationToken {
        self.cancellation_token.clone()
    }

    /// Register a background task handle to be awaited during graceful shutdown.
    pub async fn register_worker(&self, handle: JoinHandle<()>) {
        self.worker_handles.lock().await.push(handle);
    }

    /// Signal cancellation to all workers.
    pub fn signal_shutdown(&self) {
        self.cancellation_token.cancel();
    }

    /// Broadcast shutdown cancellation and await all registered workers until completed
    /// or until the graceful shutdown timeout expires.
    pub async fn drain_workers(&self) -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
        self.signal_shutdown();

        let mut handles = self.worker_handles.lock().await;
        let futures: Vec<_> = handles.drain(..).collect();
        drop(handles);

        let drain_all = async move {
            for handle in futures {
                let _ = handle.await;
            }
        };

        match tokio::time::timeout(self.timeout, drain_all).await {
            Ok(()) => {
                tracing::info!("All background worker jobs drained successfully during shutdown");
                Ok(())
            }
            Err(_) => {
                tracing::warn!("Graceful shutdown timeout exceeded while draining background workers");
                Err("Background worker drain timed out".into())
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::{AtomicBool, Ordering};
    use std::time::Duration;
    use tokio::time::sleep;

    #[tokio::test]
    async fn test_server_shutdown_coordinator_drains_workers() {
        let coordinator = ServerShutdownCoordinator::new(Duration::from_millis(500));
        let token = coordinator.cancellation_token();

        let worker_finished = Arc::new(AtomicBool::new(false));
        let worker_finished_clone = worker_finished.clone();

        // Spawn a mock background worker loop that listens for cancellation
        let handle = tokio::spawn(async move {
            tokio::select! {
                _ = token.cancelled() => {
                    // Simulate draining in-flight work before exiting
                    sleep(Duration::from_millis(50)).await;
                    worker_finished_clone.store(true, Ordering::SeqCst);
                }
                _ = sleep(Duration::from_secs(10)) => {}
            }
        });

        coordinator.register_worker(handle).await;

        // Drain workers
        let result = coordinator.drain_workers().await;
        assert!(result.is_ok());
        assert!(worker_finished.load(Ordering::SeqCst), "worker should have completed in-flight job before exiting");
    }

    #[tokio::test]
    async fn test_server_shutdown_timeout_expiration() {
        let coordinator = ServerShutdownCoordinator::new(Duration::from_millis(50));
        let token = coordinator.cancellation_token();

        // Worker that takes longer than the timeout
        let handle = tokio::spawn(async move {
            tokio::select! {
                _ = token.cancelled() => {
                    sleep(Duration::from_millis(200)).await;
                }
                _ = sleep(Duration::from_secs(10)) => {}
            }
        });

        coordinator.register_worker(handle).await;

        let result = coordinator.drain_workers().await;
        assert!(result.is_err());
    }
}
