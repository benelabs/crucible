use std::sync::OnceLock;
use std::time::Duration;

use prometheus::{Encoder, HistogramOpts, HistogramVec, IntCounterVec, Opts, Registry, TextEncoder};

/// Shared Prometheus metrics for HTTP request instrumentation.
pub struct HttpMetrics {
    registry: Registry,
    duration_seconds: HistogramVec,
    requests_total: IntCounterVec,
}

static HTTP_METRICS: OnceLock<HttpMetrics> = OnceLock::new();

impl HttpMetrics {
    fn new() -> Self {
        let registry = Registry::new();
        let duration_seconds = HistogramVec::new(
            HistogramOpts::new(
                "http_request_duration_seconds",
                "HTTP request duration in seconds",
            )
            .buckets(vec![
                0.005, 0.01, 0.025, 0.05, 0.1, 0.25, 0.5, 1.0, 2.5, 5.0, 10.0,
            ]),
            &["route", "method", "status"],
        )
        .expect("histogram definition must be valid");
        let requests_total = IntCounterVec::new(
            Opts::new("http_requests_total", "Total HTTP requests"),
            &["route", "method", "status"],
        )
        .expect("counter definition must be valid");

        registry
            .register(Box::new(duration_seconds.clone()))
            .expect("duration histogram registration must succeed");
        registry
            .register(Box::new(requests_total.clone()))
            .expect("request counter registration must succeed");

        Self {
            registry,
            duration_seconds,
            requests_total,
        }
    }

    /// Observe one HTTP request latency and increment request count.
    pub fn observe(&self, route: &str, method: &str, status: u16, duration: Duration) {
        let status = status.to_string();
        self.duration_seconds
            .with_label_values(&[route, method, &status])
            .observe(duration.as_secs_f64());
        self.requests_total
            .with_label_values(&[route, method, &status])
            .inc();
    }

    /// Render the registry as Prometheus text exposition format.
    pub fn render(&self) -> Result<String, String> {
        let encoder = TextEncoder::new();
        let mut buffer = Vec::new();
        encoder
            .encode(&self.registry.gather(), &mut buffer)
            .map_err(|error| format!("failed to encode Prometheus metrics: {error}"))?;
        String::from_utf8(buffer)
            .map_err(|error| format!("failed to convert Prometheus payload to UTF-8: {error}"))
    }
}

/// Returns the process-wide HTTP metrics collector.
pub fn http_metrics() -> &'static HttpMetrics {
    HTTP_METRICS.get_or_init(HttpMetrics::new)
}
