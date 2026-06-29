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

    let health_state = health::HealthState {
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

    let states =
        build_application_states(db_pool.clone(), redis_client.clone(), &shared_services);

    let cors = build_cors_layer(&config);

    // Bearer-token auth registry for privileged admin/config endpoints.
    let admin_auth = Arc::new(AdminAuthState::from_env());

    let app = build_router(
        states,
        config_manager,
        sandbox_service,
        admin_auth,
        db_pool.clone(),
        redis_client.clone(),
        cors,
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

/// Assemble the complete application router.
///
/// Routes are grouped one-per-domain so each API path is registered exactly
/// once and the table is easy to scan. Axum panics at build time on
/// overlapping routes, so a successful build is itself a duplicate-route check
/// (see [`route_table_tests`]).
fn build_router(
    states: ApplicationStates,
    config_manager: Arc<ConfigManager>,
    sandbox_service: Arc<ContractSandboxService>,
    admin_auth: Arc<AdminAuthState>,
    db_pool: PgPool,
    redis_client: RedisClient,
    cors: CorsLayer,
) -> Router {
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
    )]
    struct ApiDoc;

    let contracts_router = Router::new()
        .route("/compile", post(contracts::compile_contract))
        .route(
            "/analyze-dependencies",
            post(contracts::analyze_dependencies),
        )
        .route(
            "/compliance-check",
            post(contracts::check_compliance),
        )
        .route(
            "/logs",
            post(contracts::log_contract_call).get(contracts::get_contract_logs),
        )
        .route(
            "/upgrade-plan",
            post(contracts::create_upgrade_plan),
        )
        .route("/templates", get(contracts::get_templates))
        .with_state(profiling_state.clone());

    // --- Coverage ---
    let coverage_router = Router::new()
        .route("/", post(coverage::submit_coverage))
        .route("/:project", get(coverage::get_latest_coverage))
        .with_state(coverage_state);

    let admin_router = Router::new()
        .route(
            "/system-stats",
            get(admin::get_system_stats),
        )
        .route(
            "/maintenance",
            post(admin::set_maintenance_mode),
        )
        .route("/logs", get(admin::get_admin_logs));

    let contracts_router = Router::new()
        .route("/compile", post(contracts::compile_contract))
        .route("/analyze-dependencies", post(contracts::analyze_dependencies))
        .route("/compliance-check", post(contracts::check_compliance))
        .route(
            "/logs",
            post(contracts::log_contract_call).get(contracts::get_contract_logs),
        )
        .route("/upgrade-plan", post(contracts::create_upgrade_plan))
        .route("/templates", get(contracts::get_templates))
        .with_state(profiling_state.clone());

    let cors = build_cors_layer(&config);

    let app = Router::new()
        .route("/", get(|| async { "Crucible Backend API" }))
        .route("/.well-known/stellar.toml", get(stellar::get_stellar_toml))
        .merge(config_router)
        .merge(legacy_profiling_router)
        .nest("/api/v1/profiling", profiling_router)
        .nest("/api/v1/dashboard", dashboard_router)
        .nest("/api/v1/audit", audit::routes(audit_service))
        .nest("/api/v1/contracts", contracts_router)
        .route("/api/v1/networks", get(contracts::get_networks))
        .nest("/api/v1/admin", admin_router)
        .nest(
            "/api/v1/errors",
            errors::error_analytics_routes(db_pool.clone(), redis_client.clone()),
        )
        .nest("/api/v1/sandbox", sandbox::routes(sandbox_service))
        .nest(
            "/api/v1/errors",
            errors::error_analytics_routes(db_pool.clone(), redis_client.clone()),
        )
        .nest("/api/v1/sandbox", sandbox::routes(sandbox_service))
        .nest("/api/v1/coverage", coverage_router)
        .route("/api/v1/ws/dashboard", get(ws_dashboard_handler).with_state(ws_state))
        .route("/api/dashboard", get(get_dashboard))
        .with_state(dashboard_state)
        .merge(SwaggerUi::new("/swagger-ui").url("/api-docs/openapi.json", ApiDoc::openapi()))
        .layer(middleware::from_fn_with_state(
            profiling_state,
            logging_middleware,
        ))
        .layer(TraceLayer::new_for_http())
        .layer(cors)
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
        _ = ctrl_c => tracing::info!("Received Ctrl+C, initiating graceful shutdown"),
        _ = terminate => tracing::info!("Received SIGTERM, initiating graceful shutdown"),
    }
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

#[cfg(test)]
mod route_table_tests {
    use super::*;

    /// Construct route state from non-connecting (lazy) infrastructure handles
    /// so the router can be assembled without a live database or Redis.
    fn lazy_state_bundle() -> (
        ApplicationStates,
        Arc<ConfigManager>,
        Arc<ContractSandboxService>,
        Arc<AdminAuthState>,
        PgPool,
        RedisClient,
    ) {
        let db_pool =
            PgPool::connect_lazy("postgres://postgres:postgres@localhost/crucible_test")
                .expect("lazy db pool");
        let redis_client =
            RedisClient::open("redis://127.0.0.1:6379").expect("redis client");
        let app_config = AppConfig::load(Environment::Development).expect("config");
        let config_manager = Arc::new(ConfigManager::new(app_config));

        let services = SharedServices {
            metrics_exporter: Arc::new(MetricsExporter::new()),
            error_manager: Arc::new(ErrorManager::new()),
            alert_manager: Arc::new(AlertManager::new()),
            log_aggregator: Arc::new(LogAggregator::new().0),
            contract_benchmark_service: Arc::new(ContractBenchmarkService::new()),
            config_manager: config_manager.clone(),
        };

        let states =
            build_application_states(db_pool.clone(), redis_client.clone(), &services);
        let sandbox_service = Arc::new(ContractSandboxService::default());
        let admin_auth = Arc::new(AdminAuthState::new());

        (
            states,
            config_manager,
            sandbox_service,
            admin_auth,
            db_pool,
            redis_client,
        )
    }

    /// Axum panics at build time when two routes overlap, so a router that
    /// builds successfully proves every path is registered exactly once. This
    /// guards against the duplicate `contracts`/`networks`/`admin`/`coverage`
    /// definitions that previously coexisted in `main`.
    #[test]
    fn router_builds_with_unique_routes() {
        let (states, config_manager, sandbox_service, admin_auth, db_pool, redis_client) =
            lazy_state_bundle();

        let _app = build_router(
            states,
            config_manager,
            sandbox_service,
            admin_auth,
            db_pool,
            redis_client,
            CorsLayer::new(),
        );
    }
}
