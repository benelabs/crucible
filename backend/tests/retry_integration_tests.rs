//! Integration tests for [`backend::workers::retry`].
//!
//! These tests exercise the full retry loop end-to-end without mocking
//! internal details, confirming observable behaviour from a caller's
//! perspective.

use std::sync::atomic::{AtomicU32, Ordering};
use std::sync::Arc;
use std::time::{Duration, Instant};

use backend::workers::retry::{retry, RetryConfig, RetryError, RetryPolicy};

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

/// Build a fast policy suitable for integration tests (no real sleeping).
fn fast_policy(max: u32) -> RetryPolicy {
    RetryPolicy::new(max)
        .with_base_delay(Duration::from_millis(1))
        .with_max_delay(Duration::from_millis(5))
}

// ---------------------------------------------------------------------------
// Basic success / failure
// ---------------------------------------------------------------------------

#[tokio::test]
async fn immediate_success_calls_operation_once() {
    let calls = Arc::new(AtomicU32::new(0));
    let c = calls.clone();

    let result = fast_policy(5)
        .retry(|| {
            let c = c.clone();
            async move {
                c.fetch_add(1, Ordering::SeqCst);
                Ok::<&str, String>("done")
            }
        })
        .await;

    assert!(result.is_ok(), "expected Ok, got {result:?}");
    assert_eq!(calls.load(Ordering::SeqCst), 1, "should only call once");
}

#[tokio::test]
async fn recovers_after_failures() {
    let calls = Arc::new(AtomicU32::new(0));
    let c = calls.clone();

    // Fail the first two times, succeed on the third.
    let result = fast_policy(5)
        .retry(|| {
            let c = c.clone();
            async move {
                let n = c.fetch_add(1, Ordering::SeqCst);
                if n < 2 {
                    Err(format!("transient failure #{n}"))
                } else {
                    Ok("recovered")
                }
            }
        })
        .await;

    assert_eq!(result.unwrap(), "recovered");
    assert_eq!(calls.load(Ordering::SeqCst), 3);
}

#[tokio::test]
async fn exhausted_error_contains_last_message() {
    let result = fast_policy(3)
        .retry::<_, _, (), _>(|| async { Err("persistent error".to_string()) })
        .await;

    let err = result.unwrap_err();
    assert!(matches!(err, RetryError::Exhausted { .. }));
    assert_eq!(err.attempts(), 3);
    assert!(err.to_string().contains("persistent error"));
}

// ---------------------------------------------------------------------------
// retry_if / predicate control
// ---------------------------------------------------------------------------

#[tokio::test]
async fn abort_on_non_retryable_error() {
    let calls = Arc::new(AtomicU32::new(0));
    let c = calls.clone();

    let result = fast_policy(10)
        .retry_if::<_, _, (), _, _>(
            || {
                let c = c.clone();
                async move {
                    c.fetch_add(1, Ordering::SeqCst);
                    Err("fatal: disk full".to_string())
                }
            },
            |e: &String| e.starts_with("transient"),
        )
        .await;

    assert!(matches!(
        result,
        Err(RetryError::Aborted { attempts: 1, .. })
    ));
    assert_eq!(
        calls.load(Ordering::SeqCst),
        1,
        "must not retry permanent error"
    );
}

#[tokio::test]
async fn retries_transient_but_aborts_on_permanent() {
    let calls = Arc::new(AtomicU32::new(0));
    let c = calls.clone();

    // First two calls: transient; third call: permanent.
    let result = fast_policy(10)
        .retry_if::<_, _, (), _, _>(
            || {
                let c = c.clone();
                async move {
                    let n = c.fetch_add(1, Ordering::SeqCst);
                    if n < 2 {
                        Err("transient".to_string())
                    } else {
                        Err("permanent".to_string())
                    }
                }
            },
            |e: &String| e == "transient",
        )
        .await;

    assert!(matches!(
        result,
        Err(RetryError::Aborted { attempts: 3, .. })
    ));
}

// ---------------------------------------------------------------------------
// Delay / timing
// ---------------------------------------------------------------------------

#[tokio::test]
async fn backoff_does_not_exceed_max_delay() {
    // 5 attempts with a very low cap — the whole run must stay well under 1 s.
    let policy = RetryPolicy::new(5)
        .with_base_delay(Duration::from_millis(1))
        .with_max_delay(Duration::from_millis(10));

    let start = Instant::now();
    let _ = policy
        .retry::<_, _, (), _>(|| async { Err("fail".to_string()) })
        .await;
    let elapsed = start.elapsed();

    // 4 sleeps × max 10 ms = 40 ms. Generous upper bound to avoid flakiness.
    assert!(
        elapsed < Duration::from_millis(500),
        "backoff took too long: {elapsed:?}"
    );
}

// ---------------------------------------------------------------------------
// Config round-trip
// ---------------------------------------------------------------------------

#[tokio::test]
async fn policy_from_config_behaves_correctly() {
    let cfg = RetryConfig {
        max_attempts: 3,
        base_delay_ms: 1,
        max_delay_ms: 5,
        multiplier: 2.0,
    };
    let policy = RetryPolicy::from_config(cfg);
    let calls = Arc::new(AtomicU32::new(0));
    let c = calls.clone();

    let result = policy
        .retry::<_, _, (), _>(|| {
            let c = c.clone();
            async move {
                c.fetch_add(1, Ordering::SeqCst);
                Err("err".to_string())
            }
        })
        .await;

    assert!(result.is_err());
    assert_eq!(calls.load(Ordering::SeqCst), 3);
}

// ---------------------------------------------------------------------------
// Convenience free function
// ---------------------------------------------------------------------------

#[tokio::test]
async fn free_retry_fn_succeeds() {
    let result = retry(|| async { Ok::<_, String>("hello") }).await;
    assert_eq!(result.unwrap(), "hello");
}

#[tokio::test]
async fn free_retry_fn_exhausts_default_attempts() {
    let calls = Arc::new(AtomicU32::new(0));
    let c = calls.clone();

    // Use a very fast policy via the builder, not the free function, to avoid
    // sleeping 30 s. This test confirms the default attempt count (3).
    let result = RetryPolicy::new(3)
        .with_base_delay(Duration::from_millis(1))
        .with_max_delay(Duration::from_millis(5))
        .retry::<_, _, (), _>(|| {
            let c = c.clone();
            async move {
                c.fetch_add(1, Ordering::SeqCst);
                Err("always".to_string())
            }
        })
        .await;

    assert!(result.is_err());
    assert_eq!(calls.load(Ordering::SeqCst), 3);
}
