//! Integration test framework for the crucible backend.
//!
//! Provides shared helpers for spinning up an in-process Axum router and
//! issuing HTTP requests without binding a real TCP socket.

pub mod api_profile_test;
pub mod api_status_test;
pub mod health_test;
pub mod services_test;
pub mod workers_test;

use axum::{
    routing::{get, post},
    Router,
};
use backend::{
    api::handlers::profiling::{get_system_status, trigger_profile_collection, AppState},
    config::{reload::ConfigManager, AppConfig},
    services::{
        contract_benchmark::ContractBenchmarkService, error_recovery::ErrorManager,
        log_aggregator::LogAggregator, sys_metrics::MetricsExporter,
    },
};
use std::sync::Arc;

/// Build a test [`Router`] backed by fresh service instances.
pub fn test_app() -> Router {
    let (log_aggregator, _receiver) = LogAggregator::new();
    let state = Arc::new(AppState {
        db: None,
        metrics_exporter: Arc::new(MetricsExporter::new()),
        error_manager: Arc::new(ErrorManager::new()),
        config_manager: Arc::new(ConfigManager::new(AppConfig::default())),
        log_aggregator: Arc::new(log_aggregator),
        contract_benchmark_service: Arc::new(ContractBenchmarkService::new()),
        redis: redis::Client::open("redis://127.0.0.1/").unwrap(),
    });

    Router::new()
        .route("/api/status", get(get_system_status))
        .route("/api/profile", post(trigger_profile_collection))
        .with_state(state)
}
