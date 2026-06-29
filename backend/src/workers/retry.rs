//! Retry logic with exponential backoff and jitter.
//!
//! This module provides a production-ready [`RetryPolicy`] that retries any
//! fallible async operation using truncated binary-exponential backoff with
//! full jitter, as recommended by AWS Architecture Blog.
//!
//! # Quick start
//!
//! ```rust,no_run
//! use backend::workers::retry::{RetryPolicy, RetryError};
//!
//! #[tokio::main]
//! async fn main() -> Result<(), RetryError<String>> {
//!     let policy = RetryPolicy::default();
//!
//!     let result = policy
//!         .retry(|| async {
//!             // Replace with your fallible operation
//!             Ok::<_, String>("success")
//!         })
//!         .await?;
//!
//!     println!("Got: {result}");
//!     Ok(())
//! }
//! ```
//!
//! # Backoff formula
//!
//! ```text
//! sleep = rand(0, min(cap, base * 2^attempt))
//! ```
//!
//! Where `cap` is [`RetryPolicy::max_delay`] and `base` is
//! [`RetryPolicy::base_delay`]. This gives uniform jitter across the
//! window so retries spread evenly under high concurrency.

use std::fmt;
use std::future::Future;
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use serde::{Deserialize, Serialize};
use thiserror::Error;
use tracing::{debug, instrument, warn};

// ---------------------------------------------------------------------------
// Error type
// ---------------------------------------------------------------------------

/// Error returned when all retry attempts are exhausted or retries are
/// explicitly halted by the caller's predicate.
#[derive(Debug, Error)]
pub enum RetryError<E> {
    /// Every attempt failed. Contains the last error.
    #[error("All {attempts} retry attempt(s) failed. Last error: {last_error}")]
    Exhausted {
        /// Number of attempts made (including the initial one).
        attempts: u32,
        /// The error returned by the final attempt.
        last_error: E,
    },

    /// The caller's [`ShouldRetry`] predicate returned `false`, aborting
    /// early with the given error.
    #[error("Retry aborted after {attempts} attempt(s): {last_error}")]
    Aborted {
        /// Number of attempts made before aborting.
        attempts: u32,
        /// The error that caused the abort.
        last_error: E,
    },
}

impl<E: fmt::Display> RetryError<E> {
    /// Returns the underlying last error regardless of variant.
    pub fn into_inner(self) -> E {
        match self {
            Self::Exhausted { last_error, .. } | Self::Aborted { last_error, .. } => last_error,
        }
    }

    /// Returns the number of attempts made.
    pub fn attempts(&self) -> u32 {
        match self {
            Self::Exhausted { attempts, .. } | Self::Aborted { attempts, .. } => *attempts,
        }
    }
}

// ---------------------------------------------------------------------------
// Should-retry predicate
// ---------------------------------------------------------------------------

/// Determines whether a given error warrants another retry attempt.
///
/// Return `true` to retry, `false` to abort immediately.
pub trait ShouldRetry<E> {
    /// Inspect `error` and decide whether to retry.
    fn should_retry(&self, error: &E) -> bool;
}

/// Blanket implementation: a bare `bool` always returns that value.
impl<E> ShouldRetry<E> for bool {
    fn should_retry(&self, _: &E) -> bool {
        *self
    }
}

/// Blanket implementation for closures `Fn(&E) -> bool`.
impl<E, F: Fn(&E) -> bool> ShouldRetry<E, > for F {
    fn should_retry(&self, error: &E) -> bool {
        (self)(error)
    }
}

// ---------------------------------------------------------------------------
// Retry outcome (internal)
// ---------------------------------------------------------------------------

/// Outcome of a single attempt — used internally to drive the retry loop.
#[derive(Debug)]
enum Attempt<T, E> {
    Ok(T),
    Retry(E),
    Abort(E),
}

// ---------------------------------------------------------------------------
// RetryConfig — serialisable snapshot of policy parameters
// ---------------------------------------------------------------------------

/// Serialisable configuration for a [`RetryPolicy`].
///
/// Useful for storing retry settings in databases, config files, or Redis.
///
/// # Example
///
/// ```rust
/// use backend::workers::retry::RetryConfig;
/// use std::time::Duration;
///
/// let cfg = RetryConfig {
///     max_attempts: 5,
///     base_delay_ms: 100,
///     max_delay_ms: 30_000,
///     multiplier: 2.0,
/// };
/// assert_eq!(cfg.max_attempts, 5);
/// ```
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct RetryConfig {
    /// Maximum total attempts (including the first). Must be ≥ 1.
    pub max_attempts: u32,
    /// Initial delay in milliseconds before the first retry.
    pub base_delay_ms: u64,
    /// Upper bound on the computed delay in milliseconds.
    pub max_delay_ms: u64,
    /// Exponential growth factor applied per attempt.
    pub multiplier: f64,
}

impl Default for RetryConfig {
    fn default() -> Self {
        Self {
            max_attempts: 3,
            base_delay_ms: 100,
            max_delay_ms: 30_000,
            multiplier: 2.0,
        }
    }
}

// ---------------------------------------------------------------------------
// RetryPolicy
// ---------------------------------------------------------------------------

/// Policy governing how—and how many times—a failing operation is retried.
///
/// Build with [`RetryPolicy::new`] or start from [`RetryPolicy::default`] and
/// refine with the builder methods.
///
/// # Design
///
/// The policy implements *full jitter* exponential backoff:
///
/// ```text
/// window  = min(max_delay, base_delay * multiplier^attempt)
/// sleep   = rand(0, window)
/// ```
///
/// Because jitter covers the full window, concurrent callers will rarely
/// collide on the same retry slot, avoiding thundering-herd effects.
///
/// Jitter is derived from the sub-nanosecond component of the system clock
/// (`SystemTime::now()`), which provides sufficient entropy in practice
/// without requiring the `rand` crate.
///
/// # Examples
///
/// ```rust
/// use backend::workers::retry::RetryPolicy;
/// use std::time::Duration;
///
/// // Three attempts, 50 ms base, 10 s cap, 2× growth, retry any error.
/// let policy = RetryPolicy::new(3)
///     .with_base_delay(Duration::from_millis(50))
///     .with_max_delay(Duration::from_secs(10))
///     .with_multiplier(2.0);
/// ```
#[derive(Debug, Clone)]
pub struct RetryPolicy {
    /// Maximum total number of attempts (1 = no retries).
    pub max_attempts: u32,
    /// Initial backoff window before the first retry.
    pub base_delay: Duration,
    /// Upper bound on the backoff window.
    pub max_delay: Duration,
    /// Exponential growth factor.
    pub multiplier: f64,
}

impl Default for RetryPolicy {
    /// Returns a conservative default: 3 attempts, 100 ms base, 30 s cap.
    fn default() -> Self {
        Self {
            max_attempts: 3,
            base_delay: Duration::from_millis(100),
            max_delay: Duration::from_secs(30),
            multiplier: 2.0,
        }
    }
}

impl RetryPolicy {
    /// Create a policy with `max_attempts` total attempts and all other
    /// settings at their defaults.
    pub fn new(max_attempts: u32) -> Self {
        Self {
            max_attempts: max_attempts.max(1),
            ..Default::default()
        }
    }

    /// Build a [`RetryPolicy`] from a [`RetryConfig`].
    pub fn from_config(cfg: RetryConfig) -> Self {
        Self {
            max_attempts: cfg.max_attempts.max(1),
            base_delay: Duration::from_millis(cfg.base_delay_ms),
            max_delay: Duration::from_millis(cfg.max_delay_ms),
            multiplier: cfg.multiplier,
        }
    }

    /// Serialise this policy into a [`RetryConfig`].
    pub fn to_config(&self) -> RetryConfig {
        RetryConfig {
            max_attempts: self.max_attempts,
            base_delay_ms: self.base_delay.as_millis() as u64,
            max_delay_ms: self.max_delay.as_millis() as u64,
            multiplier: self.multiplier,
        }
    }

    // ---- Builder methods ---------------------------------------------------

    /// Set the base (initial) backoff delay.
    pub fn with_base_delay(mut self, d: Duration) -> Self {
        self.base_delay = d;
        self
    }

    /// Set the maximum backoff cap.
    pub fn with_max_delay(mut self, d: Duration) -> Self {
        self.max_delay = d;
        self
    }

    /// Set the exponential growth multiplier (default: 2.0).
    pub fn with_multiplier(mut self, m: f64) -> Self {
        self.multiplier = m.max(1.0);
        self
    }

    // ---- Core retry loop ---------------------------------------------------

    /// Retry `operation` according to this policy, retrying on every error.
    ///
    /// Returns `Ok(T)` on the first success, or
    /// [`RetryError::Exhausted`] when all attempts fail.
    ///
    /// # Tracing
    ///
    /// Each attempt is logged at `DEBUG` level; failures are logged at `WARN`.
    #[instrument(
        name = "retry.execute",
        skip(self, operation),
        fields(
            retry.max_attempts = self.max_attempts,
            retry.base_delay_ms = self.base_delay.as_millis() as u64,
        )
    )]
    pub async fn retry<F, Fut, T, E>(&self, operation: F) -> Result<T, RetryError<E>>
    where
        F: FnMut() -> Fut,
        Fut: Future<Output = Result<T, E>>,
        E: fmt::Display,
    {
        self.retry_if(operation, |_: &E| true).await
    }

    /// Retry `operation` only when `should_retry` returns `true`.
    ///
    /// If the predicate returns `false` for a given error the loop stops
    /// immediately and returns [`RetryError::Aborted`].
    ///
    /// # Examples
    ///
    /// ```rust,no_run
    /// use backend::workers::retry::{RetryPolicy, RetryError};
    ///
    /// #[derive(Debug)]
    /// enum MyError { Transient, Permanent }
    ///
    /// impl std::fmt::Display for MyError {
    ///     fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
    ///         write!(f, "{:?}", self)
    ///     }
    /// }
    ///
    /// # #[tokio::main]
    /// # async fn main() {
    /// let policy = RetryPolicy::default();
    /// let _ = policy.retry_if(
    ///     || async { Err::<(), _>(MyError::Transient) },
    ///     |e| matches!(e, MyError::Transient),
    /// ).await;
    /// # }
    /// ```
    #[instrument(
        name = "retry.execute_if",
        skip(self, operation, should_retry),
        fields(
            retry.max_attempts = self.max_attempts,
        )
    )]
    pub async fn retry_if<F, Fut, T, E, P>(
        &self,
        mut operation: F,
        should_retry: P,
    ) -> Result<T, RetryError<E>>
    where
        F: FnMut() -> Fut,
        Fut: Future<Output = Result<T, E>>,
        E: fmt::Display,
        P: ShouldRetry<E>,
    {
        let mut attempt = 0u32;

        loop {
            attempt += 1;
            debug!(attempt, max = self.max_attempts, "retry: attempting operation");

            match operation().await {
                Ok(value) => {
                    debug!(attempt, "retry: operation succeeded");
                    return Ok(value);
                }
                Err(err) => {
                    if !should_retry.should_retry(&err) {
                        warn!(
                            attempt,
                            error = %err,
                            "retry: predicate rejected error — aborting"
                        );
                        return Err(RetryError::Aborted {
                            attempts: attempt,
                            last_error: err,
                        });
                    }

                    if attempt >= self.max_attempts {
                        warn!(
                            attempt,
                            max = self.max_attempts,
                            error = %err,
                            "retry: all attempts exhausted"
                        );
                        return Err(RetryError::Exhausted {
                            attempts: attempt,
                            last_error: err,
                        });
                    }

                    let delay = self.compute_delay(attempt);
                    warn!(
                        attempt,
                        next_delay_ms = delay.as_millis() as u64,
                        error = %err,
                        "retry: attempt failed — backing off"
                    );
                    tokio::time::sleep(delay).await;
                }
            }
        }
    }

    // ---- Delay calculation -------------------------------------------------

    /// Compute the jittered backoff duration for the given attempt number.
    ///
    /// Uses full-jitter strategy:
    /// ```text
    /// window = min(max_delay, base_delay * multiplier^(attempt-1))
    /// sleep  = rand(0, window)
    /// ```
    ///
    /// Entropy is derived from `SystemTime` sub-nanoseconds, giving adequate
    /// spread without a dedicated random-number crate.
    pub fn compute_delay(&self, attempt: u32) -> Duration {
        // Saturating exponentiation: base * multiplier^(attempt-1)
        let exp = (self.multiplier).powi((attempt.saturating_sub(1)) as i32);
        let window_ms = (self.base_delay.as_millis() as f64 * exp)
            .min(self.max_delay.as_millis() as f64) as u64;

        if window_ms == 0 {
            return Duration::ZERO;
        }

        // Derive jitter from system clock nanoseconds (no `rand` dependency).
        let nanos = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap_or_default()
            .subsec_nanos() as u64;

        // Mix with attempt to spread concurrent callers at the same instant.
        let mixed = nanos.wrapping_add((attempt as u64).wrapping_mul(6_364_136_223_846_793_005));
        let jitter_ms = mixed % window_ms;

        Duration::from_millis(jitter_ms)
    }
}

// ---------------------------------------------------------------------------
// Convenience free function
// ---------------------------------------------------------------------------

/// Retry `operation` with the default [`RetryPolicy`].
///
/// Equivalent to `RetryPolicy::default().retry(operation).await`.
///
/// # Errors
///
/// Returns [`RetryError::Exhausted`] when all default attempts fail.
pub async fn retry<F, Fut, T, E>(operation: F) -> Result<T, RetryError<E>>
where
    F: FnMut() -> Fut,
    Fut: Future<Output = Result<T, E>>,
    E: fmt::Display,
{
    RetryPolicy::default().retry(operation).await
}

// ---------------------------------------------------------------------------
// Unit tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::{AtomicU32, Ordering};
    use std::sync::Arc;

    // ---- helpers -----------------------------------------------------------

    fn policy(max: u32) -> RetryPolicy {
        RetryPolicy::new(max)
            .with_base_delay(Duration::from_millis(1))
            .with_max_delay(Duration::from_millis(10))
    }

    // ---- compute_delay -----------------------------------------------------

    #[test]
    fn delay_is_within_window() {
        let p = RetryPolicy::default();
        for attempt in 1..=10 {
            let d = p.compute_delay(attempt);
            assert!(
                d <= p.max_delay,
                "attempt {attempt}: delay {d:?} exceeds max {:?}",
                p.max_delay
            );
        }
    }

    #[test]
    fn delay_zero_when_window_zero() {
        let p = RetryPolicy::new(3).with_base_delay(Duration::ZERO);
        assert_eq!(p.compute_delay(1), Duration::ZERO);
    }

    #[test]
    fn delay_capped_at_max() {
        let p = RetryPolicy::new(3)
            .with_base_delay(Duration::from_secs(1))
            .with_max_delay(Duration::from_millis(500));
        // After many doublings the window is always ≤ max_delay.
        for attempt in 1..=20 {
            assert!(p.compute_delay(attempt) <= p.max_delay);
        }
    }

    // ---- success on first try ----------------------------------------------

    #[tokio::test]
    async fn succeeds_immediately() {
        let calls = Arc::new(AtomicU32::new(0));
        let c = calls.clone();

        let result = policy(3)
            .retry(|| {
                let c = c.clone();
                async move {
                    c.fetch_add(1, Ordering::SeqCst);
                    Ok::<_, String>("ok")
                }
            })
            .await;

        assert!(result.is_ok());
        assert_eq!(result.unwrap(), "ok");
        assert_eq!(calls.load(Ordering::SeqCst), 1);
    }

    // ---- success on second try ---------------------------------------------

    #[tokio::test]
    async fn succeeds_after_one_failure() {
        let calls = Arc::new(AtomicU32::new(0));
        let c = calls.clone();

        let result = policy(3)
            .retry(|| {
                let c = c.clone();
                async move {
                    let n = c.fetch_add(1, Ordering::SeqCst);
                    if n == 0 {
                        Err("first fail".to_string())
                    } else {
                        Ok("second try")
                    }
                }
            })
            .await;

        assert!(result.is_ok());
        assert_eq!(calls.load(Ordering::SeqCst), 2);
    }

    // ---- exhausted ---------------------------------------------------------

    #[tokio::test]
    async fn exhausts_all_attempts() {
        let calls = Arc::new(AtomicU32::new(0));
        let c = calls.clone();

        let result = policy(3)
            .retry::<_, _, (), _>(|| {
                let c = c.clone();
                async move {
                    c.fetch_add(1, Ordering::SeqCst);
                    Err("always fails".to_string())
                }
            })
            .await;

        assert!(result.is_err());
        let err = result.unwrap_err();
        assert_eq!(err.attempts(), 3);
        assert!(matches!(err, RetryError::Exhausted { .. }));
        assert_eq!(calls.load(Ordering::SeqCst), 3);
    }

    // ---- abort on non-retryable error -------------------------------------

    #[tokio::test]
    async fn aborts_on_permanent_error() {
        let calls = Arc::new(AtomicU32::new(0));
        let c = calls.clone();

        let result = policy(5)
            .retry_if::<_, _, (), _, _>(
                || {
                    let c = c.clone();
                    async move {
                        c.fetch_add(1, Ordering::SeqCst);
                        Err("permanent".to_string())
                    }
                },
                |e: &String| e.contains("transient"),
            )
            .await;

        assert!(matches!(result, Err(RetryError::Aborted { attempts: 1, .. })));
        assert_eq!(calls.load(Ordering::SeqCst), 1);
    }

    // ---- retry_if: retries transient, aborts permanent --------------------

    #[tokio::test]
    async fn retries_transient_aborts_permanent() {
        let calls = Arc::new(AtomicU32::new(0));
        let c = calls.clone();

        // Fail twice with "transient", then fail with "permanent".
        let result = policy(5)
            .retry_if::<_, _, (), _, _>(
                || {
                    let c = c.clone();
                    async move {
                        let n = c.fetch_add(1, Ordering::SeqCst);
                        if n < 2 {
                            Err("transient error".to_string())
                        } else {
                            Err("permanent error".to_string())
                        }
                    }
                },
                |e: &String| e.contains("transient"),
            )
            .await;

        assert!(matches!(result, Err(RetryError::Aborted { attempts: 3, .. })));
    }

    // ---- single attempt means no retries ----------------------------------

    #[tokio::test]
    async fn single_attempt_no_retry() {
        let calls = Arc::new(AtomicU32::new(0));
        let c = calls.clone();

        let result = policy(1)
            .retry::<_, _, (), _>(|| {
                let c = c.clone();
                async move {
                    c.fetch_add(1, Ordering::SeqCst);
                    Err("fail".to_string())
                }
            })
            .await;

        assert!(result.is_err());
        assert_eq!(calls.load(Ordering::SeqCst), 1);
    }

    // ---- config round-trip ------------------------------------------------

    #[test]
    fn config_round_trip() {
        let original = RetryPolicy::new(7)
            .with_base_delay(Duration::from_millis(200))
            .with_max_delay(Duration::from_secs(60))
            .with_multiplier(3.0);

        let cfg = original.to_config();
        assert_eq!(cfg.max_attempts, 7);
        assert_eq!(cfg.base_delay_ms, 200);
        assert_eq!(cfg.max_delay_ms, 60_000);
        assert_eq!(cfg.multiplier, 3.0);

        let restored = RetryPolicy::from_config(cfg.clone());
        assert_eq!(restored.max_attempts, cfg.max_attempts);
        assert_eq!(restored.base_delay, Duration::from_millis(cfg.base_delay_ms));
        assert_eq!(restored.max_delay, Duration::from_millis(cfg.max_delay_ms));
    }

    #[test]
    fn default_config_serializes() {
        let cfg = RetryConfig::default();
        let json = serde_json::to_string(&cfg).expect("serialize");
        let back: RetryConfig = serde_json::from_str(&json).expect("deserialize");
        assert_eq!(cfg, back);
    }

    // ---- RetryError helpers -----------------------------------------------

    #[test]
    fn retry_error_into_inner_exhausted() {
        let err: RetryError<String> = RetryError::Exhausted {
            attempts: 3,
            last_error: "boom".into(),
        };
        assert_eq!(err.into_inner(), "boom");
    }

    #[test]
    fn retry_error_into_inner_aborted() {
        let err: RetryError<String> = RetryError::Aborted {
            attempts: 1,
            last_error: "nope".into(),
        };
        assert_eq!(err.into_inner(), "nope");
    }

    #[test]
    fn retry_error_attempts_count() {
        let e: RetryError<&str> = RetryError::Exhausted { attempts: 5, last_error: "x" };
        assert_eq!(e.attempts(), 5);
        let a: RetryError<&str> = RetryError::Aborted { attempts: 2, last_error: "y" };
        assert_eq!(a.attempts(), 2);
    }

    #[test]
    fn retry_error_display() {
        let err: RetryError<String> = RetryError::Exhausted {
            attempts: 3,
            last_error: "db timeout".into(),
        };
        let msg = err.to_string();
        assert!(msg.contains("3"));
        assert!(msg.contains("db timeout"));
    }

    // ---- bool ShouldRetry -------------------------------------------------

    #[test]
    fn bool_should_retry_true() {
        let p = true;
        assert!(p.should_retry(&"anything"));
    }

    #[test]
    fn bool_should_retry_false() {
        let p = false;
        assert!(!p.should_retry(&"anything"));
    }

    // ---- multiplier floor -------------------------------------------------

    #[test]
    fn multiplier_clamped_to_one() {
        let p = RetryPolicy::new(3).with_multiplier(0.1);
        assert!(p.multiplier >= 1.0);
    }

    // ---- max_attempts floor -----------------------------------------------

    #[test]
    fn max_attempts_floor_is_one() {
        let p = RetryPolicy::new(0);
        assert_eq!(p.max_attempts, 1);
    }

    // ---- convenience free function ----------------------------------------

    #[tokio::test]
    async fn free_retry_succeeds() {
        let result = retry(|| async { Ok::<_, String>("free") }).await;
        assert_eq!(result.unwrap(), "free");
    }
}
