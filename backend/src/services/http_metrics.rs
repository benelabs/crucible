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
    /// URL paths are automatically normalized to route templates to avoid cardinality explosion.
    pub fn observe(&self, route: &str, method: &str, status: u16, duration: Duration) {
        let normalized = normalize_route_path(route);
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

/// Normalizes dynamic path segments (UUIDs, hashes, Stellar contract/account IDs, integer IDs)
/// into parameterized template placeholders (e.g. `:id`) to prevent unbounded
/// Prometheus label cardinality explosion.
pub fn normalize_route_path(path: &str) -> String {
    let raw = path.split('?').next().unwrap_or(path);
    let raw = raw.split('#').next().unwrap_or(raw);

    if raw.is_empty() || raw == "/" {
        return "/".to_string();
    }

    let segments: Vec<&str> = raw.split('/').collect();
    let mut normalized = Vec::with_capacity(segments.len());

    for (i, seg) in segments.iter().enumerate() {
        if seg.is_empty() {
            if i == 0 || i == segments.len() - 1 {
                normalized.push("");
            }
            continue;
        }

        // Check if segment is a UUID (36 chars: 8-4-4-4-12 hex with hyphens)
        if is_uuid(seg) {
            normalized.push(":id");
        }
        // Check if segment is a 64-char or 40-char hex hash
        else if is_hex_hash(seg) {
            normalized.push(":id");
        }
        // Check if segment is a 56-char Stellar contract or account ID (e.g. C... or G...)
        else if is_stellar_id(seg) {
            normalized.push(":id");
        }
        // Check if segment is numeric ID
        else if is_numeric_id(seg) {
            normalized.push(":id");
        } else {
            normalized.push(seg);
        }
    }

    let mut result = normalized.join("/");
    if !result.starts_with('/') {
        result.insert(0, '/');
    }
    result
}

fn is_uuid(s: &str) -> bool {
    if s.len() != 36 {
        return false;
    }
    let b = s.as_bytes();
    if b[8] != b'-' || b[13] != b'-' || b[18] != b'-' || b[23] != b'-' {
        return false;
    }
    s.chars().enumerate().all(|(i, c)| {
        if i == 8 || i == 13 || i == 18 || i == 23 {
            true
        } else {
            c.is_ascii_hexdigit()
        }
    })
}

fn is_hex_hash(s: &str) -> bool {
    (s.len() == 64 || s.len() == 40) && s.chars().all(|c| c.is_ascii_hexdigit())
}

fn is_stellar_id(s: &str) -> bool {
    s.len() == 56
        && (s.starts_with('C') || s.starts_with('G'))
        && s.chars().all(|c| c.is_ascii_alphanumeric())
}

fn is_numeric_id(s: &str) -> bool {
    !s.is_empty() && s.chars().all(|c| c.is_ascii_digit())
}

/// Returns the process-wide HTTP metrics collector.
pub fn http_metrics() -> &'static HttpMetrics {
    HTTP_METRICS.get_or_init(HttpMetrics::new)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_normalize_route_path_uuid() {
        assert_eq!(
            normalize_route_path("/api/v1/contracts/123e4567-e89b-12d3-a456-426614174000"),
            "/api/v1/contracts/:id"
        );
        assert_eq!(
            normalize_route_path("/api/v1/users/550e8400-e29b-41d4-a716-446655440000/details"),
            "/api/v1/users/:id/details"
        );
    }

    #[test]
    fn test_normalize_route_path_stellar_and_hashes() {
        // 56-char Stellar contract ID
        let contract_id = "CAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAA";
        assert_eq!(
            normalize_route_path(&format!("/api/contracts/{contract_id}")),
            "/api/contracts/:id"
        );

        // 64-char sha256 hash
        let hash = "e3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b855";
        assert_eq!(
            normalize_route_path(&format!("/api/transactions/{hash}/status")),
            "/api/transactions/:id/status"
        );

        // Integer ID
        assert_eq!(
            normalize_route_path("/api/v1/items/4294967295"),
            "/api/v1/items/:id"
        );
    }

    #[test]
    fn test_normalize_route_path_static_routes() {
        assert_eq!(normalize_route_path("/api/v1/health"), "/api/v1/health");
        assert_eq!(normalize_route_path("/metrics"), "/metrics");
        assert_eq!(normalize_route_path("/"), "/");
    }

    #[test]
    fn test_observe_normalizes_dynamic_routes() {
        let metrics = HttpMetrics::new();
        metrics.observe(
            "/api/v1/contracts/a0eebc99-9c0b-4ef8-bb6d-6bb9bd380a11",
            "GET",
            200,
            Duration::from_millis(15),
        );
        let rendered = metrics.render().unwrap();
        assert!(rendered.contains("route=\"/api/v1/contracts/:id\""));
        assert!(!rendered.contains("a0eebc99-9c0b-4ef8-bb6d-6bb9bd380a11"));
    }
}
