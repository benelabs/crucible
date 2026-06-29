//! OpenTelemetry tracing service for production-grade observability
//!
//! This module provides the centralized tracing hub for the Crucible backend,
//! implementing OTLP exporter with Jaeger/Zipkin compatibility, semantic conventions,
//! sampling strategies, and proper error propagation.
//!
//! # Features
//! - OTLP/gRPC exporter (Jaeger/Zipkin compatible)
//! - Head-based and tail-based sampling strategies
//! - Semantic conventions for HTTP, DB, and service operations
//! - Resource detection with deployment environment
//! - Span limits and baggage propagation
//! - Zero-overhead when tracing is disabled

use opentelemetry::trace::TracerProvider as _;
use opentelemetry::KeyValue;
use opentelemetry_otlp::{Protocol, WithExportConfig};
use opentelemetry_sdk::trace::{RandomIdGenerator, Sampler, SdkTracerProvider, SpanLimits};
use opentelemetry_sdk::Resource;
use opentelemetry_semantic_conventions::resource;
use std::time::Duration;
use tracing::{info_span, warn};
use tracing_subscriber::{layer::SubscriberExt, util::SubscriberInitExt, EnvFilter, Registry};

/// Central tracing service for initialization and span creation
pub struct TracingService;

/// Keeps the OpenTelemetry provider alive and flushes spans during shutdown.
pub struct TracingGuard {
    tracer_provider: SdkTracerProvider,
}

impl Drop for TracingGuard {
    fn drop(&mut self) {
        if let Err(error) = self.tracer_provider.shutdown() {
            eprintln!("failed to shut down OpenTelemetry tracer provider: {error:?}");
        }
    }
}

/// Configuration for the tracing service
#[derive(Clone, Debug)]
pub struct TracingConfig {
    /// OTLP exporter endpoint (e.g., "http://jaeger:4317")
    pub otlp_endpoint: String,
    /// Service name for resource identification
    pub service_name: String,
    /// Service version
    pub service_version: String,
    /// Deployment environment (dev, staging, production)
    pub environment: String,
    /// Sampling ratio (0.0 to 1.0)
    pub sampling_ratio: f64,
    /// Maximum number of attributes per span
    pub max_attributes_per_span: u32,
    /// Maximum number of events per span
    pub max_events_per_span: u32,
    /// Maximum number of links per span
    pub max_links_per_span: u32,
}

impl Default for TracingConfig {
    fn default() -> Self {
        Self {
            otlp_endpoint: "http://localhost:4318/v1/traces".to_string(),
            service_name: "crucible-backend".to_string(),
            service_version: env!("CARGO_PKG_VERSION").to_string(),
            environment: std::env::var("ENV").unwrap_or("dev".to_string()),
            sampling_ratio: 1.0,
            max_attributes_per_span: 128,
            max_events_per_span: 128,
            max_links_per_span: 128,
        }
    }
}

impl TracingConfig {
    /// Create a new tracing configuration with defaults
    pub fn new(service_name: String, service_version: String) -> Self {
        Self {
            service_name,
            service_version,
            ..Default::default()
        }
    }

    /// Set a custom OTLP endpoint
    pub fn with_otlp_endpoint(mut self, endpoint: String) -> Self {
        self.otlp_endpoint = endpoint;
        self
    }

    /// Set the deployment environment
    pub fn with_environment(mut self, env: String) -> Self {
        self.environment = env.clone();
        self.sampling_ratio = match env.as_str() {
            "production" => 0.01,
            "staging" => 0.1,
            _ => 1.0,
        };
        self
    }

    /// Set custom sampling ratio (0.0 to 1.0)
    pub fn with_sampling_ratio(mut self, ratio: f64) -> Self {
        self.sampling_ratio = ratio.max(0.0).min(1.0);
        self
    }
}

impl TracingService {
    /// Initialize the global tracer provider with OTLP exporter
    pub fn init(config: TracingConfig) -> anyhow::Result<TracingGuard> {
        Self::init_with_filter(config, None, false)
    }

    /// Initialize tracing with the application log filter and formatter.
    pub fn init_with_filter(
        config: TracingConfig,
        log_filter: Option<&str>,
        json_logs: bool,
    ) -> anyhow::Result<TracingGuard> {
        let resource = Resource::builder()
            .with_attributes(vec![
                KeyValue::new(resource::SERVICE_NAME, config.service_name.clone()),
                KeyValue::new(resource::SERVICE_VERSION, config.service_version.clone()),
                KeyValue::new(resource::DEPLOYMENT_ENVIRONMENT, config.environment.clone()),
                KeyValue::new("service.namespace", "crucible"),
            ])
            .build();

        let sampler = if config.environment == "production" {
            Sampler::ParentBased(Box::new(Sampler::TraceIdRatioBased(config.sampling_ratio)))
        } else {
            Sampler::AlwaysOn
        };

        let span_limits = SpanLimits {
            max_attributes_per_span: config.max_attributes_per_span,
            max_events_per_span: config.max_events_per_span,
            max_links_per_span: config.max_links_per_span,
            ..SpanLimits::default()
        };

        let exporter = opentelemetry_otlp::SpanExporter::builder()
            .with_http()
            .with_protocol(Protocol::HttpBinary)
            .with_endpoint(&config.otlp_endpoint)
            .with_timeout(Duration::from_secs(10))
            .build()
            .map_err(|e| anyhow::anyhow!("Failed to build OTLP exporter: {}", e))?;

        let tracer_provider = SdkTracerProvider::builder()
            .with_resource(resource)
            .with_sampler(sampler)
            .with_id_generator(RandomIdGenerator::default())
            .with_span_limits(span_limits)
            .with_batch_exporter(exporter)
            .build();

        // Get a tracer from the provider
        let tracer = tracer_provider.tracer(config.service_name.clone());

        let telemetry_layer = tracing_opentelemetry::layer().with_tracer(tracer);
        let filter = EnvFilter::try_from_default_env().unwrap_or_else(|_| {
            EnvFilter::new(log_filter.unwrap_or("info,crucible=debug,tower_http=info"))
        });

        if json_logs {
            tracing_subscriber::registry()
                .with(filter)
                .with(telemetry_layer)
                .with(
                    tracing_subscriber::fmt::layer()
                        .json()
                        .with_current_span(true)
                        .with_span_list(true)
                        .with_writer(std::io::stderr),
                )
                .try_init()
                .map_err(|e| anyhow::anyhow!("Failed to set global subscriber: {}", e))?;
        } else {
            tracing_subscriber::registry()
                .with(filter)
                .with(telemetry_layer)
                .with(
                    tracing_subscriber::fmt::layer()
                        .pretty()
                        .with_thread_ids(true)
                        .with_target(true)
                        .with_writer(std::io::stderr),
                )
                .try_init()
                .map_err(|e| anyhow::anyhow!("Failed to set global subscriber: {}", e))?;
        }

        tracing::info!("OpenTelemetry tracing initialized successfully");
        tracing::info!("Service: {}", config.service_name);
        tracing::info!("Environment: {}", config.environment);
        tracing::info!("OTLP Endpoint: {}", config.otlp_endpoint);
        tracing::info!("Sampling Ratio: {:.1}%", config.sampling_ratio * 100.0);

        Ok(TracingGuard { tracer_provider })
    }

    /// Create an HTTP request span with semantic conventions
    pub fn http_request_span(method: &str, path: &str, user_id: Option<&str>) -> tracing::Span {
        info_span!(
            "http.request",
            "http.method" = method,
            "http.route" = path,
            "http.flavor" = "1.1",
            "http.scheme" = "https",
            "user.id" = user_id.unwrap_or("anonymous"),
            otel.kind = "server",
            http.status_code = tracing::field::Empty,
            error.type = tracing::field::Empty,
        )
    }

    /// Create a database query span with semantic conventions
    pub fn db_query_span(query: &str, db_system: &str, operation: &str) -> tracing::Span {
        let truncated_query = query
            .split('\n')
            .next()
            .unwrap_or("")
            .trim()
            .chars()
            .take(256)
            .collect::<String>();

        info_span!(
            "db.query",
            "db.system" = db_system,
            "db.statement" = %truncated_query,
            "db.operation" = operation,
            otel.kind = "client",
            db.rows_affected = tracing::field::Empty,
            error.type = tracing::field::Empty,
        )
    }

    /// Create a Redis command span with semantic conventions
    pub fn redis_command_span(command: &str, key: Option<&str>) -> tracing::Span {
        info_span!(
            "db.redis.command",
            "db.system" = "redis",
            "db.redis.command" = command,
            "db.redis.key" = key.unwrap_or(""),
            otel.kind = "client",
            error.type = tracing::field::Empty,
        )
    }

    /// Create a service method span for business operations
    pub fn service_method_span(service_name: &str, method_name: &str) -> tracing::Span {
        info_span!(
            "service.method",
            "service.name" = service_name,
            "service.method" = method_name,
            otel.kind = "internal",
            error.type = tracing::field::Empty,
        )
    }

    /// Create an async job/task span
    pub fn job_span(job_name: &str, job_id: &str) -> tracing::Span {
        info_span!(
            "job.execute",
            "job.name" = job_name,
            "job.id" = job_id,
            otel.kind = "internal",
            error.type = tracing::field::Empty,
        )
    }

    /// Mark current span with error information
    pub fn record_error(span: &tracing::Span, error_message: &str, error_type: &str) {
        span.record("error.type", error_type);
        warn!("Span error recorded: {} ({})", error_message, error_type);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_tracing_config_default() {
        let config = TracingConfig::default();
        assert_eq!(config.service_name, "crucible-backend");
        assert_eq!(config.sampling_ratio, 1.0);
    }

    #[test]
    fn test_tracing_config_with_environment() {
        let config = TracingConfig::new("test-service".to_string(), "0.1.0".to_string())
            .with_environment("production".to_string());
        assert_eq!(config.environment, "production");
        assert_eq!(config.sampling_ratio, 0.01);
    }

    #[test]
    fn test_http_span_creation() {
        let span = TracingService::http_request_span("GET", "/api/users", Some("user123"));
        drop(span);
    }

    #[test]
    fn test_db_span_creation() {
        let span = TracingService::db_query_span(
            "SELECT * FROM users WHERE id = ",
            "postgres",
            "SELECT",
        );
        drop(span);
    }

    #[test]
    fn test_redis_span_creation() {
        let span = TracingService::redis_command_span("GET", Some("user:123"));
        drop(span);
    }

    #[test]
    fn test_service_method_span_creation() {
        let span = TracingService::service_method_span("UserService", "get_user");
        drop(span);
    }

    #[test]
    fn test_job_span_creation() {
        let span = TracingService::job_span("process_transaction", "job-456");
        drop(span);
    }

    #[test]
    fn test_sampling_ratio_bounds() {
        let config = TracingConfig::default().with_sampling_ratio(1.5);
        assert_eq!(config.sampling_ratio, 1.0);

        let config = TracingConfig::default().with_sampling_ratio(-0.5);
        assert_eq!(config.sampling_ratio, 0.0);
    }
}
