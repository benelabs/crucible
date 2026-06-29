//! Prometheus metrics collection service.
//!
//! Provides a lightweight, zero-dependency metrics registry that tracks HTTP
//! request counts, latencies, database pool stats, and cache hit/miss rates.
//! Metrics are exposed in the Prometheus text exposition format via
//! `GET /metrics`.
//!
//! # Example
//!
//! ```rust,no_run
//! use backend::services::metrics::MetricsRegistry;
//!
//! let registry = MetricsRegistry::new();
//! registry.http_requests_total.inc("GET", "/health", 200);
//! let output = registry.render();
//! assert!(output.contains("http_requests_total"));
//! ```

use std::collections::HashMap;
use std::sync::{Arc, Mutex};
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::Instant;

use axum::{extract::State, http::StatusCode, response::IntoResponse};
use tracing::{debug, instrument};

// ---------------------------------------------------------------------------
// Counter
// ---------------------------------------------------------------------------

/// A thread-safe monotonically increasing counter with optional labels.
#[derive(Debug, Default)]
pub struct Counter {
    inner: Mutex<HashMap<String, u64>>,
}

impl Counter {
    pub fn new() -> Self {
        Self::default()
    }

    /// Increment the counter for the given label set.
    pub fn inc(&self, labels: &str) {
        let mut map = self.inner.lock().unwrap();
        *map.entry(labels.to_string()).or_insert(0) += 1;
    }

    /// Increment by `n`.
    pub fn inc_by(&self, labels: &str, n: u64) {
        let mut map = self.inner.lock().unwrap();
        *map.entry(labels.to_string()).or_insert(0) += n;
    }

    /// Snapshot all label→value pairs.
    pub fn snapshot(&self) -> HashMap<String, u64> {
        self.inner.lock().unwrap().clone()
    }
}

// ---------------------------------------------------------------------------
// Gauge
// ---------------------------------------------------------------------------

/// A thread-safe gauge (can go up or down).
#[derive(Debug, Default)]
pub struct Gauge {
    inner: Mutex<HashMap<String, f64>>,
}

impl Gauge {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn set(&self, labels: &str, value: f64) {
        let mut map = self.inner.lock().unwrap();
        map.insert(labels.to_string(), value);
    }

    pub fn snapshot(&self) -> HashMap<String, f64> {
        self.inner.lock().unwrap().clone()
    }
}

// ---------------------------------------------------------------------------
// Histogram (fixed buckets)
// ---------------------------------------------------------------------------

/// Observation buckets for latency histograms (milliseconds).
pub const LATENCY_BUCKETS_MS: &[f64] = &[5.0, 10.0, 25.0, 50.0, 100.0, 250.0, 500.0, 1000.0, 2500.0, f64::INFINITY];

/// A simple histogram with fixed upper-bound buckets.
#[derive(Debug)]
pub struct Histogram {
    buckets: Vec<f64>,
    inner: Mutex<HashMap<String, HistogramData>>,
}

#[derive(Debug, Clone, Default)]
struct HistogramData {
    counts: Vec<u64>,
    sum: f64,
    total: u64,
}

impl Histogram {
    pub fn new(buckets: Vec<f64>) -> Self {
        Self { buckets, inner: Mutex::new(HashMap::new()) }
    }

    /// Record an observation (value in milliseconds).
    pub fn observe(&self, labels: &str, value_ms: f64) {
        let mut map = self.inner.lock().unwrap();
        let data = map.entry(labels.to_string()).or_insert_with(|| HistogramData {
            counts: vec![0; self.buckets.len()],
            sum: 0.0,
            total: 0,
        });
        for (i, &bound) in self.buckets.iter().enumerate() {
            if value_ms <= bound {
                data.counts[i] += 1;
            }
        }
        data.sum += value_ms;
        data.total += 1;
    }

    pub fn snapshot(&self) -> HashMap<String, (Vec<(f64, u64)>, f64, u64)> {
        let map = self.inner.lock().unwrap();
        map.iter()
            .map(|(k, v)| {
                let buckets = self.buckets.iter().copied().zip(v.counts.iter().copied()).collect();
                (k.clone(), (buckets, v.sum, v.total))
            })
            .collect()
    }
}

// ---------------------------------------------------------------------------
// Registry
// ---------------------------------------------------------------------------

/// Central metrics registry for the Crucible backend.
#[derive(Debug)]
pub struct MetricsRegistry {
    /// Total HTTP requests, labelled by method, path, and status code.
    pub http_requests_total: Counter,
    /// HTTP request duration histogram (milliseconds).
    pub http_request_duration_ms: Histogram,
    /// Total errors, labelled by error kind.
    pub errors_total: Counter,
    /// Database connection pool size (active connections).
    pub db_pool_connections: Gauge,
    /// Cache hit counter.
    pub cache_hits_total: Counter,
    /// Cache miss counter.
    pub cache_misses_total: Counter,
    /// Total file uploads processed.
    pub file_uploads_total: Counter,
    /// Total bytes uploaded.
    pub file_upload_bytes_total: Counter,
    /// Application start timestamp (Unix seconds).
    pub process_start_time_seconds: AtomicU64,
}

impl MetricsRegistry {
    pub fn new() -> Self {
        use std::time::{SystemTime, UNIX_EPOCH};
        let start = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap_or_default()
            .as_secs();
        Self {
            http_requests_total: Counter::new(),
            http_request_duration_ms: Histogram::new(LATENCY_BUCKETS_MS.to_vec()),
            errors_total: Counter::new(),
            db_pool_connections: Gauge::new(),
            cache_hits_total: Counter::new(),
            cache_misses_total: Counter::new(),
            file_uploads_total: Counter::new(),
            file_upload_bytes_total: Counter::new(),
            process_start_time_seconds: AtomicU64::new(start),
        }
    }

    /// Record an HTTP request completion.
    pub fn record_request(&self, method: &str, path: &str, status: u16, duration_ms: f64) {
        let labels = format!(r#"method="{method}",path="{path}",status="{status}""#);
        self.http_requests_total.inc(&labels);
        let hist_labels = format!(r#"method="{method}",path="{path}""#);
        self.http_request_duration_ms.observe(&hist_labels, duration_ms);
    }

    /// Record an error by kind.
    pub fn record_error(&self, kind: &str) {
        self.errors_total.inc(&format!(r#"kind="{kind}""#));
    }

    /// Record a cache hit.
    pub fn record_cache_hit(&self, cache: &str) {
        self.cache_hits_total.inc(&format!(r#"cache="{cache}""#));
    }

    /// Record a cache miss.
    pub fn record_cache_miss(&self, cache: &str) {
        self.cache_misses_total.inc(&format!(r#"cache="{cache}""#));
    }

    /// Record a file upload.
    pub fn record_file_upload(&self, mime: &str, bytes: u64) {
        let label = format!(r#"mime="{mime}""#);
        self.file_uploads_total.inc(&label);
        self.file_upload_bytes_total.inc_by(&label, bytes);
    }

    /// Record the current active database pool connection count.
    pub fn record_db_pool_connections(&self, active_connections: u64) {
        self.db_pool_connections.set(r#"pool="active""#, active_connections as f64);
    }

    /// Render all metrics in Prometheus text exposition format.
    #[instrument(skip(self))]
    pub fn render(&self) -> String {
        let mut out = String::with_capacity(4096);

        // process_start_time_seconds
        let start = self.process_start_time_seconds.load(Ordering::Relaxed);
        out.push_str("# HELP process_start_time_seconds Unix timestamp of process start.\n");
        out.push_str("# TYPE process_start_time_seconds gauge\n");
        out.push_str(&format!("process_start_time_seconds {start}\n\n"));

        // http_requests_total
        out.push_str("# HELP http_requests_total Total HTTP requests by method, path, and status.\n");
        out.push_str("# TYPE http_requests_total counter\n");
        for (labels, count) in self.http_requests_total.snapshot() {
            out.push_str(&format!("http_requests_total{{{labels}}} {count}\n"));
        }
        out.push('\n');

        // http_request_duration_ms
        out.push_str("# HELP http_request_duration_ms HTTP request duration in milliseconds.\n");
        out.push_str("# TYPE http_request_duration_ms histogram\n");
        for (labels, (buckets, sum, count)) in self.http_request_duration_ms.snapshot() {
            for (bound, bucket_count) in &buckets {
                let le = if bound.is_infinite() { "+Inf".to_string() } else { bound.to_string() };
                out.push_str(&format!(
                    "http_request_duration_ms_bucket{{{labels},le=\"{le}\"}} {bucket_count}\n"
                ));
            }
            out.push_str(&format!("http_request_duration_ms_sum{{{labels}}} {sum}\n"));
            out.push_str(&format!("http_request_duration_ms_count{{{labels}}} {count}\n"));
        }
        out.push('\n');

        // errors_total
        out.push_str("# HELP errors_total Total application errors by kind.\n");
        out.push_str("# TYPE errors_total counter\n");
        for (labels, count) in self.errors_total.snapshot() {
            out.push_str(&format!("errors_total{{{labels}}} {count}\n"));
        }
        out.push('\n');

        // db_pool_connections
        out.push_str("# HELP db_pool_connections Active database pool connections.\n");
        out.push_str("# TYPE db_pool_connections gauge\n");
        for (labels, value) in self.db_pool_connections.snapshot() {
            out.push_str(&format!("db_pool_connections{{{labels}}} {value}\n"));
        }
        out.push('\n');

        // cache_hits_total / cache_misses_total
        out.push_str("# HELP cache_hits_total Total cache hits.\n");
        out.push_str("# TYPE cache_hits_total counter\n");
        for (labels, count) in self.cache_hits_total.snapshot() {
            out.push_str(&format!("cache_hits_total{{{labels}}} {count}\n"));
        }
        out.push('\n');

        out.push_str("# HELP cache_misses_total Total cache misses.\n");
        out.push_str("# TYPE cache_misses_total counter\n");
        for (labels, count) in self.cache_misses_total.snapshot() {
            out.push_str(&format!("cache_misses_total{{{labels}}} {count}\n"));
        }
        out.push('\n');

        // file_uploads_total / file_upload_bytes_total
        out.push_str("# HELP file_uploads_total Total file uploads by MIME type.\n");
        out.push_str("# TYPE file_uploads_total counter\n");
        for (labels, count) in self.file_uploads_total.snapshot() {
            out.push_str(&format!("file_uploads_total{{{labels}}} {count}\n"));
        }
        out.push('\n');

        out.push_str("# HELP file_upload_bytes_total Total bytes uploaded by MIME type.\n");
        out.push_str("# TYPE file_upload_bytes_total counter\n");
        for (labels, count) in self.file_upload_bytes_total.snapshot() {
            out.push_str(&format!("file_upload_bytes_total{{{labels}}} {count}\n"));
        }

        debug!("Rendered {} bytes of metrics", out.len());
        out
    }
}

impl Default for MetricsRegistry {
    fn default() -> Self {
        Self::new()
    }
}

/// Shared, cheaply-cloneable handle to the metrics registry.
pub type SharedMetrics = Arc<MetricsRegistry>;

/// Axum handler: `GET /metrics` — returns Prometheus text format.
#[instrument(skip(metrics))]
pub async fn metrics_handler(
    State(metrics): State<SharedMetrics>,
) -> impl IntoResponse {
    let body = metrics.render();
    (
        StatusCode::OK,
        [("content-type", "text/plain; version=0.0.4; charset=utf-8")],
        body,
    )
}

/// Timing guard: records request duration on drop.
pub struct RequestTimer<'a> {
    registry: &'a MetricsRegistry,
    method: String,
    path: String,
    status: u16,
    start: Instant,
}

impl<'a> RequestTimer<'a> {
    pub fn new(registry: &'a MetricsRegistry, method: &str, path: &str, status: u16) -> Self {
        Self {
            registry,
            method: method.to_string(),
            path: path.to_string(),
            status,
            start: Instant::now(),
        }
    }
}

impl Drop for RequestTimer<'_> {
    fn drop(&mut self) {
        let elapsed_ms = self.start.elapsed().as_secs_f64() * 1000.0;
        self.registry.record_request(&self.method, &self.path, self.status, elapsed_ms);
    }
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use axum::{body::Body, http::Request};

    #[test]
    fn counter_increments() {
        let c = Counter::new();
        c.inc("a");
        c.inc("a");
        c.inc("b");
        let snap = c.snapshot();
        assert_eq!(snap["a"], 2);
        assert_eq!(snap["b"], 1);
    }

    #[test]
    fn counter_inc_by() {
        let c = Counter::new();
        c.inc_by("x", 10);
        assert_eq!(c.snapshot()["x"], 10);
    }

    #[test]
    fn gauge_set() {
        let g = Gauge::new();
        g.set("pool", 5.0);
        g.set("pool", 7.0);
        assert_eq!(g.snapshot()["pool"], 7.0);
    }

    #[test]
    fn histogram_observe() {
        let h = Histogram::new(LATENCY_BUCKETS_MS.to_vec());
        h.observe("route", 20.0);
        h.observe("route", 80.0);
        let snap = h.snapshot();
        let (buckets, sum, count) = &snap["route"];
        assert_eq!(*count, 2);
        assert!((sum - 100.0).abs() < f64::EPSILON);
        // 20ms falls in ≤25ms bucket
        let bucket_25 = buckets.iter().find(|(b, _)| *b == 25.0).unwrap();
        assert_eq!(bucket_25.1, 1);
    }

    #[test]
    fn registry_record_request() {
        let r = MetricsRegistry::new();
        r.record_request("GET", "/health", 200, 5.0);
        let snap = r.http_requests_total.snapshot();
        assert_eq!(snap[r#"method="GET",path="/health",status="200""#], 1);
    }

    #[test]
    fn registry_record_error() {
        let r = MetricsRegistry::new();
        r.record_error("database");
        r.record_error("database");
        let snap = r.errors_total.snapshot();
        assert_eq!(snap[r#"kind="database""#], 2);
    }

    #[test]
    fn registry_record_cache() {
        let r = MetricsRegistry::new();
        r.record_cache_hit("redis");
        r.record_cache_miss("redis");
        assert_eq!(r.cache_hits_total.snapshot()[r#"cache="redis""#], 1);
        assert_eq!(r.cache_misses_total.snapshot()[r#"cache="redis""#], 1);
    }

    #[test]
    fn registry_record_file_upload() {
        let r = MetricsRegistry::new();
        r.record_file_upload("application/wasm", 1024);
        let snap = r.file_upload_bytes_total.snapshot();
        assert_eq!(snap[r#"mime="application/wasm""#], 1024);
    }

    #[test]
    fn render_contains_expected_metric_names() {
        let r = MetricsRegistry::new();
        r.record_request("POST", "/upload", 201, 42.0);
        r.record_error("redis");
        let output = r.render();
        assert!(output.contains("http_requests_total"));
        assert!(output.contains("http_request_duration_ms_bucket"));
        assert!(output.contains("errors_total"));
        assert!(output.contains("process_start_time_seconds"));
    }

    #[test]
    fn render_prometheus_text_format() {
        let r = MetricsRegistry::new();
        r.record_request("GET", "/api/v1/status", 200, 10.0);
        let output = r.render();
        // Must contain HELP and TYPE lines
        assert!(output.contains("# HELP http_requests_total"));
        assert!(output.contains("# TYPE http_requests_total counter"));
        // Must contain the labelled counter line
        assert!(output.contains(r#"method="GET""#));
    }

    #[tokio::test]
    async fn metrics_handler_returns_prometheus_text() {
        let metrics = Arc::new(MetricsRegistry::new());
        let app = axum::Router::new()
            .route("/metrics", axum::routing::get(metrics_handler))
            .with_state(metrics.clone());

        let response = app
            .oneshot(Request::builder().uri("/metrics").body(Body::empty()).unwrap())
            .await
            .unwrap();

        assert_eq!(response.status(), StatusCode::OK);
        assert_eq!(response.headers()["content-type"], "text/plain; version=0.0.4; charset=utf-8");
        let body = axum::body::to_bytes(response.into_body()).await.unwrap();
        let body = std::str::from_utf8(&body).unwrap();
        assert!(body.contains("process_start_time_seconds"));
    }

    #[test]
    fn request_timer_records_on_drop() {
        let r = MetricsRegistry::new();
        {
            let _t = RequestTimer::new(&r, "GET", "/test", 200);
            // timer drops here
        }
        let snap = r.http_requests_total.snapshot();
        assert_eq!(snap.values().sum::<u64>(), 1);
    }
}
