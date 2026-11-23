// Match the backend library's documented lint baseline.
#![allow(
    clippy::type_complexity,
    clippy::too_many_arguments,
    clippy::should_implement_trait
)]

use std::net::SocketAddr;
use std::sync::Arc;

use apalis::prelude::*;
use apalis_redis::RedisStorage;
use utoipa::OpenApi;

use backend::{
    api::handlers::{dashboard, profiling},
    app_state::{build_application_states, SharedServices},
    config::{reload::ConfigManager, AppConfig, Environment},
    jobs::{monitor_transaction, TransactionMonitorJob},
    router::build_router,
    services::audit,
    services::{
        contract_benchmark::ContractBenchmarkService,
        error_recovery::ErrorManager,
        log_aggregator::LogAggregator,
        log_alerts::AlertManager,
        sandbox::ContractSandboxService,
        sys_metrics::MetricsExporter,
        tracing::{TracingConfig, TracingService},
    },
};
use redis::Client as RedisClient;
use tokio::signal;

use tracing::info_span;

/// OpenAPI document served at `/swagger-ui`.
#[derive(OpenApi)]
#[openapi(
    paths(
        profiling::get_metrics,
        profiling::get_health,
        dashboard::get_dashboard_metrics,
        dashboard::get_contract_stats,
        dashboard::get_batch_contract_stats,
        audit::list_audit_reports,
        audit::get_audit_report,
    ),
    components(schemas(
        profiling::MetricsReport,
        profiling::HealthResponse,
        dashboard::DashboardMetrics,
        dashboard::ContractStats,
        dashboard::BatchContractStatsRequest,
        dashboard::BatchContractStatsResponse,
        audit::AuditEventRecord,
        audit::AuditEventRequest,
    )),
    tags(
        (name = "profiling", description = "Performance and health monitoring endpoints"),
        (name = "dashboard", description = "Dashboard metrics and analytics endpoints")
    )
)]
#[allow(dead_code)]
struct ApiDoc;

#[tokio::main]
async fn main() -> Result<(), anyhow::Error> {
    dotenvy::dotenv().ok();

    let env = Environment::from_env();
    let config = AppConfig::load(env).expect("Failed to load configuration");

    let tracing_config = TracingConfig::new(
        "crucible-backend".to_string(),
        env!("CARGO_PKG_VERSION").to_string(),
    )
    .with_environment(env.as_str().to_string())
    .with_otlp_endpoint(
        config
            .observability
            .tracing_endpoint
            .clone()
            .unwrap_or_else(|| "http://localhost:4318/v1/traces".to_string()),
    );

    let _tracing_guard = TracingService::init(tracing_config)?;
    let _enter = info_span!("app.startup").entered();

    let db_pool = config.database.create_pool().await?;
    tracing::info!("Database connection established");

    let redis_client = RedisClient::open(config.redis.url.clone())?;

    let metrics_exporter = Arc::new(MetricsExporter::new());
    let error_manager = Arc::new(ErrorManager::new());
    let alert_manager = Arc::new(AlertManager::new());
    let (log_aggregator, log_receiver) = LogAggregator::new();
    let log_aggregator = Arc::new(log_aggregator);
    let sandbox_service = Arc::new(ContractSandboxService::default());
    let contract_benchmark_service = Arc::new(ContractBenchmarkService::with_db(db_pool.clone()));
    let config_manager = Arc::new(ConfigManager::new(config.clone()));

    let shutdown_coordinator = Arc::new(backend::workers::ShutdownCoordinator::new());

    let collector_handle = tokio::spawn(MetricsExporter::run_collector(metrics_exporter.clone()));
    let aggregator_handle = tokio::spawn(LogAggregator::run_worker(log_receiver));
    shutdown_coordinator.register(collector_handle).await;
    shutdown_coordinator.register(aggregator_handle).await;

    let conn = backend::db::redis::connect_with_retry(&redis_client, &config.redis).await?;
    // A job's lease/heartbeat window: if the worker holding a job stops
    // renewing its lock (e.g. the Redis connection drops mid-flight during a
    // Sentinel failover), the job is requeued after this duration instead of
    // being silently lost. `max_retries` bounds how many times a requeued job
    // is retried before apalis considers it permanently failed.
    let storage_config = apalis_redis::Config::default()
        .set_reenqueue_orphaned_after(std::time::Duration::from_secs(300))
        .set_max_retries(5);
    let storage: RedisStorage<TransactionMonitorJob> =
        RedisStorage::new_with_config(conn, storage_config);
    tracing::info!("Redis connection established");

    let worker = WorkerBuilder::new("monitor-worker")
        .backend(storage)
        .build_fn(monitor_transaction);

    let worker_handle = tokio::spawn(async move {
        let _ = worker.run().await;
    });
    shutdown_coordinator.register(worker_handle).await;

    let health_cache = backend::db::redis::connect_with_retry(&redis_client, &config.redis).await?;
    let health_queue = backend::db::redis::connect_with_retry(&redis_client, &config.redis).await?;

    let _health_state = backend::api::handlers::health::HealthState {
        db: db_pool.clone(),
        cache: health_cache,
        queue: health_queue,
        soroban_rpc_url: None,
    };

    let shared_services = SharedServices {
        metrics_exporter,
        error_manager,
        alert_manager,
        log_aggregator,
        contract_benchmark_service,
        config_manager: config_manager.clone(),
    };

    let states = build_application_states(db_pool.clone(), redis_client.clone(), &shared_services);

    let app = build_router(
        states,
        config_manager,
        db_pool.clone(),
        redis_client.clone(),
        sandbox_service,
        &config,
    );

    let addr: SocketAddr = format!("{}:{}", config.server.host, config.server.port).parse()?;
    tracing::info!("Crucible backend listening on {addr}");
    let listener = tokio::net::TcpListener::bind(&addr).await?;

    let res = axum::serve(listener, app)
        .with_graceful_shutdown(shutdown_signal())
        .await;
    let worker_grace_period =
        std::time::Duration::from_secs(config.server.worker_grace_period_secs);
    tracing::info!(
        "Server stopping; waiting for background workers to drain (grace period: {}s)...",
        config.server.worker_grace_period_secs
    );
    shutdown_coordinator
        .shutdown_with_grace(worker_grace_period)
        .await;
    tracing::info!("Closing database connection pool");
    db_pool.close().await;

    res?;
    Ok(())
}

async fn shutdown_signal() {
    let ctrl_c = async {
        signal::ctrl_c()
            .await
            .expect("failed to install Ctrl+C handler");
    };

    #[cfg(unix)]
    let terminate = async {
        signal::unix::signal(signal::unix::SignalKind::terminate())
            .expect("failed to install signal handler")
            .recv()
            .await;
    };

    #[cfg(not(unix))]
    let terminate = std::future::pending::<()>();

    tokio::select! {
        _ = ctrl_c => {},
        _ = terminate => {},
    }
}
