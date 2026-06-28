//! Top-level HTTP router construction.
//!
//! Keeping route composition in a standalone, side-effect-free function
//! allows the full route tree to be constructed and exercised in unit
//! tests without booting a database, Redis connection, or HTTP listener.

use std::sync::Arc;

use axum::{
    middleware,
    routing::{get, post},
    Router,
};
use redis::Client as RedisClient;
use sqlx::PgPool;
use tower_http::{
    cors::{AllowOrigin, Any, CorsLayer},
    trace::TraceLayer,
};
use utoipa::OpenApi;
use utoipa_swagger_ui::SwaggerUi;

use crate::{
    api::handlers::{
        admin, contracts as contract_handlers, coverage, dashboard,
        dashboard::get_dashboard, errors, profiling, sandbox, stellar, ws::ws_dashboard_handler,
    },
    api::middleware::logging::logging_middleware,
    app_state::ApplicationStates,
    config::{
        reload::{handle_get_config, handle_reload, ConfigManager},
        AppConfig,
    },
    services::{audit, sandbox::ContractSandboxService},
};

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
        (name = "dashboard", description = "Dashboard metrics and analytics endpoints"),
    )
)]
struct ApiDoc;

/// Build the complete Axum [`Router`] for the backend API.
///
/// This function performs no I/O of its own (no listener binding, no
/// connection establishment) — it only wires already-constructed state and
/// services into route handlers. That makes it safe to call from unit tests
/// with lazily-connected pools/clients, since nothing here actually talks
/// to the network until a request is dispatched through the router.
pub fn build_router(
    states: ApplicationStates,
    config_manager: Arc<ConfigManager>,
    db_pool: PgPool,
    redis_client: RedisClient,
    sandbox_service: Arc<ContractSandboxService>,
    config: &AppConfig,
) -> Router {
    let ApplicationStates {
        profiling: profiling_state,
        dashboard: dashboard_state,
        coverage: coverage_state,
        websocket: ws_state,
        audit: audit_service,
    } = states;

    let contracts_router = Router::new()
        .route("/compile", post(contract_handlers::compile_contract))
        .route(
            "/analyze-dependencies",
            post(contract_handlers::analyze_dependencies),
        )
        .route(
            "/compliance-check",
            post(contract_handlers::check_compliance),
        )
        .route(
            "/logs",
            post(contract_handlers::log_contract_call).get(contract_handlers::get_contract_logs),
        )
        .route(
            "/upgrade-plan",
            post(contract_handlers::create_upgrade_plan),
        )
        .route("/templates", get(contract_handlers::get_templates))
        .with_state(profiling_state.clone());

    let admin_router = Router::new()
        .route("/system-stats", get(admin::get_system_stats))
        .route("/maintenance", post(admin::set_maintenance_mode))
        .route("/logs", get(admin::get_admin_logs))
        .with_state(profiling_state.clone());

    let cors = build_cors_layer(config);

    Router::new()
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
                .route(
                    "/contracts/benchmark",
                    post(profiling::run_contract_benchmark),
                )
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
                .route(
                    "/contracts/:contract_id/stats",
                    get(dashboard::get_contract_stats),
                )
                .with_state(dashboard_state.clone()),
        )
        .nest("/api/v1/audit", audit::routes(audit_service))
        .nest("/api/v1/contracts", contracts_router)
        .route("/api/v1/networks", get(contract_handlers::get_networks))
        .nest("/api/v1/admin", admin_router)
        .nest(
            "/api/v1/errors",
            errors::error_analytics_routes(db_pool, redis_client),
        )
        .nest("/api/v1/sandbox", sandbox::routes(sandbox_service))
        .nest(
            "/api/v1/coverage",
            Router::new()
                .route("/", post(coverage::submit_coverage))
                .route("/:project", get(coverage::get_latest_coverage))
                .with_state(coverage_state),
        )
        .route(
            "/api/v1/ws/dashboard",
            get(ws_dashboard_handler).with_state(ws_state),
        )
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
mod tests {
    use super::*;
    use crate::{
        app_state::build_application_states,
        config::Environment,
        services::{
            contract_benchmark::ContractBenchmarkService, error_recovery::ErrorManager,
            log_aggregator::LogAggregator, log_alerts::AlertManager,
            sys_metrics::MetricsExporter,
        },
    };
    use axum::{body::Body, http::Request};
    use tower::ServiceExt;

    fn test_config() -> AppConfig {
        AppConfig::load(Environment::Development).expect("config")
    }

    // Lazy connections: no real DB/Redis required to construct the router.
    fn test_db_pool() -> PgPool {
        PgPool::connect_lazy("postgres://postgres:postgres@localhost/crucible_test")
            .expect("lazy db pool")
    }

    fn test_redis_client() -> RedisClient {
        RedisClient::open("redis://127.0.0.1:6379").expect("redis client")
    }

    fn test_shared_services(config: &AppConfig) -> crate::app_state::SharedServices {
        crate::app_state::SharedServices {
            metrics_exporter: Arc::new(MetricsExporter::new()),
            error_manager: Arc::new(ErrorManager::new()),
            alert_manager: Arc::new(AlertManager::new()),
            log_aggregator: Arc::new(LogAggregator::new().0),
            contract_benchmark_service: Arc::new(ContractBenchmarkService::new()),
            config_manager: Arc::new(ConfigManager::new(config.clone())),
        }
    }

    fn test_router() -> Router {
        let config = test_config();
        let services = test_shared_services(&config);
        let states = build_application_states(test_db_pool(), test_redis_client(), &services);

        build_router(
            states,
            services.config_manager.clone(),
            test_db_pool(),
            test_redis_client(),
            Arc::new(ContractSandboxService::default()),
            &config,
        )
    }

    #[test]
    fn build_router_constructs_with_test_state() {
        // Constructing the router exercises every `.route`, `.nest`, and
        // `.layer` call. If any handler/state pairing is wrong, or any path
        // syntax is invalid, this panics at build time rather than at
        // runtime in production.
        let _router: Router = test_router();
    }

    #[tokio::test]
    async fn build_router_serves_root_route() {
        let response = test_router()
            .oneshot(Request::builder().uri("/").body(Body::empty()).unwrap())
            .await
            .unwrap();

        assert_eq!(response.status(), axum::http::StatusCode::OK);
    }
}
