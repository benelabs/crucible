use std::sync::OnceLock;
use std::time::Duration;

use prometheus::{
    Encoder, HistogramOpts, HistogramVec, IntCounterVec, Opts, Registry, TextEncoder,
};

/// Check if a path segment is a dynamic parameter (UUID, numeric ID, hex hash, or address).
pub fn is_dynamic_segment(segment: &str) -> bool {
    if segment.is_empty() {
        return false;
    }

    // If it's already a route parameter template (e.g. :id, :contract_id, {id}), keep it
    if segment.starts_with(':') || (segment.starts_with('{') && segment.ends_with('}')) {
        return false;
    }

    // 1. UUID check (with or without hyphens)
    if uuid::Uuid::parse_str(segment).is_ok() {
        return true;
    }

    // 2. Pure numeric ID check (e.g. 12345)
    if segment.chars().all(|c| c.is_ascii_digit()) {
        return true;
    }

    // 3. Hex hash / address check (e.g. 0x... or 16+ hex chars)
    let hex_part = segment.strip_prefix("0x").unwrap_or(segment);
    if hex_part.len() >= 16 && hex_part.chars().all(|c| c.is_ascii_hexdigit()) {
        return true;
    }

    // 4. Stellar public address check (56 chars alphanumeric starting with G or C)
    if segment.len() == 56
        && (segment.starts_with('G') || segment.starts_with('C'))
        && segment.chars().all(|c| c.is_ascii_alphanumeric())
    {
        return true;
    }

    false
}

/// Normalize an HTTP request path into a bounded route pattern template.
///
/// Replaces dynamic path segments (such as UUIDs, numeric IDs, or hashes)
/// with `:id` to prevent Prometheus time-series label cardinality explosion.
pub fn normalize_route(path: &str) -> String {
    if path.is_empty() || path == "/" {
        return "/".to_string();
    }

    // Strip query string if present
    let path = path.split('?').next().unwrap_or(path);

    let ends_with_slash = path.ends_with('/') && path.len() > 1;

    let normalized: Vec<&str> = path
        .split('/')
        .map(|segment| {
            if is_dynamic_segment(segment) {
                ":id"
            } else {
                segment
            }
        })
        .collect();

    let mut result = normalized.join("/");
    if ends_with_slash && !result.ends_with('/') {
        result.push('/');
    }
    result
}

/// Shared Prometheus metrics for HTTP request instrumentation.
pub struct HttpMetrics {
    registry: Registry,
    duration_seconds: HistogramVec,
    requests_total: IntCounterVec,
}

static HTTP_METRICS: OnceLock<HttpMetrics> = OnceLock::new();

impl Default for HttpMetrics {
    fn default() -> Self {
        Self::new()
    }
}

impl HttpMetrics {
    pub fn new() -> Self {
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

    /// Observe one HTTP request latency and increment request count with route normalization.
    pub fn observe(&self, route: &str, method: &str, status: u16, duration: Duration) {
        let normalized = normalize_route(route);
        let status = status.to_string();
        self.duration_seconds
            .with_label_values(&[&normalized, method, &status])
            .observe(duration.as_secs_f64());
        self.requests_total
            .with_label_values(&[&normalized, method, &status])
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_normalize_uuid_path() {
        let raw = "/api/v1/contracts/550e8400-e29b-41d4-a716-446655440000";
        assert_eq!(normalize_route(raw), "/api/v1/contracts/:id");
    }

    #[test]
    fn test_normalize_uuid_with_subpath() {
        let raw = "/api/v1/contracts/550e8400-e29b-41d4-a716-446655440000/stats";
        assert_eq!(normalize_route(raw), "/api/v1/contracts/:id/stats");
    }

    #[test]
    fn test_normalize_numeric_id() {
        let raw = "/api/v1/users/42/profile";
        assert_eq!(normalize_route(raw), "/api/v1/users/:id/profile");
    }

    #[test]
    fn test_normalize_hex_hash() {
        let raw = "/api/v1/tx/0x4f35e89b2c34d89a7710bfe34a9b5107e324efb1c93a8d11b22e1189acbd7190";
        assert_eq!(normalize_route(raw), "/api/v1/tx/:id");
    }

    #[test]
    fn test_normalize_existing_template_param() {
        let raw = "/api/v1/contracts/:contract_id/stats";
        assert_eq!(normalize_route(raw), "/api/v1/contracts/:contract_id/stats");
    }

    #[test]
    fn test_normalize_static_routes() {
        assert_eq!(normalize_route("/metrics"), "/metrics");
        assert_eq!(
            normalize_route("/api/v1/profiling/health"),
            "/api/v1/profiling/health"
        );
        assert_eq!(normalize_route("/"), "/");
    }

    #[test]
    fn test_normalize_trailing_slash_and_query() {
        assert_eq!(
            normalize_route("/api/v1/contracts/550e8400-e29b-41d4-a716-446655440000/?foo=bar"),
            "/api/v1/contracts/:id/"
        );
    }

    #[test]
    fn test_observe_bounds_metric_cardinality() {
        let metrics = HttpMetrics::new();
        for _ in 0..100 {
            let path = format!("/api/v1/contracts/{}", uuid::Uuid::new_v4());
            metrics.observe(&path, "GET", 200, Duration::from_millis(10));
        }

        let rendered = metrics.render().unwrap();
        assert!(rendered.contains(r#"route="/api/v1/contracts/:id""#));
        assert!(rendered.contains(
            r#"http_requests_total{method="GET",route="/api/v1/contracts/:id",status="200"} 100"#
        ));
    }
}
