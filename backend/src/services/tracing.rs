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
use serde_json::{json, Map, Value};
use std::{
    fmt,
    fs::{self, OpenOptions},
    io::{self, Write},
    path::PathBuf,
    sync::Mutex,
    time::{Duration, SystemTime, UNIX_EPOCH},
};
use tracing::{field::{Field, Visit}, info_span, span::{Attributes, Record}, warn, Id, Subscriber};
use tracing_subscriber::{
    layer::{Context, Layer},
    registry::LookupSpan,
    layer::SubscriberExt,
    util::SubscriberInitExt,
    EnvFilter,
};

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


/// Durable local copy of completed tracing spans.
///
/// OTLP's batch processor is still used for normal remote export. This layer
/// independently appends completed spans to JSON Lines on disk, so telemetry
/// remains available locally if the collector is unavailable or the in-memory
/// exporter queue overflows. The spool is bounded and rotates to one archive.
///
/// Configure with:
/// - `CRUCIBLE_OTEL_SPOOL_PATH` (default: `./data/otel-spool/spans.jsonl`)
/// - `CRUCIBLE_OTEL_SPOOL_MAX_BYTES` (default: 67,108,864 bytes / 64 MiB)
///
/// The JSONL spool is a durable local fallback; it is not automatically replayed
/// to the OTLP collector. An external recovery job can ship the retained records.
#[derive(Debug)]
struct DiskSpoolLayer {
    path: PathBuf,
    max_bytes: u64,
    write_lock: Mutex<()>,
}

#[derive(Default, Debug)]
struct SpanFields(Map<String, Value>);

impl DiskSpoolLayer {
    fn from_env() -> anyhow::Result<Self> {
        let path = std::env::var_os("CRUCIBLE_OTEL_SPOOL_PATH")
            .map(PathBuf::from)
            .unwrap_or_else(|| PathBuf::from("./data/otel-spool/spans.jsonl"));

        let max_bytes = std::env::var("CRUCIBLE_OTEL_SPOOL_MAX_BYTES")
            .ok()
            .map(|value| value.parse::<u64>())
            .transpose()
            .map_err(|error| anyhow::anyhow!("Invalid CRUCIBLE_OTEL_SPOOL_MAX_BYTES: {error}"))?
            .unwrap_or(64 * 1024 * 1024);

        if max_bytes == 0 {
            anyhow::bail!("CRUCIBLE_OTEL_SPOOL_MAX_BYTES must be greater than zero");
        }

        if let Some(parent) = path.parent().filter(|parent| !parent.as_os_str().is_empty()) {
            fs::create_dir_all(parent).map_err(|error| {
                anyhow::anyhow!("Failed to create OTLP spool directory {}: {error}", parent.display())
            })?;
        }

        // Ensure the configured path is writable before enabling the layer.
        let mut options = OpenOptions::new();
        options.create(true).append(true);
        #[cfg(unix)]
        {
            use std::os::unix::fs::OpenOptionsExt;
            options.mode(0o600);
        }
        options.open(&path).map_err(|error| {
            anyhow::anyhow!("Failed to open OTLP spool file {}: {error}", path.display())
        })?;

        Ok(Self {
            path,
            max_bytes,
            write_lock: Mutex::new(()),
        })
    }

    fn append_record(&self, record: &Value) -> io::Result<()> {
        let _guard = self
            .write_lock
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);

        if self.max_bytes == 0 {
            return Err(io::Error::new(
                io::ErrorKind::InvalidInput,
                "spool size limit must be greater than zero",
            ));
        }

        let mut line = serde_json::to_vec(record).map_err(io::Error::other)?;
        line.push(b'\n');

        let current_len = fs::metadata(&self.path)
            .map(|metadata| metadata.len())
            .unwrap_or(0);

        // Rotate before writing to keep disk usage bounded to approximately
        // two spool files. Oversized single records are skipped rather than
        // violating the configured disk limit.
        if line.len() as u64 > self.max_bytes {
            return Err(io::Error::new(
                io::ErrorKind::InvalidData,
                "serialized span exceeds the configured spool size limit",
            ));
        }

        if current_len.saturating_add(line.len() as u64) > self.max_bytes {
            let archive = self.path.with_extension("jsonl.1");
            if archive.exists() {
                fs::remove_file(&archive)?;
            }
            if self.path.exists() {
                fs::rename(&self.path, &archive)?;
            }
        }

        let mut options = OpenOptions::new();
        options.create(true).append(true);
        #[cfg(unix)]
        {
            use std::os::unix::fs::OpenOptionsExt;
            options.mode(0o600);
        }
        let mut file = options.open(&self.path)?;
        file.write_all(&line)?;
        file.flush()?;
        // Persist the record before returning so a process/host crash is less
        // likely to lose spans that were already acknowledged by the spool.
        file.sync_data()?;
        Ok(())
    }
}

#[derive(Default)]
struct FieldVisitor {
    fields: Map<String, Value>,
}

impl Visit for FieldVisitor {
    fn record_bool(&mut self, field: &Field, value: bool) {
        self.fields.insert(field.name().to_owned(), Value::Bool(value));
    }

    fn record_i64(&mut self, field: &Field, value: i64) {
        self.fields.insert(field.name().to_owned(), json!(value));
    }

    fn record_u64(&mut self, field: &Field, value: u64) {
        self.fields.insert(field.name().to_owned(), json!(value));
    }

    fn record_f64(&mut self, field: &Field, value: f64) {
        let json_value = serde_json::Number::from_f64(value)
            .map(Value::Number)
            .unwrap_or(Value::Null);
        self.fields.insert(field.name().to_owned(), json_value);
    }

    fn record_str(&mut self, field: &Field, value: &str) {
        self.fields
            .insert(field.name().to_owned(), Value::String(value.to_owned()));
    }

    fn record_debug(&mut self, field: &Field, value: &dyn fmt::Debug) {
        self.fields.insert(
            field.name().to_owned(),
            Value::String(format!("{value:?}")),
        );
    }
}

impl<S> Layer<S> for DiskSpoolLayer
where
    S: Subscriber + for<'lookup> LookupSpan<'lookup>,
{
    fn on_new_span(&self, attributes: &Attributes<'_>, id: &Id, context: Context<'_, S>) {
        let Some(span) = context.span(id) else {
            return;
        };

        let mut visitor = FieldVisitor::default();
        attributes.record(&mut visitor);
        span.extensions_mut().insert(SpanFields(visitor.fields));
    }

    fn on_record(&self, id: &Id, values: &Record<'_>, context: Context<'_, S>) {
        let Some(span) = context.span(id) else {
            return;
        };

        let mut extensions = span.extensions_mut();
        let fields = extensions.get_mut::<SpanFields>();
        let mut visitor = FieldVisitor::default();
        values.record(&mut visitor);

        if let Some(fields) = fields {
            fields.0.extend(visitor.fields);
        } else {
            extensions.insert(SpanFields(visitor.fields));
        }
    }

    fn on_close(&self, id: Id, context: Context<'_, S>) {
        let Some(span) = context.span(&id) else {
            return;
        };
        let metadata = span.metadata();
        let fields = span
            .extensions()
            .get::<SpanFields>()
            .map(|fields| Value::Object(fields.0.clone()))
            .unwrap_or_else(|| Value::Object(Map::new()));
        let timestamp_unix_ms = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap_or_default()
            .as_millis();

        let record = json!({
            "record_type": "completed_span",
            "timestamp_unix_ms": timestamp_unix_ms,
            "name": metadata.name(),
            "target": metadata.target(),
            "level": metadata.level().to_string(),
            "module_path": metadata.module_path(),
            "file": metadata.file(),
            "line": metadata.line(),
            "fields": fields,
        });

        if let Err(error) = self.append_record(&record) {
            // Avoid tracing from inside the tracing layer to prevent recursion.
            eprintln!("failed to spool completed tracing span: {error}");
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

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum OtlpProtocol {
    Grpc,
    HttpBinary,
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
                KeyValue::new(resource::DEPLOYMENT_ENVIRONMENT_NAME, config.environment.clone()),
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

        // Persist completed spans locally as a fallback when the remote OTLP
        // collector is unavailable. Startup fails if durable spooling cannot be
        // initialized, rather than silently running without the configured fallback.
        let disk_spool_layer = DiskSpoolLayer::from_env()?;

        if json_logs {
            tracing_subscriber::registry()
                .with(filter)
                .with(telemetry_layer)
                .with(disk_spool_layer)
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
                .with(disk_spool_layer)
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

    /// Inject trace context into a carrier map for distributed propagation
    pub fn inject_trace_context(
        cx: &opentelemetry::Context,
    ) -> std::collections::HashMap<String, String> {
        use opentelemetry::propagation::TextMapPropagator;
        let propagator = opentelemetry_sdk::propagation::TraceContextPropagator::new();
        let mut carrier = std::collections::HashMap::new();
        propagator.inject_context(cx, &mut carrier);
        carrier
    }

    /// Extract trace context from a carrier map for distributed propagation
    pub fn extract_trace_context(
        carrier: &std::collections::HashMap<String, String>,
    ) -> opentelemetry::Context {
        use opentelemetry::propagation::TextMapPropagator;
        let propagator = opentelemetry_sdk::propagation::TraceContextPropagator::new();
        propagator.extract(carrier)
    }

    /// Extract W3C Trace Context headers from HTTP request headers and attach to span
    pub fn extract_and_attach_trace_context(headers: &axum::http::HeaderMap, span: &tracing::Span) {
        use opentelemetry::propagation::TextMapPropagator;
        use tracing_opentelemetry::OpenTelemetrySpanExt;

        let mut carrier = std::collections::HashMap::new();
        for (name, val) in headers.iter() {
            if let Ok(v) = val.to_str() {
                carrier.insert(name.as_str().to_string(), v.to_string());
            }
        }
        let propagator = opentelemetry_sdk::propagation::TraceContextPropagator::new();
        let parent_cx = propagator.extract(&carrier);
        span.set_parent(parent_cx);
    }

    /// Spawn an asynchronous background task with current OpenTelemetry trace span context attached.
    pub fn spawn_instrumented<F, T>(future: F) -> tokio::task::JoinHandle<T>
    where
        F: std::future::Future<Output = T> + Send + 'static,
        T: Send + 'static,
    {
        use tracing::Instrument;
        tokio::spawn(future.instrument(tracing::Span::current()))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_disk_spool_persists_json_lines_and_rotates() {
        use std::time::{SystemTime, UNIX_EPOCH};

        let unique = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .expect("clock should be after epoch")
            .as_nanos();
        let directory = std::env::temp_dir().join(format!("crucible-otel-spool-{unique}"));
        std::fs::create_dir_all(&directory).expect("temporary directory should be created");
        let path = directory.join("spans.jsonl");
        let spool = DiskSpoolLayer {
            path: path.clone(),
            max_bytes: 80,
            write_lock: Mutex::new(()),
        };

        spool.append_record(&json!({"span": "first"})).expect("first span should spool");
        spool.append_record(&json!({"span": "second"})).expect("second span should spool");
        spool.append_record(&json!({"span": "third", "payload": "large enough to rotate"}))
            .expect("third span should spool after rotation");

        let current = std::fs::read_to_string(&path).expect("current spool should be readable");
        let archive = std::fs::read_to_string(directory.join("spans.jsonl.1"))
            .expect("rotated spool should be readable");
        assert!(!current.trim().is_empty());
        assert!(!archive.trim().is_empty());
        assert!(current.lines().all(|line| serde_json::from_str::<Value>(line).is_ok()));
        assert!(archive.lines().all(|line| serde_json::from_str::<Value>(line).is_ok()));

        std::fs::remove_dir_all(directory).expect("temporary spool directory should be removed");
    }

    #[test]
    fn test_disk_spool_rejects_zero_size_limit() {
        let spool = DiskSpoolLayer {
            path: PathBuf::from("unused-spool.jsonl"),
            max_bytes: 0,
            write_lock: Mutex::new(()),
        };

        let error = spool.append_record(&json!({"span": "test"}));
        assert!(error.is_err());
    }

    #[test]
    fn test_trace_context_extraction_and_injection() {
        let mut carrier = std::collections::HashMap::new();
        carrier.insert(
            "traceparent".to_string(),
            "00-4bf92f3577b34da6a3ce929d0e0e4736-00f067aa0ba902b7-01".to_string(),
        );
        let cx = TracingService::extract_trace_context(&carrier);
        let injected = TracingService::inject_trace_context(&cx);
        assert!(injected.contains_key("traceparent"));
    }

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

    #[tokio::test]
    async fn test_spawn_instrumented_context_propagation() {
        let parent_span = TracingService::job_span("test_async_job", "job-1023");
        let _guard = parent_span.enter();

        let handle = TracingService::spawn_instrumented(async {
            tracing::Span::current().is_none()
        });

        let is_span_none = handle.await.unwrap();
        assert!(!is_span_none);
    }
}
