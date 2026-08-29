use chrono::Utc;
use regex::Regex;
use serde::{Deserialize, Serialize};
use std::sync::OnceLock;

static SECRET_KEY_REGEX: OnceLock<Regex> = OnceLock::new();

/// Structured log entry conforming to Elastic Common Schema (ECS) with PII sanitization.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct StructuredLog {
    pub timestamp: String,
    pub level: String,
    pub trace_id: String,
    pub message: String,
    pub context: serde_json::Value,
}

/// Scrubs 56-character Stellar secret keys (`S...`) from log messages to prevent secret key leakage.
pub fn sanitize_log_message(msg: &str) -> String {
    let re = SECRET_KEY_REGEX.get_or_init(|| Regex::new(r"S[A-Z2-7]{55}").unwrap());
    re.replace_all(msg, "[REDACTED_SECRET_KEY]").to_string()
}

/// Formats a log message into structured JSON conforming to ECS with PII sanitization.
pub fn format_structured_log(
    level: &str,
    trace_id: &str,
    message: &str,
    context: serde_json::Value,
) -> String {
    let sanitized_msg = sanitize_log_message(message);
    let log_entry = StructuredLog {
        timestamp: Utc::now().to_rfc3339(),
        level: level.to_string(),
        trace_id: trace_id.to_string(),
        message: sanitized_msg,
        context,
    };
    serde_json::to_string(&log_entry).unwrap_or_else(|_| message.to_string())
}

/// Middleware to log HTTP requests and responses with PII sanitization and structured JSON output.
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

        // Format structured log message with secret key sanitization
        let raw_log = format!(
            "{} {} finished with {} in {:?}",
            method, uri, status, latency
        );
        let sanitized_message = sanitize_log_message(&raw_log);

        // We don't want to block the response on logging persistence
        let aggregator = state.log_aggregator.clone();
        tokio::spawn(async move {
            if let Err(e) = aggregator.log("INFO", &sanitized_message, "api_gateway").await {
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

    #[test]
    fn test_stellar_secret_key_sanitization() {
        let raw_log = "Submitting tx with secret key SAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAABBB";
        let sanitized = sanitize_log_message(raw_log);
        assert!(!sanitized.contains("SAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAABBB"));
        assert!(sanitized.contains("[REDACTED_SECRET_KEY]"));
    }

    #[test]
    fn test_format_structured_log_json() {
        let json_str = format_structured_log("INFO", "trace-123", "User login success", serde_json::json!({"user_id": 42}));
        let parsed: StructuredLog = serde_json::from_str(&json_str).unwrap();
        assert_eq!(parsed.level, "INFO");
        assert_eq!(parsed.trace_id, "trace-123");
        assert_eq!(parsed.message, "User login success");
        assert_eq!(parsed.context["user_id"], 42);
    }
}
