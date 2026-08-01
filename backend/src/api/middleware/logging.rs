use crate::api::handlers::profiling::AppState;
use crate::services::http_metrics::http_metrics;
use crate::services::tracing::TracingService;
use axum::{
    body::Body,
    extract::{MatchedPath, State},
    http::Request,
    middleware::Next,
    response::IntoResponse,
};
use std::{sync::Arc, time::Instant};
use tracing::Instrument;

/// Sanitize a string for safe log output by stripping newlines and control characters.
///
/// Replaces:
/// - `\n` (newline) → space
/// - `\r` (carriage return) → space
/// - `\t` (tab) → space
/// - All other ASCII control characters (0x00–0x1F, 0x7F) → empty string
///
/// This prevents log injection attacks where user-controlled input
/// contains fake log entries via embedded newlines.
pub fn sanitize_for_log(input: &str) -> String {
    input.chars().filter_map(|c| match c {
        '\n' | '\r' | '\t' => Some(' '),
        c if c.is_ascii_control() => None,
        c => Some(c),
    }).collect()
}

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
    let metric_route = request
        .extensions()
        .get::<MatchedPath>()
        .map(MatchedPath::as_str)
        .unwrap_or("UNMATCHED")
        .to_string();

    let path = sanitize_for_log(uri.path());
    let span = TracingService::http_request_span(method.as_str(), &path, None);

    let value = span.clone();
    async move {
        // Log the incoming request
        tracing::debug!("Incoming request");

        let response = next.run(request).await;

        let latency = start_time.elapsed();
        let status = response.status();
        http_metrics().observe(&metric_route, method.as_str(), status.as_u16(), latency);
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
            sanitize_for_log(method.as_str()),
            path,
            status,
            latency
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

    #[test]
    fn test_sanitize_for_log_preserves_normal_text() {
        let input = "GET /api/v1/users";
        assert_eq!(sanitize_for_log(input), "GET /api/v1/users");
    }

    #[test]
    fn test_sanitize_for_log_replaces_newline_with_space() {
        let input = "POST /api\nX-Injected: true";
        let result = sanitize_for_log(input);
        assert!(!result.contains('\n'));
        assert!(result.contains("true"));
    }

    #[test]
    fn test_sanitize_for_log_replaces_carriage_return() {
        let input = "data\r\nInjected";
        let result = sanitize_for_log(input);
        assert!(!result.contains('\r'));
        assert!(result.contains("Injected"));
    }

    #[test]
    fn test_sanitize_for_log_replaces_tab() {
        let input = "GET\t/path";
        let result = sanitize_for_log(input);
        assert!(!result.contains('\t'));
        assert_eq!(result, "GET /path");
    }

    #[test]
    fn test_sanitize_for_log_strips_control_characters() {
        let input = format!("GET{} /path\x00null", '\x1b');
        let result = sanitize_for_log(&input);
        assert!(!result.contains('\x1b'));
        assert!(!result.contains('\x00'));
        assert_eq!(result, "GET  /pathnull");
    }

    #[test]
    fn test_sanitize_for_log_handles_empty_string() {
        assert_eq!(sanitize_for_log(""), "");
    }

    #[test]
    fn test_sanitize_for_log_preserves_unicode() {
        let input = "GET /café/🚀";
        assert_eq!(sanitize_for_log(input), input);
    }

    #[test]
    fn test_sanitize_for_log_mixed_injection_attempt() {
        let input = "GET /\nHTTP/1.1\r\nX-User: admin\r\n";
        let result = sanitize_for_log(input);
        assert!(!result.contains('\n'));
        assert!(!result.contains('\r'));
        assert!(result.contains("HTTP/1.1"));
        assert!(result.contains("X-User"));
    }
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
            config_manager: config_manager.clone(),
            log_aggregator,
            contract_benchmark_service: Arc::new(ContractBenchmarkService::new()),
            redis,
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
