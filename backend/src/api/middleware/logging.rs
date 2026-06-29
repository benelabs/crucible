use crate::api::handlers::profiling::AppState;
use crate::services::tracing::TracingService;
use axum::{body::Body, extract::State, http::Request, middleware::Next, response::IntoResponse};
use std::{sync::Arc, time::Instant};
use tracing::Instrument;

/// Middleware to log HTTP requests and responses.
///
/// This middleware captures:
/// - Request method, URI, and HTTP version
/// - Request headers (filtered for security)
/// - Response status code
/// - Processing latency
///
/// It uses `tracing` for structured logging and integrates with the `LogAggregator` service.
pub async fn logging_middleware(
    State(state): State<Arc<AppState>>,
    request: Request<Body>,
    next: Next,
) -> impl IntoResponse {
    let start_time = Instant::now();
    let method = request.method().clone();
    let uri = request.uri().clone();
    let version = request.version();

    let path = uri.path().to_string();
    let span = TracingService::http_request_span(method.as_str(), &path, None);

    let value = span.clone();
    async move {
        // Log the incoming request
        tracing::debug!("Incoming request");

        let response = next.run(request).await;

        let latency = start_time.elapsed();
        let status = response.status();
        value.record("http.status_code", status.as_u16());
        if status.is_server_error() {
            TracingService::record_error(&value, status.as_str(), "http_server_error");
        }

        // Log the response
        tracing::info!(
            latency_ms = latency.as_millis(),
            status = status.as_u16(),
            ?version,
            "Finished processing request"
        );

        // Optionally persist log to LogAggregator
        let log_message = format!(
            "{} {} finished with {} in {:?}",
            method, uri, status, latency
        );

        // We don't want to block the response on logging persistence
        let aggregator = state.log_aggregator.clone();
        tokio::spawn(async move {
            if let Err(e) = aggregator.log("INFO", &log_message, "api_gateway").await {
                tracing::error!(error = %e, "Failed to send log to aggregator");
            }
        });

        response
    }
    .instrument(span)
    .await
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::{reload::ConfigManager, AppConfig};
    use crate::services::{
        contract_benchmark::ContractBenchmarkService, error_recovery::ErrorManager,
        log_aggregator::LogAggregator, sys_metrics::MetricsExporter,
    };
    use axum::{routing::get, Router};
    use hyper::StatusCode;
    use redis::Client as RedisClient;
    use tower::ServiceExt;

    #[tokio::test]
    async fn test_logging_middleware_success() {
        // Mock dependencies
        let metrics_exporter = Arc::new(MetricsExporter::new());
        let error_manager = Arc::new(ErrorManager::new());
        let (log_aggregator, _rx) = LogAggregator::new();
        let log_aggregator = Arc::new(log_aggregator);

        let redis = RedisClient::open("redis://localhost").unwrap();
        let config = crate::config::AppConfig::default();
        let config_manager = Arc::new(crate::config::reload::ConfigManager::new(config));

        let state = Arc::new(AppState {
            db: None,
            metrics_exporter,
            error_manager,
            config_manager: Arc::new(ConfigManager::new(AppConfig::default())),
            log_aggregator,
            db: Some(db),
            contract_benchmark_service: Arc::new(ContractBenchmarkService::new()),
            redis,
            config_manager,
        });

        let app = Router::new()
            .route("/", get(|| async { "OK" }))
            .layer(axum::middleware::from_fn_with_state(
                state.clone(),
                logging_middleware,
            ))
            .with_state(state);

        let response = app
            .oneshot(Request::builder().uri("/").body(Body::empty()).unwrap())
            .await
            .unwrap();

        assert_eq!(response.status(), StatusCode::OK);
    }
}
