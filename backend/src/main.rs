use std::net::SocketAddr;
use std::sync::Arc;

use apalis::prelude::*;
use apalis_redis::RedisStorage;
use axum::{
    middleware,
    routing::{get, post},
    Router,
};
use backend::api::handlers::dashboard::get_dashboard;
use backend::api::handlers::ws::ws_dashboard_handler;
use utoipa::OpenApi;

use backend::{
    api::handlers::{contracts, dashboard, errors, profiling, sandbox, stellar},
    api::middleware::logging::logging_middleware,
    app_state::{build_application_states, ApplicationStates, SharedServices},
    config::{
        reload::{handle_get_config, handle_reload, ConfigManager},
        AppConfig, Environment,
    },
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
use redis::aio::ConnectionManager;
use redis::Client as RedisClient;
use sqlx::PgPool;
use tokio::signal;
use tokio_util::sync::CancellationToken;

use tower_http::{
    cors::{AllowOrigin, Any, CorsLayer},
    trace::TraceLayer,
};
use tracing::info_span;

/// OpenAPI document served at `/swagger-ui`.
#[derive(OpenApi)]
#[openapi(
    paths(
        profiling::get_metrics,
        profiling::get_health,
        dashboard::get_dashboard_metrics,
        dashboard::get_contract_stats,
        audit::list_audit_reports,
        audit::get_audit_report,
    ),
    components(schemas(
        profiling::MetricsReport,
        profiling::HealthResponse,
        dashboard::DashboardMetrics,
        dashboard::ContractStats,
        audit::AuditEventRecord,
        audit::AuditEventRequest,
    )),
    tags(
        (name = "profiling", description = "Performance and health monitoring endpoints"),
        (name = "dashboard", description = "Dashboard metrics and analytics endpoints")
    )
)]
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

    let db_pool = config
        .database
        .to_sqlx_pool_options()
        .connect(&config.database.url)
        .await?;
    tracing::info!("Database connection established");

    let redis_client = RedisClient::open(config.redis.url.clone())?;

    let metrics_exporter = Arc::new(MetricsExporter::new());
    let error_manager = Arc::new(ErrorManager::new());
    let alert_manager = Arc::new(AlertManager::new());
    let (log_aggregator, log_receiver) = LogAggregator::new();
    let log_aggregator = Arc::new(log_aggregator);
    let sandbox_service = Arc::new(ContractSandboxService::default());
    let contract_benchmark_service = Arc::new(ContractBenchmarkService::new());
    let config_manager = Arc::new(ConfigManager::new(config.clone()));

    tokio::spawn(MetricsExporter::run_collector(metrics_exporter.clone()));
    tokio::spawn(LogAggregator::run_worker(log_receiver));

    let conn = ConnectionManager::new(redis_client.clone()).await?;
    let storage: RedisStorage<TransactionMonitorJob> = RedisStorage::new(conn);
    tracing::info!("Redis connection established");

    let worker = WorkerBuilder::new("monitor-worker")
        .backend(storage)
        .build_fn(monitor_transaction);

    let health_cache = ConnectionManager::new(redis_client.clone()).await?;
    let health_queue = ConnectionManager::new(redis_client.clone()).await?;

    let health_state = backend::api::handlers::health::HealthState {
        db: db_pool.clone(),
        cache: health_cache,
        queue: health_queue,
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

    let result = tokio::select! {
        res = axum::serve(listener, app).with_graceful_shutdown(shutdown_signal()) => {
            db_pool.close().await;
            res
        },
        _ = worker.run() => Ok(()),
    };

    result?;
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
