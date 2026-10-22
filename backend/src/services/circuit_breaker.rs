//! Circuit breaker pattern for external service calls.
//!
//! Implements the classic three-state circuit breaker (Closed → Open → Half-Open)
//! to prevent cascading failures when external services are unavailable.

use std::collections::VecDeque;
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
    /// Number of failures within the sliding window before opening the circuit.
    pub failure_threshold: u32,
    /// How long to wait in Open state before transitioning to HalfOpen.
    pub recovery_timeout: Duration,
    /// Number of consecutive successes in HalfOpen before closing.
    pub success_threshold: u32,
    /// Sliding time window for tracking failures in Closed state.
    pub failure_window: Duration,
}

impl Default for CircuitBreakerConfig {
    fn default() -> Self {
        Self {
            failure_threshold: 5,
            recovery_timeout: Duration::from_secs(30),
            success_threshold: 2,
            failure_window: Duration::from_secs(60),
        }
    }
}

#[derive(Debug)]
struct CircuitBreakerInner {
    state: CircuitState,
    failure_timestamps: VecDeque<Instant>,
    consecutive_successes: u32,
    config: CircuitBreakerConfig,
}

impl CircuitBreakerInner {
    fn new(config: CircuitBreakerConfig) -> Self {
        Self {
            state: CircuitState::Closed,
            failure_timestamps: VecDeque::new(),
            consecutive_successes: 0,
            config,
        }
    }

    fn prune_failures(&mut self, now: Instant) {
        if let Some(cutoff) = now.checked_sub(self.config.failure_window) {
            while let Some(&timestamp) = self.failure_timestamps.front() {
                if timestamp < cutoff {
                    self.failure_timestamps.pop_front();
                } else {
                    break;
                }
            }
        }
    }

    fn is_request_allowed(&mut self) -> bool {
        match &self.state {
            CircuitState::Closed => true,
            CircuitState::Open { opened_at } => {
                if opened_at.elapsed() >= self.config.recovery_timeout {
                    self.state = CircuitState::HalfOpen;
                    self.consecutive_successes = 0;
                    info!("Circuit breaker transitioning to HalfOpen");
                    true
                } else {
                    false
                }
            }
            CircuitState::HalfOpen => true,
        }
    }

    fn record_success(&mut self) {
        let now = Instant::now();
        self.prune_failures(now);
        match self.state {
            CircuitState::HalfOpen => {
                self.consecutive_successes += 1;
                if self.consecutive_successes >= self.config.success_threshold {
                    self.state = CircuitState::Closed;
                    self.failure_timestamps.clear();
                    info!("Circuit breaker closed after successful recovery");
                }
            }
            CircuitState::Closed => {
                // Do not immediately reset failure count to zero on an isolated success.
                // Failures age out gracefully via the sliding time window.
            }
            _ => {}
        }
    }

    fn record_failure(&mut self) {
        let now = Instant::now();
        self.prune_failures(now);
        self.failure_timestamps.push_back(now);

        match self.state {
            CircuitState::Closed => {
                if self.failure_timestamps.len() >= self.config.failure_threshold as usize {
                    self.state = CircuitState::Open {
                        opened_at: now,
                    };
                    warn!(
                        failures = self.failure_timestamps.len(),
                        "Circuit breaker opened after threshold exceeded within sliding window"
                    );
                }
            }
            CircuitState::HalfOpen => {
                self.state = CircuitState::Open {
                    opened_at: now,
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

        match f().await {
            Ok(value) => {
                self.inner.write().await.record_success();
                Ok(value)
            }
            Err(e) => {
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
            failure_window: Duration::from_millis(200),
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
    async fn test_flaky_service_with_intermittent_success_trips_breaker() {
        let cb = CircuitBreaker::new("test", fast_config());
        // Failure 1
        let _ = cb.call(|| async { Err::<(), _>("fail1") }).await;
        // Intermittent success (should NOT immediately reset the failure counter)
        let _ = cb.call(|| async { Ok::<_, String>("ok1") }).await;
        // Failure 2
        let _ = cb.call(|| async { Err::<(), _>("fail2") }).await;
        // Intermittent success
        let _ = cb.call(|| async { Ok::<_, String>("ok2") }).await;
        // Failure 3 (within sliding window, trips breaker)
        let _ = cb.call(|| async { Err::<(), _>("fail3") }).await;

        let result = cb.call(|| async { Ok::<_, String>("ok3") }).await;
        assert!(matches!(result, Err(CircuitBreakerError::Open(_))));
    }

    #[tokio::test]
    async fn test_failures_expire_outside_sliding_window() {
        let mut config = fast_config();
        config.failure_window = Duration::from_millis(40);
        let cb = CircuitBreaker::new("test", config);

        // 2 failures
        let _ = cb.call(|| async { Err::<(), _>("fail") }).await;
        let _ = cb.call(|| async { Err::<(), _>("fail") }).await;

        // Sleep to let failures expire outside sliding window
        tokio::time::sleep(Duration::from_millis(60)).await;

        // 1 more failure (total within window is now only 1)
        let _ = cb.call(|| async { Err::<(), _>("fail") }).await;

        // Breaker should still be closed and allow request
        let result = cb.call(|| async { Ok::<_, String>("ok") }).await;
        assert!(result.is_ok());
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
}
