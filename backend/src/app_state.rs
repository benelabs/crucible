//! Application route state construction.
//!
//! Each route group receives an explicit, named state bundle so ownership in
//! `main` is intentional and free of moved-value errors.

use std::sync::Arc;

use redis::Client as RedisClient;
use sqlx::PgPool;

use crate::api::handlers::{
    coverage::CoverageState,
    dashboard::DashboardState,
    profiling::AppState,
    ws::WsState,
};
use crate::config::reload::ConfigManager;
use crate::services::{
    audit::AuditService,
    contract_benchmark::ContractBenchmarkService,
    error_recovery::ErrorManager,
    log_aggregator::LogAggregator,
    log_alerts::AlertManager,
    sys_metrics::MetricsExporter,
    test_coverage::TestCoverageService,
};

/// Shared service handles used when building route state.
pub struct SharedServices {
    pub metrics_exporter: Arc<MetricsExporter>,
    pub error_manager: Arc<ErrorManager>,
    pub alert_manager: Arc<AlertManager>,
    pub log_aggregator: Arc<LogAggregator>,
    pub contract_benchmark_service: Arc<ContractBenchmarkService>,
    pub config_manager: Arc<ConfigManager>,
}

/// Named Axum state for each route group.
pub struct ApplicationStates {
    pub profiling: Arc<AppState>,
    pub dashboard: Arc<DashboardState>,
    pub coverage: Arc<CoverageState>,
    pub websocket: Arc<WsState>,
    pub audit: Arc<AuditService>,
}

/// Build all route states from shared infrastructure handles.
pub fn build_application_states(
    db_pool: PgPool,
    redis_client: RedisClient,
    services: &SharedServices,
) -> ApplicationStates {
    let profiling = Arc::new(AppState {
        db: Some(db_pool.clone()),
        metrics_exporter: services.metrics_exporter.clone(),
        error_manager: services.error_manager.clone(),
        config_manager: services.config_manager.clone(),
        log_aggregator: services.log_aggregator.clone(),
        contract_benchmark_service: services.contract_benchmark_service.clone(),
        redis: redis_client.clone(),
    });

    let dashboard = Arc::new(DashboardState {
        metrics_exporter: services.metrics_exporter.clone(),
        error_manager: services.error_manager.clone(),
        alert_manager: services.alert_manager.clone(),
        db: db_pool.clone(),
        redis: redis_client.clone(),
    });

    let coverage = Arc::new(CoverageState {
        service: TestCoverageService::new(db_pool.clone(), redis_client.clone()),
    });

    let websocket = Arc::new(WsState {
        metrics_exporter: services.metrics_exporter.clone(),
        error_manager: services.error_manager.clone(),
    });

    let audit = Arc::new(AuditService::new(db_pool, Arc::new(redis_client)));

    ApplicationStates {
        profiling,
        dashboard,
        coverage,
        websocket,
        audit,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::{AppConfig, Environment};

    fn test_redis_client() -> RedisClient {
        RedisClient::open("redis://127.0.0.1:6379").expect("redis client")
    }

    fn test_db_pool() -> PgPool {
        PgPool::connect_lazy("postgres://postgres:postgres@localhost/crucible_test")
            .expect("lazy db pool")
    }

    fn test_services() -> SharedServices {
        let app_config = AppConfig::load(Environment::Development).expect("config");
        SharedServices {
            metrics_exporter: Arc::new(MetricsExporter::new()),
            error_manager: Arc::new(ErrorManager::new()),
            alert_manager: Arc::new(AlertManager::new()),
            log_aggregator: Arc::new(LogAggregator::new().0),
            contract_benchmark_service: Arc::new(ContractBenchmarkService::new()),
            config_manager: Arc::new(ConfigManager::new(app_config)),
        }
    }

    #[test]
    fn application_states_construction_succeeds() {
        let services = test_services();
        let states = build_application_states(
            test_db_pool(),
            test_redis_client(),
            &services,
        );

        assert!(Arc::strong_count(&states.profiling) >= 1);
        assert!(Arc::strong_count(&states.dashboard) >= 1);
        assert!(Arc::strong_count(&states.coverage) >= 1);
        assert!(Arc::strong_count(&states.websocket) >= 1);
        assert!(Arc::strong_count(&states.audit) >= 1);
        assert!(states.profiling.db.is_some());
    }

    #[test]
    fn profiling_and_websocket_share_metrics_exporter() {
        let services = test_services();
        let states = build_application_states(
            test_db_pool(),
            test_redis_client(),
            &services,
        );

        assert!(Arc::ptr_eq(
            &states.profiling.metrics_exporter,
            &states.websocket.metrics_exporter,
        ));
    }
}
