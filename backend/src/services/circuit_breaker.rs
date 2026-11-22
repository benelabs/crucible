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

use std::collections::VecDeque;

/// Configuration for a circuit breaker.
#[derive(Debug, Clone)]
pub struct CircuitBreakerConfig {
    /// Number of failures within the sliding window before opening the circuit.
    pub failure_threshold: u32,
    /// How long to wait in Open state before transitioning to HalfOpen.
    pub recovery_timeout: Duration,
    /// Number of consecutive successes in HalfOpen before closing.
    pub success_threshold: u32,
    /// Sliding time window for failure counting in Closed state.
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
    probe_in_flight: bool,
    config: CircuitBreakerConfig,
}

impl CircuitBreakerInner {
    fn new(config: CircuitBreakerConfig) -> Self {
        Self {
            state: CircuitState::Closed,
            failure_timestamps: VecDeque::new(),
            consecutive_successes: 0,
            probe_in_flight: false,
            config,
        }
    }

    fn prune_expired_failures(&mut self, now: Instant) {
        let cutoff = now.checked_sub(self.config.failure_window).unwrap_or(now);
        while let Some(&ts) = self.failure_timestamps.front() {
            if ts < cutoff {
                self.failure_timestamps.pop_front();
            } else {
                break;
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
                    self.probe_in_flight = true;
                    info!("Circuit breaker transitioning to HalfOpen");
                    true
                } else {
                    false
                }
            }
            CircuitState::HalfOpen => {
                if !self.probe_in_flight {
                    self.probe_in_flight = true;
                    true
                } else {
                    false
                }
            }
        }
    }

    fn record_success(&mut self) {
        match self.state {
            CircuitState::HalfOpen => {
                self.probe_in_flight = false;
                self.consecutive_successes += 1;
                if self.consecutive_successes >= self.config.success_threshold {
                    self.state = CircuitState::Closed;
                    self.failure_timestamps.clear();
                    self.consecutive_successes = 0;
                    info!("Circuit breaker closed after successful recovery");
                }
            }
            CircuitState::Closed => {
                // In Closed state, do not immediately wipe out the failure counter on a single isolated success.
                // Instead, prune only failures that have aged out of the sliding time window.
                // This prevents fluttering/flaky services from escaping the threshold.
                let now = Instant::now();
                self.prune_expired_failures(now);
            }
            _ => {}
        }
    }

    fn record_failure(&mut self) {
        let now = Instant::now();
        match self.state {
            CircuitState::Closed => {
                self.prune_expired_failures(now);
                self.failure_timestamps.push_back(now);
                if self.failure_timestamps.len() >= self.config.failure_threshold as usize {
                    self.state = CircuitState::Open { opened_at: now };
                    let count = self.failure_timestamps.len();
                    self.failure_timestamps.clear();
                    warn!(
                        failures = count,
                        window_secs = self.config.failure_window.as_secs(),
                        "Circuit breaker opened after threshold exceeded in sliding window"
                    );
                }
            }
            CircuitState::HalfOpen => {
                self.probe_in_flight = false;
                self.state = CircuitState::Open { opened_at: now };
                self.failure_timestamps.clear();
                self.consecutive_successes = 0;
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

    /// Returns the number of failures currently recorded in the sliding window.
    pub async fn failure_count(&self) -> usize {
        self.inner.read().await.failure_timestamps.len()
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
            failure_window: Duration::from_secs(60),
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
    async fn test_half_open_limits_concurrency_to_single_probe() {
        let cb = CircuitBreaker::new("test", fast_config());
        for _ in 0..3 {
            let _ = cb.call(|| async { Err::<(), _>("fail") }).await;
        }
        assert_eq!(cb.state_label().await, "open");

        // Wait for recovery timeout
        tokio::time::sleep(Duration::from_millis(60)).await;

        let (started_tx, started_rx) = tokio::sync::oneshot::channel();
        let (release_tx, release_rx) = tokio::sync::oneshot::channel();

        let cb_clone = cb.clone();
        let probe_handle = tokio::spawn(async move {
            cb_clone
                .call(|| async move {
                    let _ = started_tx.send(());
                    let _ = release_rx.await;
                    Ok::<_, String>("probe_ok")
                })
                .await
        });

        // Wait for probe to start and acquire the canary slot
        started_rx.await.unwrap();

        // While probe is in-flight in HalfOpen, concurrent requests must be rejected
        let concurrent_result = cb.call(|| async { Ok::<_, String>("concurrent") }).await;
        assert!(
            matches!(concurrent_result, Err(CircuitBreakerError::Open(_))),
            "Concurrent requests in HalfOpen must be rejected while probe is in flight"
        );

        // Allow probe to finish
        let _ = release_tx.send(());
        let probe_result = probe_handle.await.unwrap();
        assert!(probe_result.is_ok());

        // Probe succeeded, so with success_threshold = 1, circuit is now closed
        assert_eq!(cb.state_label().await, "closed");
        let next_result = cb.call(|| async { Ok::<_, String>("next") }).await;
        assert!(next_result.is_ok());
    }

    #[tokio::test]
    async fn test_half_open_probe_failure_reopens_circuit() {
        let cb = CircuitBreaker::new("test", fast_config());
        for _ in 0..3 {
            let _ = cb.call(|| async { Err::<(), _>("fail") }).await;
        }
        tokio::time::sleep(Duration::from_millis(60)).await;

        let probe_result = cb.call(|| async { Err::<(), _>("probe_failed") }).await;
        assert!(matches!(
            probe_result,
            Err(CircuitBreakerError::ServiceError(_))
        ));

        // Circuit must immediately be Open again
        assert_eq!(cb.state_label().await, "open");
        let next_result = cb.call(|| async { Ok::<_, String>("next") }).await;
        assert!(matches!(next_result, Err(CircuitBreakerError::Open(_))));
    }

    #[tokio::test]
    async fn test_sliding_window_catches_fluttering_failures_interspersed_with_successes() {
        let cb = CircuitBreaker::new("test", fast_config());
        // Flaky service alternates: fail, success, fail, success, fail
        let _ = cb.call(|| async { Err::<(), _>("fail 1") }).await;
        let _ = cb.call(|| async { Ok::<_, String>("success 1") }).await;
        let _ = cb.call(|| async { Err::<(), _>("fail 2") }).await;
        let _ = cb.call(|| async { Ok::<_, String>("success 2") }).await;
        let _ = cb.call(|| async { Err::<(), _>("fail 3") }).await;

        // With sliding window, the circuit breaker opens because 3 failures occurred within the window.
        let result = cb.call(|| async { Ok::<_, String>("ok") }).await;
        assert!(matches!(result, Err(CircuitBreakerError::Open(_))));
        assert_eq!(cb.state_label().await, "open");
    }

    #[tokio::test]
    async fn test_fluttering_failures_trip_breaker_sliding_window() {
        let cb = CircuitBreaker::new("test", fast_config());
        let _ = cb.call(|| async { Err::<(), _>("fail") }).await;
        let _ = cb.call(|| async { Err::<(), _>("fail") }).await;
        assert_eq!(cb.failure_count().await, 2);

        // A single success must not reset failure history in Closed state.
        let ok_res = cb.call(|| async { Ok::<_, String>("ok") }).await;
        assert!(ok_res.is_ok());
        assert_eq!(cb.failure_count().await, 2);

        let _ = cb.call(|| async { Err::<(), _>("fail") }).await;
        assert_eq!(cb.state_label().await, "open");

        let blocked = cb.call(|| async { Ok::<_, String>("ok") }).await;
        assert!(matches!(blocked, Err(CircuitBreakerError::Open(_))));
    }

    #[tokio::test]
    async fn test_failures_expire_outside_sliding_window() {
        let config = CircuitBreakerConfig {
            failure_threshold: 3,
            recovery_timeout: Duration::from_millis(50),
            success_threshold: 1,
            failure_window: Duration::from_millis(60),
        };
        let cb = CircuitBreaker::new("test", config);

        let _ = cb.call(|| async { Err::<(), _>("fail") }).await;
        let _ = cb.call(|| async { Err::<(), _>("fail") }).await;
        assert_eq!(cb.failure_count().await, 2);

        tokio::time::sleep(Duration::from_millis(80)).await;

        let ok_res = cb.call(|| async { Ok::<_, String>("ok") }).await;
        assert!(ok_res.is_ok());
        assert_eq!(cb.failure_count().await, 0);

        let _ = cb.call(|| async { Err::<(), _>("fail") }).await;
        assert_eq!(cb.failure_count().await, 1);
        assert_eq!(cb.state_label().await, "closed");
    }

    #[tokio::test]
    async fn test_recovery_clears_sliding_window() {
        let cb = CircuitBreaker::new("test", fast_config());
        for _ in 0..3 {
            let _ = cb.call(|| async { Err::<(), _>("fail") }).await;
        }
        assert_eq!(cb.state_label().await, "open");

        tokio::time::sleep(Duration::from_millis(60)).await;

        let probe = cb.call(|| async { Ok::<_, String>("ok") }).await;
        assert!(probe.is_ok());
        assert_eq!(cb.state_label().await, "closed");
        assert_eq!(cb.failure_count().await, 0);
    }
}
