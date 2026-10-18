use axum::{response::IntoResponse, routing::get, Router};
use prometheus::{Encoder, TextEncoder, Registry, Counter};

pub struct MetricsService {
    pub registry: Registry,
    pub active_users: Counter,
    pub volume: Counter,
    pub pool_exhaustion: Counter,
    pub worker_job_dropped: Counter,
}

static POOL_EXHAUSTION_COUNTER: std::sync::OnceLock<Counter> = std::sync::OnceLock::new();

pub fn pool_exhaustion_counter() -> &'static Counter {
    POOL_EXHAUSTION_COUNTER.get_or_init(|| {
        Counter::new(
            "db_pool_exhaustion_total",
            "Total PostgreSQL connection pool timeouts / exhaustions",
        )
        .unwrap()
    })
}

pub fn inc_pool_exhaustion_metric() {
    pool_exhaustion_counter().inc();
}

static WORKER_JOB_DROPPED_COUNTER: std::sync::OnceLock<Counter> = std::sync::OnceLock::new();

/// `worker.job.dropped` — counts background jobs routed to the dead letter
/// queue after exhausting their retries (e.g. after a lease/heartbeat
/// timeout following a Redis connection loss mid-flight).
pub fn worker_job_dropped_counter() -> &'static Counter {
    WORKER_JOB_DROPPED_COUNTER.get_or_init(|| {
        Counter::new(
            "worker_job_dropped_total",
            "Total background jobs routed to the dead letter queue after exhausting retries",
        )
        .unwrap()
    })
}

pub fn inc_worker_job_dropped_metric() {
    worker_job_dropped_counter().inc();
}

impl MetricsService {
    pub fn new() -> Self {
        let registry = Registry::new();
        let active_users = Counter::new("business_daily_active_users", "Daily active users").unwrap();
        let volume = Counter::new("business_volume_total", "Total business volume").unwrap();
        let pool_exhaustion = pool_exhaustion_counter().clone();
        let worker_job_dropped = worker_job_dropped_counter().clone();
        registry.register(Box::new(active_users.clone())).unwrap();
        registry.register(Box::new(volume.clone())).unwrap();
        let _ = registry.register(Box::new(pool_exhaustion.clone()));
        let _ = registry.register(Box::new(worker_job_dropped.clone()));
        Self {
            registry,
            active_users,
            volume,
            pool_exhaustion,
            worker_job_dropped,
        }
    }
}

pub async fn metrics_handler() -> impl IntoResponse {
    let encoder = TextEncoder::new();
    let registry = Registry::new();
    let _ = registry.register(Box::new(pool_exhaustion_counter().clone()));
    let _ = registry.register(Box::new(worker_job_dropped_counter().clone()));

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

#[cfg(test)]
mod worker_job_dropped_tests {
    use super::*;

    #[test]
    fn inc_worker_job_dropped_metric_increments_the_counter() {
        let before = worker_job_dropped_counter().get();
        inc_worker_job_dropped_metric();
        let after = worker_job_dropped_counter().get();
        assert_eq!(after, before + 1.0);
    }
}
