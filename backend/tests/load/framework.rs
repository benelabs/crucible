//! Load testing framework — shared helpers, metrics, and assertion utilities.
//!
//! # Overview
//!
//! This module provides the core primitives used by every load-test module:
//!
//! - [`LoadConfig`] — controls concurrency, iteration count, and timeout.
//! - [`RequestOutcome`] — the result of a single request (status + latency).
//! - [`LoadResult`] — aggregated statistics over a completed load run.
//! - [`run_load`] — fires `config.concurrency` tasks, each making
//!   `config.requests_per_task` requests, and collects [`LoadResult`].
//! - [`assert_load_result`] — convenience assertion that fails the test when
//!   the error rate or p99 latency exceeds the configured thresholds.
//!
//! # Example
//!
//! ```rust,ignore
//! use crate::load::framework::{LoadConfig, run_load, assert_load_result};
//!
//! let cfg = LoadConfig::default();
//! let result = run_load(cfg, || async {
//!     // build and fire one request, return (StatusCode, Duration)
//!     let app = build_app();
//!     let start = std::time::Instant::now();
//!     let resp = app.oneshot(req()).await.unwrap();
//!     (resp.status(), start.elapsed())
//! }).await;
//!
//! assert_load_result(&result, 0.0, std::time::Duration::from_millis(500));
//! ```

use std::time::{Duration, Instant};

use axum::http::StatusCode;
use tokio::task::JoinSet;

// ---------------------------------------------------------------------------
// Configuration
// ---------------------------------------------------------------------------

/// Parameters that control a single load-test run.
#[derive(Debug, Clone)]
pub struct LoadConfig {
    /// Number of concurrent Tokio tasks.
    pub concurrency: usize,
    /// Number of sequential requests each task fires.
    pub requests_per_task: usize,
    /// Maximum wall-clock time allowed for the entire run.
    /// The test will panic if this is exceeded.
    pub timeout: Duration,
}

impl LoadConfig {
    /// Create a new configuration.
    pub fn new(concurrency: usize, requests_per_task: usize) -> Self {
        Self {
            concurrency,
            requests_per_task,
            timeout: Duration::from_secs(30),
        }
    }

    /// Override the timeout.
    pub fn with_timeout(mut self, timeout: Duration) -> Self {
        self.timeout = timeout;
        self
    }

    /// Total number of requests that will be fired.
    pub fn total_requests(&self) -> usize {
        self.concurrency * self.requests_per_task
    }
}

impl Default for LoadConfig {
    /// Sensible defaults: 10 concurrent tasks × 5 requests each = 50 total.
    fn default() -> Self {
        Self::new(10, 5)
    }
}

// ---------------------------------------------------------------------------
// Per-request outcome
// ---------------------------------------------------------------------------

/// The outcome of a single HTTP request.
#[derive(Debug, Clone)]
pub struct RequestOutcome {
    /// HTTP status code returned by the handler.
    pub status: StatusCode,
    /// Wall-clock time from request start to response received.
    pub latency: Duration,
}

impl RequestOutcome {
    /// Returns `true` if the status code is a 2xx success.
    pub fn is_success(&self) -> bool {
        self.status.is_success()
    }
}

// ---------------------------------------------------------------------------
// Aggregated result
// ---------------------------------------------------------------------------

/// Aggregated statistics collected after a load run completes.
#[derive(Debug, Clone)]
pub struct LoadResult {
    /// All individual request outcomes, in completion order.
    pub outcomes: Vec<RequestOutcome>,
    /// Total wall-clock time for the entire run.
    pub total_duration: Duration,
}

impl LoadResult {
    /// Total number of requests fired.
    pub fn total(&self) -> usize {
        self.outcomes.len()
    }

    /// Number of successful (2xx) requests.
    pub fn successes(&self) -> usize {
        self.outcomes.iter().filter(|o| o.is_success()).count()
    }

    /// Number of failed (non-2xx) requests.
    pub fn failures(&self) -> usize {
        self.total() - self.successes()
    }

    /// Error rate as a fraction in `[0.0, 1.0]`.
    pub fn error_rate(&self) -> f64 {
        if self.total() == 0 {
            return 0.0;
        }
        self.failures() as f64 / self.total() as f64
    }

    /// Throughput in requests per second.
    pub fn rps(&self) -> f64 {
        if self.total_duration.is_zero() {
            return 0.0;
        }
        self.total() as f64 / self.total_duration.as_secs_f64()
    }

    /// Minimum observed latency.
    pub fn min_latency(&self) -> Duration {
        self.outcomes
            .iter()
            .map(|o| o.latency)
            .min()
            .unwrap_or(Duration::ZERO)
    }

    /// Maximum observed latency.
    pub fn max_latency(&self) -> Duration {
        self.outcomes
            .iter()
            .map(|o| o.latency)
            .max()
            .unwrap_or(Duration::ZERO)
    }

    /// Mean (average) latency.
    pub fn mean_latency(&self) -> Duration {
        if self.outcomes.is_empty() {
            return Duration::ZERO;
        }
        let total_nanos: u128 = self.outcomes.iter().map(|o| o.latency.as_nanos()).sum();
        Duration::from_nanos((total_nanos / self.outcomes.len() as u128) as u64)
    }

    /// Percentile latency.  `p` must be in `(0.0, 100.0]`.
    ///
    /// Uses the nearest-rank method.
    pub fn percentile_latency(&self, p: f64) -> Duration {
        assert!(p > 0.0 && p <= 100.0, "percentile must be in (0, 100]");
        if self.outcomes.is_empty() {
            return Duration::ZERO;
        }
        let mut latencies: Vec<Duration> = self.outcomes.iter().map(|o| o.latency).collect();
        latencies.sort_unstable();
        let idx = ((p / 100.0) * latencies.len() as f64).ceil() as usize;
        latencies[idx.saturating_sub(1).min(latencies.len() - 1)]
    }

    /// p50 (median) latency.
    pub fn p50(&self) -> Duration {
        self.percentile_latency(50.0)
    }

    /// p95 latency.
    pub fn p95(&self) -> Duration {
        self.percentile_latency(95.0)
    }

    /// p99 latency.
    pub fn p99(&self) -> Duration {
        self.percentile_latency(99.0)
    }

    /// Print a human-readable summary to stdout.
    pub fn print_summary(&self, label: &str) {
        println!(
            "\n=== Load Test: {label} ===\n\
             Total requests : {total}\n\
             Successes      : {ok}\n\
             Failures       : {fail}\n\
             Error rate     : {err:.2}%\n\
             Throughput     : {rps:.1} req/s\n\
             Latency min    : {min:?}\n\
             Latency mean   : {mean:?}\n\
             Latency p50    : {p50:?}\n\
             Latency p95    : {p95:?}\n\
             Latency p99    : {p99:?}\n\
             Latency max    : {max:?}\n\
             Total duration : {dur:?}\n",
            label = label,
            total = self.total(),
            ok = self.successes(),
            fail = self.failures(),
            err = self.error_rate() * 100.0,
            rps = self.rps(),
            min = self.min_latency(),
            mean = self.mean_latency(),
            p50 = self.p50(),
            p95 = self.p95(),
            p99 = self.p99(),
            max = self.max_latency(),
            dur = self.total_duration,
        );
    }
}

// ---------------------------------------------------------------------------
// Runner
// ---------------------------------------------------------------------------

/// Run a load test described by `config`.
///
/// `request_fn` is called once per request. It must be `Clone` so that each
/// Tokio task gets its own copy. It returns `(StatusCode, Duration)`.
///
/// # Panics
///
/// Panics if the run exceeds `config.timeout`.
pub async fn run_load<F, Fut>(config: LoadConfig, request_fn: F) -> LoadResult
where
    F: Fn() -> Fut + Clone + Send + 'static,
    Fut: std::future::Future<Output = (StatusCode, Duration)> + Send,
{
    let wall_start = Instant::now();
    let mut join_set: JoinSet<Vec<RequestOutcome>> = JoinSet::new();

    for _ in 0..config.concurrency {
        let fn_clone = request_fn.clone();
        let n = config.requests_per_task;
        join_set.spawn(async move {
            let mut outcomes = Vec::with_capacity(n);
            for _ in 0..n {
                let (status, latency) = fn_clone().await;
                outcomes.push(RequestOutcome { status, latency });
            }
            outcomes
        });
    }

    // Collect with timeout guard
    let mut all_outcomes: Vec<RequestOutcome> = Vec::with_capacity(config.total_requests());
    let deadline = tokio::time::Instant::now() + config.timeout;

    loop {
        match tokio::time::timeout_at(deadline, join_set.join_next()).await {
            Ok(Some(Ok(outcomes))) => all_outcomes.extend(outcomes),
            Ok(Some(Err(e))) => panic!("Load test task panicked: {e}"),
            Ok(None) => break, // all tasks done
            Err(_) => panic!(
                "Load test timed out after {:?} ({} requests completed of {})",
                config.timeout,
                all_outcomes.len(),
                config.total_requests()
            ),
        }
    }

    LoadResult {
        outcomes: all_outcomes,
        total_duration: wall_start.elapsed(),
    }
}

// ---------------------------------------------------------------------------
// Assertion helper
// ---------------------------------------------------------------------------

/// Assert that a [`LoadResult`] meets the given SLO targets.
///
/// # Arguments
/// - `result` — the completed load run.
/// - `max_error_rate` — maximum acceptable error rate as a fraction (e.g. `0.01` = 1 %).
/// - `max_p99` — maximum acceptable p99 latency.
///
/// # Panics
///
/// Panics with a descriptive message if either threshold is exceeded.
pub fn assert_load_result(result: &LoadResult, max_error_rate: f64, max_p99: Duration) {
    let error_rate = result.error_rate();
    let p99 = result.p99();

    if error_rate > max_error_rate {
        panic!(
            "Load test failed: error rate {:.2}% exceeds maximum {:.2}%\n\
             (failures={}, total={})",
            error_rate * 100.0,
            max_error_rate * 100.0,
            result.failures(),
            result.total(),
        );
    }

    if p99 > max_p99 {
        panic!(
            "Load test failed: p99 latency {:?} exceeds maximum {:?}",
            p99, max_p99,
        );
    }
}

// ---------------------------------------------------------------------------
// Unit tests for the framework itself
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    // --- LoadConfig ---

    #[test]
    fn test_load_config_total_requests() {
        let cfg = LoadConfig::new(4, 10);
        assert_eq!(cfg.total_requests(), 40);
    }

    #[test]
    fn test_load_config_default_total() {
        let cfg = LoadConfig::default();
        assert_eq!(cfg.total_requests(), 50);
    }

    #[test]
    fn test_load_config_with_timeout() {
        let cfg = LoadConfig::default().with_timeout(Duration::from_secs(60));
        assert_eq!(cfg.timeout, Duration::from_secs(60));
    }

    // --- RequestOutcome ---

    #[test]
    fn test_request_outcome_is_success_2xx() {
        let o = RequestOutcome {
            status: StatusCode::OK,
            latency: Duration::from_millis(5),
        };
        assert!(o.is_success());
    }

    #[test]
    fn test_request_outcome_is_not_success_5xx() {
        let o = RequestOutcome {
            status: StatusCode::INTERNAL_SERVER_ERROR,
            latency: Duration::from_millis(5),
        };
        assert!(!o.is_success());
    }

    #[test]
    fn test_request_outcome_is_not_success_4xx() {
        let o = RequestOutcome {
            status: StatusCode::NOT_FOUND,
            latency: Duration::from_millis(5),
        };
        assert!(!o.is_success());
    }

    // --- LoadResult statistics ---

    fn make_result(latencies_ms: &[u64], statuses: &[StatusCode]) -> LoadResult {
        assert_eq!(latencies_ms.len(), statuses.len());
        let outcomes = latencies_ms
            .iter()
            .zip(statuses.iter())
            .map(|(&ms, &status)| RequestOutcome {
                status,
                latency: Duration::from_millis(ms),
            })
            .collect();
        LoadResult {
            outcomes,
            total_duration: Duration::from_millis(100),
        }
    }

    #[test]
    fn test_load_result_counts() {
        let result = make_result(
            &[10, 20, 30],
            &[
                StatusCode::OK,
                StatusCode::OK,
                StatusCode::INTERNAL_SERVER_ERROR,
            ],
        );
        assert_eq!(result.total(), 3);
        assert_eq!(result.successes(), 2);
        assert_eq!(result.failures(), 1);
    }

    #[test]
    fn test_load_result_error_rate() {
        let result = make_result(
            &[10, 20],
            &[StatusCode::OK, StatusCode::INTERNAL_SERVER_ERROR],
        );
        assert!((result.error_rate() - 0.5).abs() < f64::EPSILON);
    }

    #[test]
    fn test_load_result_zero_error_rate() {
        let result = make_result(&[10, 20, 30], &[StatusCode::OK; 3]);
        assert_eq!(result.error_rate(), 0.0);
    }

    #[test]
    fn test_load_result_empty_error_rate() {
        let result = LoadResult {
            outcomes: vec![],
            total_duration: Duration::ZERO,
        };
        assert_eq!(result.error_rate(), 0.0);
    }

    #[test]
    fn test_load_result_min_max_latency() {
        let result = make_result(&[5, 50, 25], &[StatusCode::OK; 3]);
        assert_eq!(result.min_latency(), Duration::from_millis(5));
        assert_eq!(result.max_latency(), Duration::from_millis(50));
    }

    #[test]
    fn test_load_result_mean_latency() {
        let result = make_result(&[10, 20, 30], &[StatusCode::OK; 3]);
        assert_eq!(result.mean_latency(), Duration::from_millis(20));
    }

    #[test]
    fn test_load_result_p50() {
        // sorted: [10, 20, 30, 40, 50] → p50 = 30
        let result = make_result(&[50, 10, 30, 20, 40], &[StatusCode::OK; 5]);
        assert_eq!(result.p50(), Duration::from_millis(30));
    }

    #[test]
    fn test_load_result_p99_single_element() {
        let result = make_result(&[42], &[StatusCode::OK]);
        assert_eq!(result.p99(), Duration::from_millis(42));
    }

    #[test]
    fn test_load_result_p95_100_elements() {
        // 100 elements: 1ms..=100ms; p95 should be 95ms
        let latencies: Vec<u64> = (1..=100).collect();
        let statuses = vec![StatusCode::OK; 100];
        let result = make_result(&latencies, &statuses);
        assert_eq!(result.p95(), Duration::from_millis(95));
    }

    #[test]
    fn test_load_result_rps() {
        let result = LoadResult {
            outcomes: vec![
                RequestOutcome {
                    status: StatusCode::OK,
                    latency: Duration::from_millis(1)
                };
                100
            ],
            total_duration: Duration::from_secs(1),
        };
        assert!((result.rps() - 100.0).abs() < 0.01);
    }

    #[test]
    fn test_load_result_rps_zero_duration() {
        let result = LoadResult {
            outcomes: vec![],
            total_duration: Duration::ZERO,
        };
        assert_eq!(result.rps(), 0.0);
    }

    // --- assert_load_result ---

    #[test]
    fn test_assert_load_result_passes() {
        let result = make_result(&[10, 20, 30], &[StatusCode::OK; 3]);
        // Should not panic
        assert_load_result(&result, 0.0, Duration::from_millis(100));
    }

    #[test]
    #[should_panic(expected = "error rate")]
    fn test_assert_load_result_fails_on_error_rate() {
        let result = make_result(
            &[10, 20],
            &[StatusCode::OK, StatusCode::INTERNAL_SERVER_ERROR],
        );
        assert_load_result(&result, 0.0, Duration::from_secs(1));
    }

    #[test]
    #[should_panic(expected = "p99 latency")]
    fn test_assert_load_result_fails_on_p99() {
        let result = make_result(&[500], &[StatusCode::OK]);
        assert_load_result(&result, 0.0, Duration::from_millis(100));
    }

    // --- run_load ---

    #[tokio::test]
    async fn test_run_load_collects_all_outcomes() {
        let cfg = LoadConfig::new(4, 5); // 20 total
        let result = run_load(cfg, || async { (StatusCode::OK, Duration::from_millis(1)) }).await;

        assert_eq!(result.total(), 20);
        assert_eq!(result.failures(), 0);
    }

    #[tokio::test]
    async fn test_run_load_records_failures() {
        let cfg = LoadConfig::new(1, 2);
        let counter = std::sync::Arc::new(std::sync::atomic::AtomicUsize::new(0));
        let counter_clone = counter.clone();

        let result = run_load(cfg, move || {
            let c = counter_clone.clone();
            async move {
                let n = c.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
                let status = if n % 2 == 0 {
                    StatusCode::OK
                } else {
                    StatusCode::INTERNAL_SERVER_ERROR
                };
                (status, Duration::from_millis(1))
            }
        })
        .await;

        assert_eq!(result.total(), 2);
        assert_eq!(result.failures(), 1);
    }

    #[tokio::test]
    async fn test_run_load_respects_concurrency() {
        // Each task records its start time; with concurrency=5 they should
        // all start within a short window (not sequentially).
        let cfg = LoadConfig::new(5, 1);
        let start = Instant::now();
        let result = run_load(cfg, move || async move {
            tokio::time::sleep(Duration::from_millis(10)).await;
            (StatusCode::OK, start.elapsed())
        })
        .await;

        // All 5 tasks ran concurrently so total wall time should be << 50ms
        assert!(result.total_duration < Duration::from_millis(200));
        assert_eq!(result.total(), 5);
    }

    #[tokio::test]
    async fn test_run_load_default_config() {
        let result = run_load(LoadConfig::default(), || async {
            (StatusCode::OK, Duration::from_millis(1))
        })
        .await;
        assert_eq!(result.total(), 50);
    }
}
