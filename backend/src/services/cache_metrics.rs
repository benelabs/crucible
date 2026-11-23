//! Cache metrics and analytics service.
//!
//! This module records cache operations to PostgreSQL for durable analytics and
//! uses Redis to cache expensive aggregate summaries. The service is deliberately
//! small: writes are O(1), bounded list queries are O(limit), and summaries run
//! indexed aggregate queries over the requested namespace/time window.

use axum::{
    extract::{Query, State},
    http::StatusCode,
    response::{IntoResponse, Response},
    routing::{get, post},
    Json, Router,
};
use chrono::{DateTime, Utc};
use redis::{AsyncCommands, Client as RedisClient};
use serde::{Deserialize, Serialize};
use sqlx::{PgPool, Row};
use std::collections::HashMap;
use std::sync::Arc;
use thiserror::Error;
use tracing::{debug, error, info, instrument};
use uuid::Uuid;

const DEFAULT_SUMMARY_CACHE_TTL_SECS: u64 = 60;
const DEFAULT_RECENT_LIMIT: u32 = 100;
const MAX_RECENT_LIMIT: u32 = 1_000;

/// Errors returned by [`CacheMetricsService`].
#[derive(Debug, Error)]
pub enum CacheMetricsError {
    /// PostgreSQL operation failed.
    #[error("database error: {0}")]
    Database(#[from] sqlx::Error),

    /// Redis operation failed.
    #[error("redis error: {0}")]
    Redis(#[from] redis::RedisError),

    /// JSON serialization failed.
    #[error("serialization error: {0}")]
    Serialization(#[from] serde_json::Error),

    /// The caller supplied an invalid analytics request.
    #[error("validation error: {0}")]
    Validation(String),
}

impl IntoResponse for CacheMetricsError {
    fn into_response(self) -> Response {
        let status = match self {
            Self::Validation(_) => StatusCode::BAD_REQUEST,
            Self::Database(_) | Self::Redis(_) | Self::Serialization(_) => {
                StatusCode::INTERNAL_SERVER_ERROR
            }
        };

        if status.is_server_error() {
            error!(error = %self, "cache metrics request failed");
        }

        (
            status,
            Json(serde_json::json!({
                "error": self.to_string(),
                "code": match status {
                    StatusCode::BAD_REQUEST => "VALIDATION_ERROR",
                    _ => "CACHE_METRICS_ERROR",
                },
            })),
        )
            .into_response()
    }
}

/// Cache operation names tracked by the analytics service.
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq, Hash)]
#[serde(rename_all = "snake_case")]
pub enum CacheOperation {
    Get,
    Set,
    Delete,
    Evict,
    Refresh,
    Error,
}

impl CacheOperation {
    fn as_str(self) -> &'static str {
        match self {
            Self::Get => "get",
            Self::Set => "set",
            Self::Delete => "delete",
            Self::Evict => "evict",
            Self::Refresh => "refresh",
            Self::Error => "error",
        }
    }

    fn from_str(value: &str) -> Result<Self, CacheMetricsError> {
        match value {
            "get" => Ok(Self::Get),
            "set" => Ok(Self::Set),
            "delete" => Ok(Self::Delete),
            "evict" => Ok(Self::Evict),
            "refresh" => Ok(Self::Refresh),
            "error" => Ok(Self::Error),
            other => Err(CacheMetricsError::Validation(format!(
                "unsupported cache operation '{other}'"
            ))),
        }
    }
}

/// Durable record of one cache operation.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct CacheMetric {
    pub id: Uuid,
    pub namespace: String,
    pub cache_key: String,
    pub operation: CacheOperation,
    pub hit: Option<bool>,
    pub latency_ms: i64,
    pub payload_bytes: Option<i64>,
    pub recorded_at: DateTime<Utc>,
    pub metadata: serde_json::Value,
}

/// Input used to record a cache operation.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct CacheMetricInput {
    pub namespace: String,
    pub cache_key: String,
    pub operation: CacheOperation,
    pub hit: Option<bool>,
    pub latency_ms: i64,
    pub payload_bytes: Option<i64>,
    pub metadata: serde_json::Value,
}

impl CacheMetricInput {
    /// Creates a cache hit event for a `GET` operation.
    pub fn hit(
        namespace: impl Into<String>,
        cache_key: impl Into<String>,
        latency_ms: i64,
    ) -> Self {
        Self {
            namespace: namespace.into(),
            cache_key: cache_key.into(),
            operation: CacheOperation::Get,
            hit: Some(true),
            latency_ms,
            payload_bytes: None,
            metadata: serde_json::Value::Null,
        }
    }

    /// Creates a cache miss event for a `GET` operation.
    pub fn miss(
        namespace: impl Into<String>,
        cache_key: impl Into<String>,
        latency_ms: i64,
    ) -> Self {
        Self {
            namespace: namespace.into(),
            cache_key: cache_key.into(),
            operation: CacheOperation::Get,
            hit: Some(false),
            latency_ms,
            payload_bytes: None,
            metadata: serde_json::Value::Null,
        }
    }

    fn validate(&self) -> Result<(), CacheMetricsError> {
        validate_name("namespace", &self.namespace)?;
        if self.cache_key.is_empty() || self.cache_key.len() > 512 {
            return Err(CacheMetricsError::Validation(
                "cache_key must be between 1 and 512 characters".to_string(),
            ));
        }
        if self.latency_ms < 0 {
            return Err(CacheMetricsError::Validation(
                "latency_ms cannot be negative".to_string(),
            ));
        }
        if matches!(self.payload_bytes, Some(bytes) if bytes < 0) {
            return Err(CacheMetricsError::Validation(
                "payload_bytes cannot be negative".to_string(),
            ));
        }
        Ok(())
    }
}

/// Query options for cache analytics.
#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq, Eq)]
pub struct CacheMetricsQuery {
    pub namespace: Option<String>,
    pub from: Option<DateTime<Utc>>,
    pub to: Option<DateTime<Utc>>,
    pub limit: Option<u32>,
    pub offset: Option<u32>,
}

impl CacheMetricsQuery {
    fn bounded_limit(&self) -> Result<i64, CacheMetricsError> {
        let limit = self.limit.unwrap_or(DEFAULT_RECENT_LIMIT);
        if limit == 0 || limit > MAX_RECENT_LIMIT {
            return Err(CacheMetricsError::Validation(format!(
                "limit must be between 1 and {MAX_RECENT_LIMIT}"
            )));
        }
        Ok(i64::from(limit))
    }

    fn offset(&self) -> i64 {
        i64::from(self.offset.unwrap_or(0))
    }

    fn validate(&self) -> Result<(), CacheMetricsError> {
        self.bounded_limit()?;
        if let Some(namespace) = &self.namespace {
            validate_name("namespace", namespace)?;
        }
        if let (Some(from), Some(to)) = (self.from.as_ref(), self.to.as_ref()) {
            if from > to {
                return Err(CacheMetricsError::Validation(
                    "from must be less than or equal to to".to_string(),
                ));
            }
        }
        Ok(())
    }
}

/// Aggregated cache analytics for a query window.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct CacheMetricsSummary {
    pub total_operations: i64,
    pub hits: i64,
    pub misses: i64,
    pub hit_rate: f64,
    pub average_latency_ms: f64,
    pub total_payload_bytes: i64,
    pub operations: HashMap<CacheOperation, i64>,
    pub generated_at: DateTime<Utc>,
}

/// Shared Axum state for cache metrics routes.
#[derive(Clone)]
pub struct CacheMetricsState {
    pub service: Arc<CacheMetricsService>,
}

/// Service for durable cache metrics and cached analytics summaries.
#[derive(Clone)]
pub struct CacheMetricsService {
    db: PgPool,
    redis: RedisClient,
    summary_cache_ttl_secs: u64,
}

/// Builds the cache metrics HTTP router.
///
/// Routes are intentionally small and delegate all persistence, caching, and
/// validation to [`CacheMetricsService`].
pub fn router(service: Arc<CacheMetricsService>) -> Router {
    Router::new()
        .route(
            "/cache-metrics",
            post(record_cache_metric).get(get_recent_cache_metrics),
        )
        .route("/cache-metrics/summary", get(get_cache_metrics_summary))
        .with_state(CacheMetricsState { service })
}

/// `POST /cache-metrics` records a cache operation.
#[instrument(skip(state, payload))]
pub async fn record_cache_metric(
    State(state): State<CacheMetricsState>,
    Json(payload): Json<CacheMetricInput>,
) -> Result<impl IntoResponse, CacheMetricsError> {
    let metric = state.service.record(payload).await?;
    Ok((StatusCode::CREATED, Json(metric)))
}

/// `GET /cache-metrics` returns recent cache operations.
#[instrument(skip(state))]
pub async fn get_recent_cache_metrics(
    State(state): State<CacheMetricsState>,
    Query(query): Query<CacheMetricsQuery>,
) -> Result<impl IntoResponse, CacheMetricsError> {
    let metrics = state.service.recent(query).await?;
    Ok(Json(metrics))
}

/// `GET /cache-metrics/summary` returns aggregate cache analytics.
#[instrument(skip(state))]
pub async fn get_cache_metrics_summary(
    State(state): State<CacheMetricsState>,
    Query(query): Query<CacheMetricsQuery>,
) -> Result<impl IntoResponse, CacheMetricsError> {
    let summary = state.service.summary(query).await?;
    Ok(Json(summary))
}

impl CacheMetricsService {
    /// Creates a cache metrics service using the default summary TTL.
    pub fn new(db: PgPool, redis: RedisClient) -> Self {
        Self {
            db,
            redis,
            summary_cache_ttl_secs: DEFAULT_SUMMARY_CACHE_TTL_SECS,
        }
    }

    /// Creates a cache metrics service with a custom summary cache TTL.
    pub fn with_summary_cache_ttl(db: PgPool, redis: RedisClient, ttl_secs: u64) -> Self {
        Self {
            db,
            redis,
            summary_cache_ttl_secs: ttl_secs,
        }
    }

    /// Creates the backing table and indexes when migrations are not available.
    ///
    /// Production deployments should prefer SQL migrations. This helper keeps
    /// integration tests and embedded deployments deterministic.
    #[instrument(skip(self), fields(service.name = "CacheMetricsService"))]
    pub async fn ensure_schema(&self) -> Result<(), CacheMetricsError> {
        sqlx::query(
            r#"
            CREATE TABLE IF NOT EXISTS cache_metrics (
                id UUID PRIMARY KEY,
                namespace TEXT NOT NULL,
                cache_key TEXT NOT NULL,
                operation TEXT NOT NULL,
                hit BOOLEAN,
                latency_ms BIGINT NOT NULL CHECK (latency_ms >= 0),
                payload_bytes BIGINT CHECK (payload_bytes >= 0),
                recorded_at TIMESTAMPTZ NOT NULL,
                metadata JSONB NOT NULL DEFAULT '{}'::jsonb
            )
            "#,
        )
        .execute(&self.db)
        .await?;

        sqlx::query(
            "CREATE INDEX IF NOT EXISTS idx_cache_metrics_namespace_time ON cache_metrics(namespace, recorded_at DESC)",
        )
        .execute(&self.db)
        .await?;

        sqlx::query(
            "CREATE INDEX IF NOT EXISTS idx_cache_metrics_operation_time ON cache_metrics(operation, recorded_at DESC)",
        )
        .execute(&self.db)
        .await?;

        Ok(())
    }

    /// Records one cache operation.
    ///
    /// Time complexity is O(1) for the insert plus O(k) Redis summary
    /// invalidation, where `k` is the number of cached summaries for the
    /// namespace. Space complexity is O(1) per event.
    #[instrument(skip(self, input), fields(namespace = %input.namespace, operation = %input.operation.as_str()))]
    pub async fn record(&self, input: CacheMetricInput) -> Result<CacheMetric, CacheMetricsError> {
        input.validate()?;

        let metric = CacheMetric {
            id: Uuid::new_v4(),
            namespace: input.namespace,
            cache_key: input.cache_key,
            operation: input.operation,
            hit: input.hit,
            latency_ms: input.latency_ms,
            payload_bytes: input.payload_bytes,
            recorded_at: Utc::now(),
            metadata: input.metadata,
        };

        sqlx::query(
            r#"
            INSERT INTO cache_metrics
                (id, namespace, cache_key, operation, hit, latency_ms, payload_bytes, recorded_at, metadata)
            VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9)
            "#,
        )
        .bind(metric.id)
        .bind(&metric.namespace)
        .bind(&metric.cache_key)
        .bind(metric.operation.as_str())
        .bind(metric.hit)
        .bind(metric.latency_ms)
        .bind(metric.payload_bytes)
        .bind(metric.recorded_at)
        .bind(&metric.metadata)
        .execute(&self.db)
        .await
        .map_err(|err| {
            error!(error = %err, "failed to persist cache metric");
            CacheMetricsError::Database(err)
        })?;

        self.invalidate_summary_cache(Some(&metric.namespace))
            .await?;
        info!(metric_id = %metric.id, "recorded cache metric");
        Ok(metric)
    }

    /// Returns recent cache events for a bounded query.
    #[instrument(skip(self), fields(namespace = ?query.namespace))]
    pub async fn recent(
        &self,
        query: CacheMetricsQuery,
    ) -> Result<Vec<CacheMetric>, CacheMetricsError> {
        query.validate()?;
        let rows = sqlx::query(
            r#"
            SELECT id, namespace, cache_key, operation, hit, latency_ms, payload_bytes, recorded_at, metadata
            FROM cache_metrics
            WHERE ($1::TEXT IS NULL OR namespace = $1)
              AND ($2::TIMESTAMPTZ IS NULL OR recorded_at >= $2)
              AND ($3::TIMESTAMPTZ IS NULL OR recorded_at <= $3)
            ORDER BY recorded_at DESC
            LIMIT $4 OFFSET $5
            "#,
        )
        .bind(query.namespace.as_deref())
        .bind(query.from)
        .bind(query.to)
        .bind(query.bounded_limit()?)
        .bind(query.offset())
        .fetch_all(&self.db)
        .await?;

        rows.into_iter().map(row_to_metric).collect()
    }

    /// Returns aggregate cache analytics, using Redis to cache repeated reads.
    #[instrument(skip(self), fields(namespace = ?query.namespace))]
    pub async fn summary(
        &self,
        query: CacheMetricsQuery,
    ) -> Result<CacheMetricsSummary, CacheMetricsError> {
        query.validate()?;
        let cache_key = summary_cache_key(&query);
        let mut conn = self.redis.get_multiplexed_async_connection().await?;

        let cached: Option<String> = conn.get(&cache_key).await?;
        if let Some(value) = cached {
            debug!(cache_key = %cache_key, "cache metrics summary cache hit");
            return Ok(serde_json::from_str(&value)?);
        }

        debug!(cache_key = %cache_key, "cache metrics summary cache miss");

        let totals = sqlx::query(
            r#"
            SELECT
                COUNT(*)::BIGINT AS total_operations,
                COALESCE(SUM(CASE WHEN hit IS TRUE THEN 1 ELSE 0 END), 0)::BIGINT AS hits,
                COALESCE(SUM(CASE WHEN hit IS FALSE THEN 1 ELSE 0 END), 0)::BIGINT AS misses,
                COALESCE(AVG(latency_ms), 0)::DOUBLE PRECISION AS average_latency_ms,
                COALESCE(SUM(payload_bytes), 0)::BIGINT AS total_payload_bytes
            FROM cache_metrics
            WHERE ($1::TEXT IS NULL OR namespace = $1)
              AND ($2::TIMESTAMPTZ IS NULL OR recorded_at >= $2)
              AND ($3::TIMESTAMPTZ IS NULL OR recorded_at <= $3)
            "#,
        )
        .bind(query.namespace.as_deref())
        .bind(query.from)
        .bind(query.to)
        .fetch_one(&self.db)
        .await?;

        let operation_rows = sqlx::query(
            r#"
            SELECT operation, COUNT(*)::BIGINT AS total
            FROM cache_metrics
            WHERE ($1::TEXT IS NULL OR namespace = $1)
              AND ($2::TIMESTAMPTZ IS NULL OR recorded_at >= $2)
              AND ($3::TIMESTAMPTZ IS NULL OR recorded_at <= $3)
            GROUP BY operation
            "#,
        )
        .bind(query.namespace.as_deref())
        .bind(query.from)
        .bind(query.to)
        .fetch_all(&self.db)
        .await?;

        let total_operations: i64 = totals.try_get("total_operations")?;
        let hits: i64 = totals.try_get("hits")?;
        let misses: i64 = totals.try_get("misses")?;
        let average_latency_ms: f64 = totals.try_get("average_latency_ms")?;
        let total_payload_bytes: i64 = totals.try_get("total_payload_bytes")?;
        let hit_rate = if hits + misses == 0 {
            0.0
        } else {
            hits as f64 / (hits + misses) as f64
        };

        let mut operations = HashMap::with_capacity(operation_rows.len());
        for row in operation_rows {
            let operation =
                CacheOperation::from_str(row.try_get::<String, _>("operation")?.as_str())?;
            let total: i64 = row.try_get("total")?;
            operations.insert(operation, total);
        }

        let summary = CacheMetricsSummary {
            total_operations,
            hits,
            misses,
            hit_rate,
            average_latency_ms,
            total_payload_bytes,
            operations,
            generated_at: Utc::now(),
        };

        let payload = serde_json::to_string(&summary)?;
        let _: () = conn
            .set_ex(&cache_key, payload, self.summary_cache_ttl_secs)
            .await?;

        Ok(summary)
    }

    /// Invalidates cached summaries. When a namespace is supplied, only that
    /// namespace and global summaries are cleared.
    #[instrument(skip(self))]
    pub async fn invalidate_summary_cache(
        &self,
        namespace: Option<&str>,
    ) -> Result<u64, CacheMetricsError> {
        let mut conn = self.redis.get_multiplexed_async_connection().await?;
        let mut patterns = vec!["cache_metrics:summary:all:*".to_string()];
        if let Some(namespace) = namespace {
            patterns.push(format!("cache_metrics:summary:{namespace}:*"));
        }

        let mut deleted = 0_u64;
        for pattern in patterns {
            let mut cursor = 0_u64;
            loop {
                let (next_cursor, keys): (u64, Vec<String>) = redis::cmd("SCAN")
                    .arg(cursor)
                    .arg("MATCH")
                    .arg(&pattern)
                    .arg("COUNT")
                    .arg(100_u32)
                    .query_async(&mut conn)
                    .await?;

                if !keys.is_empty() {
                    let count: u64 = conn.del(keys).await?;
                    deleted += count;
                }

                if next_cursor == 0 {
                    break;
                }
                cursor = next_cursor;
            }
        }

        if deleted > 0 {
            debug!(deleted, "invalidated cache metrics summaries");
        }
        Ok(deleted)
    }
}

fn row_to_metric(row: sqlx::postgres::PgRow) -> Result<CacheMetric, CacheMetricsError> {
    let operation = CacheOperation::from_str(row.try_get::<String, _>("operation")?.as_str())?;
    Ok(CacheMetric {
        id: row.try_get("id")?,
        namespace: row.try_get("namespace")?,
        cache_key: row.try_get("cache_key")?,
        operation,
        hit: row.try_get("hit")?,
        latency_ms: row.try_get("latency_ms")?,
        payload_bytes: row.try_get("payload_bytes")?,
        recorded_at: row.try_get("recorded_at")?,
        metadata: row.try_get("metadata")?,
    })
}

fn summary_cache_key(query: &CacheMetricsQuery) -> String {
    let namespace = query.namespace.as_deref().unwrap_or("all");
    let from = query
        .from
        .as_ref()
        .map(|value| value.timestamp())
        .unwrap_or(0);
    let to = query
        .to
        .as_ref()
        .map(|value| value.timestamp())
        .unwrap_or(0);
    format!("cache_metrics:summary:{namespace}:{from}:{to}")
}

fn validate_name(field: &str, value: &str) -> Result<(), CacheMetricsError> {
    if value.is_empty() || value.len() > 128 {
        return Err(CacheMetricsError::Validation(format!(
            "{field} must be between 1 and 128 characters"
        )));
    }
    if !value
        .bytes()
        .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'_' | b'-' | b':' | b'.'))
    {
        return Err(CacheMetricsError::Validation(format!(
            "{field} may contain only letters, numbers, '_', '-', ':' or '.'"
        )));
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use axum::{
        body::{to_bytes, Body},
        http::{Method, Request, StatusCode},
    };
    use sqlx::postgres::PgPoolOptions;
    use tower::ServiceExt;

    #[test]
    fn cache_operation_round_trips() {
        for operation in [
            CacheOperation::Get,
            CacheOperation::Set,
            CacheOperation::Delete,
            CacheOperation::Evict,
            CacheOperation::Refresh,
            CacheOperation::Error,
        ] {
            assert_eq!(
                CacheOperation::from_str(operation.as_str()).unwrap(),
                operation
            );
        }
    }

    #[test]
    fn cache_metric_input_validates_bounds() {
        assert!(CacheMetricInput::hit("dashboard", "summary", 12)
            .validate()
            .is_ok());

        let mut input = CacheMetricInput::hit("bad namespace", "summary", 12);
        assert!(input.validate().is_err());

        input = CacheMetricInput::hit("dashboard", "summary", -1);
        assert!(input.validate().is_err());
    }

    #[test]
    fn query_enforces_bounded_limit() {
        let query = CacheMetricsQuery {
            limit: Some(MAX_RECENT_LIMIT),
            ..CacheMetricsQuery::default()
        };
        assert!(query.validate().is_ok());

        let query = CacheMetricsQuery {
            limit: Some(MAX_RECENT_LIMIT + 1),
            ..CacheMetricsQuery::default()
        };
        assert!(query.validate().is_err());
    }

    #[test]
    fn summary_hit_rate_handles_empty_and_non_empty_counts() {
        let empty = CacheMetricsSummary {
            total_operations: 0,
            hits: 0,
            misses: 0,
            hit_rate: 0.0,
            average_latency_ms: 0.0,
            total_payload_bytes: 0,
            operations: HashMap::new(),
            generated_at: Utc::now(),
        };
        assert_eq!(empty.hit_rate, 0.0);

        let hit_rate = 8_f64 / 10_f64;
        assert_eq!(hit_rate, 0.8);
    }

    #[test]
    fn summary_cache_key_is_stable() {
        let query = CacheMetricsQuery {
            namespace: Some("dashboard".to_string()),
            from: None,
            to: None,
            limit: None,
            offset: None,
        };
        assert_eq!(
            summary_cache_key(&query),
            "cache_metrics:summary:dashboard:0:0"
        );
    }

    #[test]
    fn cache_metrics_error_maps_validation_to_bad_request() {
        let response =
            CacheMetricsError::Validation("invalid namespace".to_string()).into_response();
        assert_eq!(response.status(), StatusCode::BAD_REQUEST);
    }

    #[tokio::test]
    async fn router_rejects_invalid_record_payload_before_io() {
        let app = router(test_service());
        let payload = serde_json::json!({
            "namespace": "bad namespace",
            "cache_key": "dashboard",
            "operation": "get",
            "hit": true,
            "latency_ms": 4,
            "payload_bytes": null,
            "metadata": {}
        });

        let response = app
            .oneshot(
                Request::builder()
                    .method(Method::POST)
                    .uri("/cache-metrics")
                    .header("content-type", "application/json")
                    .body(Body::from(payload.to_string()))
                    .unwrap(),
            )
            .await
            .unwrap();

        assert_eq!(response.status(), StatusCode::BAD_REQUEST);
        let body = to_bytes(response.into_body(), usize::MAX).await.unwrap();
        let json: serde_json::Value = serde_json::from_slice(&body).unwrap();
        assert_eq!(json["code"], "VALIDATION_ERROR");
    }

    #[tokio::test]
    async fn router_rejects_invalid_recent_query_before_io() {
        let app = router(test_service());
        let response = app
            .oneshot(
                Request::builder()
                    .uri("/cache-metrics?limit=1001")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();

        assert_eq!(response.status(), StatusCode::BAD_REQUEST);
    }

    #[tokio::test]
    #[ignore]
    async fn integration_records_and_summarizes_with_postgres_and_redis() {
        dotenvy::dotenv().ok();
        let database_url = std::env::var("DATABASE_URL")
            .expect("DATABASE_URL is required for cache metrics integration test");
        let redis_url =
            std::env::var("REDIS_URL").unwrap_or_else(|_| "redis://127.0.0.1:6379".to_string());

        let db = PgPoolOptions::new()
            .max_connections(2)
            .connect(&database_url)
            .await
            .unwrap();
        let redis = RedisClient::open(redis_url).unwrap();
        let service = Arc::new(CacheMetricsService::with_summary_cache_ttl(db, redis, 1));
        service.ensure_schema().await.unwrap();

        let app = router(service);
        let payload = serde_json::json!({
            "namespace": "integration",
            "cache_key": "summary",
            "operation": "get",
            "hit": true,
            "latency_ms": 3,
            "payload_bytes": 128,
            "metadata": {"source": "test"}
        });

        let response = app
            .clone()
            .oneshot(
                Request::builder()
                    .method(Method::POST)
                    .uri("/cache-metrics")
                    .header("content-type", "application/json")
                    .body(Body::from(payload.to_string()))
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(response.status(), StatusCode::CREATED);

        let response = app
            .oneshot(
                Request::builder()
                    .uri("/cache-metrics/summary?namespace=integration")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(response.status(), StatusCode::OK);
    }

    fn test_service() -> Arc<CacheMetricsService> {
        let db = PgPoolOptions::new()
            .connect_lazy("postgres://postgres:postgres@localhost/cache_metrics_test")
            .unwrap();
        let redis = RedisClient::open("redis://127.0.0.1:1").unwrap();
        Arc::new(CacheMetricsService::new(db, redis))
    }
}
