use axum::{response::IntoResponse, routing::get, Router};
use prometheus::{Encoder, TextEncoder, Registry, Counter};

pub struct MetricsService {
    pub registry: Registry,
    pub active_users: Counter,
    pub volume: Counter,
}

impl MetricsService {
    pub fn new() -> Self {
        let registry = Registry::new();
        let active_users = Counter::new("business_daily_active_users", "Daily active users").unwrap();
        let volume = Counter::new("business_volume_total", "Total business volume").unwrap();
        registry.register(Box::new(active_users.clone())).unwrap();
        registry.register(Box::new(volume.clone())).unwrap();
        Self {
            registry,
            active_users,
            volume,
        }
    }
}

pub async fn metrics_handler() -> impl IntoResponse {
    let encoder = TextEncoder::new();
    let registry = Registry::new();

    // Read the actual memory stats
    let (rss, _heap) = crate::services::sys_metrics::get_linux_memory_stats();

    let opts = prometheus::Opts::new("process_resident_memory_bytes", "Resident set size in bytes");
    let rss_gauge = prometheus::IntGauge::with_opts(opts).unwrap();
    rss_gauge.set(rss as i64);

    registry.register(Box::new(rss_gauge)).unwrap();

    let mut buffer = vec![];
    encoder.encode(&registry.gather(), &mut buffer).unwrap();
    String::from_utf8(buffer).unwrap()
}

pub fn router() -> Router {
    Router::new().route("/metrics", get(metrics_handler))
}
