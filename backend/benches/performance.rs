use axum::extract::State;
use backend::api::handlers::profiling::{get_metrics, AppState};
use backend::config::reload::ConfigManager;
use backend::config::AppConfig;
use backend::services::{
    contract_benchmark::ContractBenchmarkService,
    error_recovery::ErrorManager,
    log_aggregator::LogAggregator,
    sys_metrics::MetricsExporter,
};
use criterion::{criterion_group, criterion_main, Criterion};
use redis::Client as RedisClient;
use std::hint::black_box;
use std::sync::Arc;
use tokio::runtime::Runtime;

fn bench_metrics_handler(c: &mut Criterion) {
    let rt = Runtime::new().unwrap();
    let (log_aggregator, _rx) = LogAggregator::new();
    let state = Arc::new(AppState {
        db: None,
        metrics_exporter: Arc::new(MetricsExporter::new()),
        error_manager: Arc::new(ErrorManager::new()),
        config_manager: Arc::new(ConfigManager::new(AppConfig::default())),
        log_aggregator: Arc::new(log_aggregator),
        contract_benchmark_service: Arc::new(ContractBenchmarkService::new()),
        redis: RedisClient::open("redis://127.0.0.1:1/").unwrap(),
    });

    c.bench_function("get_metrics_handler", |b| {
        let state = state.clone();
        b.to_async(&rt).iter(|| {
            let state = state.clone();
            async move {
                let _ = get_metrics(State(state)).await;
                black_box(())
            }
        });
    });
}

criterion_group!(benches, bench_metrics_handler);
criterion_main!(benches);
