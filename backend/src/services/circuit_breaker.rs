//! Circuit breaker pattern for external service calls.
//!
//! Implements the classic three-state circuit breaker (Closed → Open → Half-Open)
//! to prevent cascading failures when external services are unavailable.

use std::sync::Arc;
use std::time::{Duration, Instant};
use tokio::sync::RwLock;
use tracing::{info, warn};

/// The state of a circuit breaker.
#[derive(Debug, Clone, PartialEq)]
pub enum CircuitState {
    /// Normal operation — requests pass through.
    Closed,
    /// Service is failing — requests are rejected immediately.
    Open { opened_at: Instant },
    /// Testing recovery — one probe request is allowed through.
    HalfOpen,
}

/// Configuration for a circuit breaker.
#[derive(Debug, Clone)]
pub struct CircuitBreakerConfig {
    /// Number of consecutive failures before opening the circuit.
    pub failure_threshold: u32,
    /// How long to wait in Open state before transitioning to HalfOpen.
    pub recovery_timeout: Duration,
    /// Number of consecutive successes in HalfOpen before closing.
    pub success_threshold: u32,
}

impl Default for CircuitBreakerConfig {
    fn default() -> Self {
        Self {
            failure_threshold: 5,
            recovery_timeout: Duration::from_secs(30),
            success_threshold: 2,
        }
    }
}

#[derive(Debug)]
struct CircuitBreakerInner {
    state: CircuitState,
    consecutive_failures: u32,
    consecutive_successes: u32,
    probe_in_flight: bool,
    config: CircuitBreakerConfig,
}

impl CircuitBreakerInner {
    fn new(config: CircuitBreakerConfig) -> Self {
        Self {
            state: CircuitState::Closed,
            consecutive_failures: 0,
            consecutive_successes: 0,
            probe_in_flight: false,
            config,
        }
    }

    fn is_request_allowed(&mut self) -> bool {
        match &self.state {
            CircuitState::Closed => true,
            CircuitState::Open { opened_at } => {
                if opened_at.elapsed() >= self.config.recovery_timeout {
                    self.state = CircuitState::HalfOpen;
                    self.consecutive_successes = 0;
                    self.probe_in_flight = true;
                    info!("Circuit breaker transitioning to HalfOpen, allowing single canary probe");
                    true
                } else {
                    false
                }
            }
            CircuitState::HalfOpen => {
                if self.probe_in_flight {
                    // Restrict HalfOpen to exactly one in-flight probe request;
                    // reject concurrent requests to prevent thundering herd on recovering service.
                    false
                } else {
                    self.probe_in_flight = true;
                    info!("Circuit breaker allowing probe request in HalfOpen");
                    true
                }
            }
        }
    }

    fn record_success(&mut self) {
        self.probe_in_flight = false;
        match self.state {
            CircuitState::HalfOpen => {
                self.consecutive_successes += 1;
                if self.consecutive_successes >= self.config.success_threshold {
                    self.state = CircuitState::Closed;
                    self.consecutive_failures = 0;
                    info!("Circuit breaker closed after successful recovery");
                }
            }
            CircuitState::Closed => {
                self.consecutive_failures = 0;
            }
            _ => {}
        }
    }

    fn record_failure(&mut self) {
        self.probe_in_flight = false;
        self.consecutive_failures += 1;
        match self.state {
            CircuitState::Closed => {
                if self.consecutive_failures >= self.config.failure_threshold {
                    self.state = CircuitState::Open {
                        opened_at: Instant::now(),
                    };
                    warn!(
                        failures = self.consecutive_failures,
                        "Circuit breaker opened after threshold exceeded"
                    );
                }
            }
            CircuitState::HalfOpen => {
                self.state = CircuitState::Open {
                    opened_at: Instant::now(),
                };
                warn!("Circuit breaker re-opened after probe failure");
            }
            _ => {}
        }
    }
}

/// A thread-safe circuit breaker for wrapping external service calls.
#[derive(Clone)]
pub struct CircuitBreaker {
    name: String,
    inner: Arc<RwLock<CircuitBreakerInner>>,
}

#[derive(Debug, thiserror::Error)]
pub enum CircuitBreakerError<E> {
    #[error("Circuit is open for service '{0}'")]
    Open(String),
    #[error("Service call failed: {0}")]
    ServiceError(E),
}

impl CircuitBreaker {
    pub fn new(name: impl Into<String>, config: CircuitBreakerConfig) -> Self {
        Self {
            name: name.into(),
            inner: Arc::new(RwLock::new(CircuitBreakerInner::new(config))),
        }
    }

    /// Execute a fallible async closure through the circuit breaker.
    pub async fn call<F, Fut, T, E>(&self, f: F) -> Result<T, CircuitBreakerError<E>>
    where
        F: FnOnce() -> Fut,
        Fut: std::future::Future<Output = Result<T, E>>,
    {
        {
            let mut inner = self.inner.write().await;
            if !inner.is_request_allowed() {
                return Err(CircuitBreakerError::Open(self.name.clone()));
            }
        }

        struct ProbeGuard<'a> {
            inner: &'a RwLock<CircuitBreakerInner>,
            completed: bool,
        }
        impl<'a> Drop for ProbeGuard<'a> {
            fn drop(&mut self) {
                if !self.completed {
                    if let Ok(mut inner) = self.inner.try_write() {
                        inner.probe_in_flight = false;
                    }
                }
            }
        }

        let mut guard = ProbeGuard {
            inner: &self.inner,
            completed: false,
        };

        match f().await {
            Ok(value) => {
                guard.completed = true;
                self.inner.write().await.record_success();
                Ok(value)
            }
            Err(e) => {
                guard.completed = true;
                self.inner.write().await.record_failure();
                Err(CircuitBreakerError::ServiceError(e))
            }
        }
    }

    /// Returns the current state label for observability.
    pub async fn state_label(&self) -> &'static str {
        match self.inner.read().await.state {
            CircuitState::Closed => "closed",
            CircuitState::Open { .. } => "open",
            CircuitState::HalfOpen => "half_open",
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn fast_config() -> CircuitBreakerConfig {
        CircuitBreakerConfig {
            failure_threshold: 3,
            recovery_timeout: Duration::from_millis(50),
            success_threshold: 1,
        }
    }

    #[tokio::test]
    async fn test_closed_allows_requests() {
        let cb = CircuitBreaker::new("test", fast_config());
        let result = cb.call(|| async { Ok::<_, String>("ok") }).await;
        assert!(result.is_ok());
    }

    #[tokio::test]
    async fn test_opens_after_threshold() {
        let cb = CircuitBreaker::new("test", fast_config());
        for _ in 0..3 {
            let _ = cb.call(|| async { Err::<(), _>("fail") }).await;
        }
        let result = cb.call(|| async { Ok::<_, String>("ok") }).await;
        assert!(matches!(result, Err(CircuitBreakerError::Open(_))));
    }

    #[tokio::test]
    async fn test_recovers_after_timeout() {
        let cb = CircuitBreaker::new("test", fast_config());
        for _ in 0..3 {
            let _ = cb.call(|| async { Err::<(), _>("fail") }).await;
        }
        tokio::time::sleep(Duration::from_millis(60)).await;
        let result = cb.call(|| async { Ok::<_, String>("ok") }).await;
        assert!(result.is_ok());
    }

    #[tokio::test]
    async fn test_state_label() {
        let cb = CircuitBreaker::new("test", fast_config());
        assert_eq!(cb.state_label().await, "closed");
        for _ in 0..3 {
            let _ = cb.call(|| async { Err::<(), _>("fail") }).await;
        }
        assert_eq!(cb.state_label().await, "open");
    }

    #[tokio::test]
    async fn test_half_open_restricts_to_single_in_flight_probe() {
        let cb = CircuitBreaker::new("test", fast_config());
        // Force circuit breaker into Open state
        for _ in 0..3 {
            let _ = cb.call(|| async { Err::<(), _>("fail") }).await;
        }
        assert_eq!(cb.state_label().await, "open");

        // Wait for recovery timeout to transition to HalfOpen
        tokio::time::sleep(Duration::from_millis(60)).await;

        let (probe_started_tx, probe_started_rx) = tokio::sync::oneshot::channel();
        let (release_probe_tx, release_probe_rx) = tokio::sync::oneshot::channel();

        let cb_clone = cb.clone();
        // Spawn canary probe request that holds the probe in-flight
        let probe_task = tokio::spawn(async move {
            cb_clone
                .call(|| async move {
                    let _ = probe_started_tx.send(());
                    let _ = release_probe_rx.await;
                    Ok::<_, String>("probe_success")
                })
                .await
        });

        // Wait for the probe to start and be in-flight
        let _ = probe_started_rx.await;

        // Any concurrent request during HalfOpen must be rejected
        let concurrent_result = cb.call(|| async { Ok::<_, String>("concurrent") }).await;
        assert!(
            matches!(concurrent_result, Err(CircuitBreakerError::Open(_))),
            "Concurrent request in HalfOpen must be rejected while probe is in flight"
        );

        // Release the canary probe
        let _ = release_probe_tx.send(());
        let probe_result = probe_task.await.unwrap();
        assert!(probe_result.is_ok());

        // Now circuit breaker should be closed (success_threshold = 1) and allow requests
        assert_eq!(cb.state_label().await, "closed");
        let post_recovery = cb.call(|| async { Ok::<_, String>("allowed") }).await;
        assert!(post_recovery.is_ok());
    }
}
