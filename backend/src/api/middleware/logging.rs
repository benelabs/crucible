use chrono::Utc;
use regex::Regex;
use serde::{Deserialize, Serialize};
use std::sync::OnceLock;

static SECRET_KEY_REGEX: OnceLock<Regex> = OnceLock::new();
static SENSITIVE_PARAM_REGEX: OnceLock<Regex> = OnceLock::new();
static SENSITIVE_QUERY_REGEX: OnceLock<Regex> = OnceLock::new();

/// Structured log entry conforming to Elastic Common Schema (ECS) with PII sanitization.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct StructuredLog {
    pub timestamp: String,
    pub level: String,
    pub trace_id: String,
    pub message: String,
    pub context: serde_json::Value,
}

/// Redacts sensitive query parameter values (e.g., token, secret, api_key, key, password, auth)
/// from query strings to prevent credential leaks in logs.
pub fn sanitize_query_string(query: &str) -> String {
    let re = SENSITIVE_QUERY_REGEX.get_or_init(|| {
        Regex::new(r"(?i)(token|secret|api_key|apikey|key|password|auth|access_token|refresh_token)=([^&]*)").unwrap()
    });
    re.replace_all(query, "$1=[REDACTED]").to_string()
}

/// Scrubs 56-character Stellar secret keys (`S...`) and sensitive query parameters (`token=`, `secret=`, `api_key=`)
/// from log messages to prevent secret key and credential leakage.
pub fn sanitize_log_message(msg: &str) -> String {
    let secret_re = SECRET_KEY_REGEX.get_or_init(|| Regex::new(r"S[A-Z2-7]{55}").unwrap());
    let scrubbed = secret_re.replace_all(msg, "[REDACTED_SECRET_KEY]");
    let param_re = SENSITIVE_PARAM_REGEX.get_or_init(|| {
        Regex::new(r"(?i)(token|secret|api_key|apikey|key|password|auth|access_token|refresh_token)=([^&\s]*)").unwrap()
    });
    param_re.replace_all(&scrubbed, "$1=[REDACTED]").to_string()
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

/// Sanitizes a request URI for safe logging by extracting its path and query,
/// redacting sensitive query parameter values (`token=`, `secret=`, `api_key=`),
/// and stripping control characters / newlines.
pub fn sanitize_uri_for_log(uri: &axum::http::Uri) -> String {
    let path = sanitize_for_log(uri.path());
    match uri.query() {
        Some(query) if !query.is_empty() => {
            let sanitized_query = sanitize_query_string(query);
            let safe_query = sanitize_for_log(&sanitized_query);
            format!("{path}?{safe_query}")
        }
        _ => path,
    }
}

/// Middleware to log HTTP requests and responses.
///
/// This middleware captures:
/// - Request method, URI (with sensitive query parameters redacted), and HTTP version
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

    let sanitized_url = sanitize_uri_for_log(&uri);
    let span = TracingService::http_request_span(method.as_str(), &sanitized_url, None);
    TracingService::extract_and_attach_trace_context(request.headers(), &span);

    let value = span.clone();
    let url_for_log = sanitized_url.clone();
    async move {
        // Log the incoming request
        tracing::debug!(url = %url_for_log, "Incoming request");

        let response = next.run(request).await;

        let latency = start_time.elapsed();
        let status = response.status();
        http_metrics().observe(&metric_route, method.as_str(), status.as_u16(), latency);
        value.record("http.status_code", status.as_u16());
        if status.is_server_error() {
            TracingService::record_error(&value, status.as_str(), "http_server_error");
            crate::services::sys_metrics::record_error_rate("http_gateway", status.as_str());
        }

        // Log the response
        tracing::info!(
            url = %url_for_log,
            latency_ms = latency.as_millis(),
            status = status.as_u16(),
            ?version,
            "Finished processing request"
        );

        // Format structured log message with secret key and query parameter sanitization
        let raw_log = format!(
            "{} {} finished with {} in {:?}",
            sanitize_for_log(method.as_str()),
            url_for_log,
            status,
            latency
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
        assert_eq!(result, "GET /pathnull");
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

    #[test]
    fn test_sanitize_query_string_redacts_sensitive_tokens() {
        let query = "token=secret_token_123&user=alice&api_key=key_456&secret=topsecret&page=1";
        let sanitized = sanitize_query_string(query);
        assert!(!sanitized.contains("secret_token_123"));
        assert!(!sanitized.contains("key_456"));
        assert!(!sanitized.contains("topsecret"));
        assert!(sanitized.contains("token=[REDACTED]"));
        assert!(sanitized.contains("api_key=[REDACTED]"));
        assert!(sanitized.contains("secret=[REDACTED]"));
        assert!(sanitized.contains("user=alice"));
        assert!(sanitized.contains("page=1"));
    }

    #[test]
    fn test_sanitize_uri_for_log_with_and_without_query() {
        let uri_with_secrets: axum::http::Uri = "/api/v1/auth?token=mytoken&api_key=secretkey".parse().unwrap();
        let sanitized = sanitize_uri_for_log(&uri_with_secrets);
        assert_eq!(sanitized, "/api/v1/auth?token=[REDACTED]&api_key=[REDACTED]");

        let uri_clean: axum::http::Uri = "/api/v1/health".parse().unwrap();
        assert_eq!(sanitize_uri_for_log(&uri_clean), "/api/v1/health");
    }

    #[tokio::test]
    async fn test_logging_middleware_redacts_query_parameters() {
        let metrics_exporter = Arc::new(MetricsExporter::new());
        let error_manager = Arc::new(ErrorManager::new());
        let (log_aggregator, mut rx) = LogAggregator::new();
        let log_aggregator = Arc::new(log_aggregator);

        let redis = RedisClient::open("redis://localhost").unwrap();
        let config = crate::config::AppConfig::default();
        let config_manager = Arc::new(crate::config::reload::ConfigManager::new(config));

        let state = Arc::new(AppState {
            db: None,
            metrics_exporter,
            error_manager,
            config_manager,
            log_aggregator,
            contract_benchmark_service: Arc::new(ContractBenchmarkService::new()),
            redis,
        });

        let app = Router::new()
            .route("/api/test", get(|| async { "OK" }))
            .layer(axum::middleware::from_fn_with_state(
                state.clone(),
                logging_middleware,
            ))
            .with_state(state);

        let req = Request::builder()
            .uri("/api/test?token=super_secret_token_999&api_key=key_xyz&filter=active")
            .body(Body::empty())
            .unwrap();

        let response = app.oneshot(req).await.unwrap();
        assert_eq!(response.status(), StatusCode::OK);

        // Receive logged message from channel
        if let Ok(entry) = tokio::time::timeout(std::time::Duration::from_millis(200), rx.recv()).await {
            let msg = entry.expect("should receive log entry").message;
            assert!(!msg.contains("super_secret_token_999"));
            assert!(!msg.contains("key_xyz"));
            assert!(msg.contains("token=[REDACTED]"));
            assert!(msg.contains("api_key=[REDACTED]"));
            assert!(msg.contains("filter=active"));
        }
    }
}
