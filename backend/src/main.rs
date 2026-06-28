use std::{net::SocketAddr, sync::Arc};

use apalis::prelude::*;
use apalis_redis::RedisStorage;
use axum::{
    middleware,
    routing::{get, post},
    Router,
};
use backend::api::handlers::dashboard::get_dashboard;
use backend::api::handlers::ws::ws_dashboard_handler;
use backend::{
    api::handlers::{dashboard, errors, profiling, sandbox, stellar},
    api::middleware::logging::logging_middleware,
    app_state::{build_application_states, ApplicationStates, SharedServices},
    config::{
        reload::{handle_get_config, handle_reload, ConfigManager},
        AppConfig, Environment,
    },
    jobs::{monitor_transaction, TransactionMonitorJob},
    services::{
        audit,
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
use std::net::SocketAddr;
use std::sync::Arc;
use tokio::signal;
use tokio_util::sync::CancellationToken;

use tower_http::{
    cors::{AllowOrigin, Any, CorsLayer},
    trace::TraceLayer,
};
use tracing::info_span;
use utoipa::OpenApi;
use utoipa_swagger_ui::SwaggerUi;

#[tokio::main]
async fn main() -> Result<(), anyhow::Error> {
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

    let shared_services = SharedServices {
        metrics_exporter,
        error_manager,
        alert_manager,
        log_aggregator,
        contract_benchmark_service,
        config_manager: config_manager.clone(),
    };

    let ApplicationStates {
        profiling: profiling_state,
        dashboard: dashboard_state,
        coverage: coverage_state,
        websocket: ws_state,
        audit: audit_service,
    } = build_application_states(db_pool.clone(), redis_client.clone(), &shared_services);

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
        .route("/compliance-check", post(contracts::check_compliance))
        .route(
            "/logs",
            post(contracts::log_contract_call).get(contracts::get_contract_logs),
        )
        .route("/upgrade-plan", post(contracts::create_upgrade_plan))
        .route("/templates", get(contracts::get_templates));

    let coverage_router = Router::new()
        .route("/", post(coverage::submit_coverage))
        .route("/:project", get(coverage::get_latest_coverage))
        .with_state(coverage_state);

    let admin_router = Router::new()
        .route(
            "/system-stats",
            get(backend::api::handlers::admin::get_system_stats),
        )
        .route(
            "/maintenance",
            post(backend::api::handlers::admin::set_maintenance_mode),
        )
        .route("/logs", get(backend::api::handlers::admin::get_admin_logs));

    let cors = build_cors_layer(&config);

    let app = Router::new()
        .route("/", get(|| async { "Crucible Backend API" }))
        .route("/.well-known/stellar.toml", get(stellar::get_stellar_toml))
        .merge(
            Router::new()
                .route("/api/config", get(handle_get_config))
                .route("/api/config/reload", post(handle_reload))
                .with_state(config_manager),
        )
        .nest(
            "/api/v1/profiling",
            Router::new()
                .route("/metrics", get(profiling::get_metrics))
                .route("/health", get(profiling::get_health))
                .route("/prometheus", get(profiling::get_prometheus_metrics))
                .route("/status", get(profiling::get_system_status))
                .route("/profile", post(profiling::trigger_profile_collection))
                .route("/contracts/benchmark", post(profiling::run_contract_benchmark))
                .with_state(profiling_state.clone()),
        )
        .route("/api/status", get(profiling::get_system_status))
        .route("/api/profile", post(profiling::trigger_profile_collection))
        .with_state(profiling_state.clone())
        .nest(
            "/api/v1/dashboard",
            Router::new()
                .route("/", get(get_dashboard))
                .route("/metrics", get(dashboard::get_dashboard_metrics))
                .route("/contracts/:contract_id/stats", get(dashboard::get_contract_stats))
                .with_state(dashboard_state.clone()),
        )
        .nest("/api/v1/audit", audit::routes(audit_service))
        .nest(
            "/api/v1/contracts",
            Router::new()
                .route("/compile", post(backend::api::handlers::contracts::compile_contract))
                .route(
                    "/analyze-dependencies",
                    post(backend::api::handlers::contracts::analyze_dependencies),
                )
                .route(
                    "/compliance-check",
                    post(backend::api::handlers::contracts::check_compliance),
                )
                .route(
                    "/logs",
                    post(backend::api::handlers::contracts::log_contract_call)
                        .get(backend::api::handlers::contracts::get_contract_logs),
                )
                .route(
                    "/upgrade-plan",
                    post(backend::api::handlers::contracts::create_upgrade_plan),
                )
                .route("/templates", get(backend::api::handlers::contracts::get_templates))
                .with_state(profiling_state.clone()),
        )
        .route("/api/v1/networks", get(backend::api::handlers::contracts::get_networks))
        .nest(
            "/api/v1/admin",
            Router::new()
                .route("/system-stats", get(backend::api::handlers::admin::get_system_stats))
                .route("/maintenance", post(backend::api::handlers::admin::set_maintenance_mode))
                .route("/logs", get(backend::api::handlers::admin::get_admin_logs))
                .with_state(profiling_state.clone()),
        )
        .nest(
            "/api/v1/errors",
            errors::error_analytics_routes(db_pool.clone(), redis_client.clone()),
        )
        .nest("/api/v1/contracts", contracts_router)
        .route("/api/v1/networks", get(contracts::get_networks))
        .nest("/api/v1/admin", admin_router)
        .nest("/api/v1/sandbox", sandbox::routes(sandbox_service))
        .nest(
            "/api/v1/coverage",
            Router::new()
                .route("/", post(backend::api::handlers::coverage::submit_coverage))
                .route("/:project", get(backend::api::handlers::coverage::get_latest_coverage))
                .with_state(coverage_state),
        )
        .route("/api/v1/ws/dashboard", get(ws_dashboard_handler).with_state(ws_state))
        .route("/api/dashboard", get(get_dashboard))
        .with_state(dashboard_state)
        .merge(SwaggerUi::new("/swagger-ui").url("/api-docs/openapi.json", ApiDoc::openapi()))
        .layer(middleware::from_fn_with_state(profiling_state, logging_middleware))
        .layer(TraceLayer::new_for_http())
        .layer(cors);

    let addr: SocketAddr = format!("{}:{}", config.server.host, config.server.port).parse()?;
    tracing::info!("Crucible backend listening on {addr}");
    let listener = tokio::net::TcpListener::bind(&addr).await?;

    let cancel_token = CancellationToken::new();
    let worker_cancel_token = cancel_token.clone();

    let worker_handle = tokio::spawn(async move {
        // Apalis worker run loop.
        // We also wait for cancellation so the supervisor task resolves promptly.
        tokio::select! {
            res = worker.run() => {
                // Propagate result to the caller.
                res
            }
            _ = worker_cancel_token.cancelled() => {
                tracing::info!("Cancellation requested; stopping worker");
                Ok(())
            }
        }
    });

    let server_cancel_token = cancel_token.clone();
    let server_handle = tokio::spawn(async move {
        let res = axum::serve(listener, app).with_graceful_shutdown(async move {
            // Wait for cancellation to ensure both worker + server observe the same shutdown.
            let _ = server_cancel_token.cancelled().await;
        });

        db_pool.close().await;
        res
    });

    // Listen for Ctrl+C / SIGTERM and trigger global cancellation.
    let signal_cancel_token = cancel_token.clone();
    let signal_handle = tokio::spawn(async move {
        let ctrl_c = async {
            signal::ctrl_c().await.expect("failed to install Ctrl+C handler");
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
            _ = ctrl_c => {
                tracing::info!("Received Ctrl+C, initiating graceful shutdown");
            }
            _ = terminate => {
                tracing::info!("Received SIGTERM, initiating graceful shutdown");
            }
        }

        signal_cancel_token.cancel();
    });


    // Structured coordination: if worker fails, cancel the server.
    let result: Result<(), anyhow::Error> = tokio::select! {
        server_res = server_handle => {
            match server_res {
                Ok(Ok(())) => Ok(()),
                Ok(Err(e)) => Err(anyhow::Error::new(e)),
                Err(join_err) => Err(anyhow::Error::new(join_err))
            }
        }
        worker_res = worker_handle => {
            // Cancel server, then report worker failure.
            cancel_token.cancel();

            match worker_res {
                Ok(Ok(())) => Ok(()),
                Ok(Err(e)) => {
                    tracing::error!(error = %e, "Unhandled background worker crash; canceling HTTP server");
                    Err(anyhow::Error::new(e))
                }
                Err(join_err) => {
                    tracing::error!(error = %join_err, "Unhandled background worker task panicked or was aborted; canceling HTTP server");
                    Err(anyhow::Error::new(join_err))
                }
            }
        }
        _ = signal_handle => {
            // Signal triggered cancellation; wait for server/worker resolution implicitly.
            Ok(())
        }
    };

    result?;
    Ok(())
}


fn build_cors_layer(config: &AppConfig) -> CorsLayer {
    if config.cors.allowed_origins.contains(&"*".to_string()) {
        CorsLayer::new()
            .allow_origin(Any)
            .allow_methods(Any)
            .allow_headers(Any)
    } else {
        let origins: Vec<axum::http::HeaderValue> = config
            .cors
            .allowed_origins
            .iter()
            .map(|o| {
                o.parse()
                    .unwrap_or_else(|_| panic!("Invalid CORS origin: {}", o))
            })
            .collect();
        CorsLayer::new()
            .allow_origin(AllowOrigin::list(origins))
            .allow_methods(Any)
            .allow_headers(Any)
    }
}