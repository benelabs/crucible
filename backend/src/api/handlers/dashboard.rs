//! Dashboard data API handler.
//!
//! Provides dashboard endpoints for aggregated system metrics, contract
//! metrics, and active runtime state.
//! Results are cached in Redis for [`CACHE_TTL_SECS`] seconds to reduce load
//! on downstream services.
//!
//! # Example
//! ```rust,no_run
//! use std::sync::Arc;
//! use axum::{Router, routing::get};
//! use backend::api::handlers::dashboard::{DashboardState, get_dashboard};
//!
//! # async fn example() {
//! // state is constructed with your real service instances
//! # }
//! ```

use axum::{
    extract::{Path, State},
    response::IntoResponse,
    Json,
};
use chrono::{DateTime, Utc};
use rand::Rng;
use redis::{AsyncCommands, Client as RedisClient};
use serde::{de::DeserializeOwned, Deserialize, Serialize};
use sqlx::PgPool;
use std::sync::Arc;
use thiserror::Error;
use tracing::{debug, error, warn};
use utoipa::ToSchema;

use crate::services::{
    error_recovery::{ErrorManager, RecoveryTask},
    log_alerts::{Alert, AlertManager},
    sys_metrics::{MetricsExporter, SystemMetrics},
};

// ---------------------------------------------------------------------------
// Constants
// ---------------------------------------------------------------------------

const CACHE_KEY: &str = "dashboard:summary";
const CACHE_TTL_SECS: u64 = 30;

// ---------------------------------------------------------------------------
// Error type
// ---------------------------------------------------------------------------

/// Errors that can occur while building the dashboard response.
#[derive(Debug, Error)]
pub enum DashboardError {
    /// A Redis error occurred.
    #[error("Cache error: {0}")]
    Cache(#[from] redis::RedisError),

    /// A database error occurred.
    #[error("Database error: {0}")]
    Database(#[from] sqlx::Error),

    /// JSON serialization/deserialization failed.
    #[error("Serialization error: {0}")]
    Serialization(#[from] serde_json::Error),

    /// The requested resource was not found.
    #[error("{0}")]
    NotFound(String),
}

impl IntoResponse for DashboardError {
    fn into_response(self) -> axum::response::Response {
        let status = match self {
            DashboardError::NotFound(_) => axum::http::StatusCode::NOT_FOUND,
            _ => axum::http::StatusCode::INTERNAL_SERVER_ERROR,
        };

        error!(error = %self, status = %status, "Dashboard handler error");
        let body = serde_json::json!({ "error": self.to_string() });
        (status, Json(body)).into_response()
    }
}

// ---------------------------------------------------------------------------
// Response types
// ---------------------------------------------------------------------------

/// Aggregated dashboard data returned by `GET /api/dashboard`.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DashboardData {
    /// Current system metrics snapshot.
    pub metrics: SystemMetrics,
    /// Recovery tasks that are currently active.
    pub active_recovery_tasks: Vec<RecoveryTask>,
    /// Alerts that have fired and not yet been resolved.
    pub active_alerts: Vec<Alert>,
}

/// Aggregated dashboard metrics for the build and contract overview.
#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
pub struct DashboardMetrics {
    pub total_contracts: u64,
    pub total_transactions: u64,
    pub avg_processing_time_ms: f64,
    pub failed_transactions_24h: u64,
    pub timestamp: DateTime<Utc>,
}

/// Contract-specific usage metrics.
#[derive(Debug, Clone, Serialize, Deserialize, ToSchema, PartialEq)]
pub struct ContractStats {
    pub contract_id: String,
    pub invocation_count: u64,
    pub last_invoked: Option<DateTime<Utc>>,
    pub avg_gas_cost: f64,
}

/// Envelope for cached items storing metadata for the XFetch probabilistic early expiration algorithm.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct XFetchEnvelope<T> {
    pub value: T,
    pub created_at: i64,
    pub delta_secs: f64,
    pub ttl_secs: u64,
}

/// Metadata item for calculating XFetch probabilistic early expiration.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct XFetchItem {
    pub created_at: i64,
    pub delta_secs: f64,
    pub ttl_secs: u64,
}

impl XFetchItem {
    pub fn new(created_at: i64, delta_secs: f64, ttl_secs: u64) -> Self {
        Self {
            created_at,
            delta_secs: delta_secs.max(0.001),
            ttl_secs,
        }
    }

    /// Evaluates the XFetch probabilistic early expiration condition:
    /// `now - (delta * beta * ln(rand)) > expiry`
    ///
    /// When `now >= expiry`, this unconditionally returns `true`.
    /// When `now < expiry`, the probability of early expiration increases as `now` approaches `expiry`.
    pub fn should_refresh_early(&self, beta: f64) -> bool {
        let now = Utc::now().timestamp() as f64;
        let expiry = (self.created_at + self.ttl_secs as i64) as f64;

        if now >= expiry {
            return true;
        }

        let mut rng = rand::thread_rng();
        let u: f64 = rng.gen_range(0.000001..=1.0);
        now - (self.delta_secs * beta * u.ln()) > expiry
    }
}

/// Request payload for batch contract stats aggregation.
#[derive(Debug, Clone, Serialize, Deserialize, ToSchema, PartialEq, Eq)]
pub struct BatchContractStatsRequest {
    pub contract_ids: Vec<String>,
}

/// Query parameters for batch contract stats aggregation.
#[derive(Debug, Clone, Deserialize)]
pub struct BatchContractStatsQuery {
    pub contract_ids: Option<String>,
}

/// Aggregated response containing batched contract statistics.
#[derive(Debug, Clone, Serialize, Deserialize, ToSchema, PartialEq)]
pub struct BatchContractStatsResponse {
    pub stats: Vec<ContractStats>,
}

// ---------------------------------------------------------------------------
// State
// ---------------------------------------------------------------------------

/// Shared application state for the dashboard handler.
pub struct DashboardState {
    pub metrics_exporter: Arc<MetricsExporter>,
    pub error_manager: Arc<ErrorManager>,
    pub alert_manager: Arc<AlertManager>,
    pub db: PgPool,
    pub redis: RedisClient,
}

// ---------------------------------------------------------------------------
// Handler
// ---------------------------------------------------------------------------

/// `GET /api/dashboard` — return aggregated dashboard data.
///
/// Attempts to serve a cached response from Redis with cache stampede prevention.
/// Implements XFetch probabilistic early expiration and distributed mutex locking.
#[tracing::instrument(skip(state))]
pub async fn get_dashboard(
    State(state): State<Arc<DashboardState>>,
) -> Result<impl IntoResponse, DashboardError> {
    let lock_key = "lock:dashboard:summary:recompute";

    // --- try cache with XFetch probabilistic early expiration ---
    match try_cache_get_xfetch::<DashboardData>(&state.redis, CACHE_KEY).await {
        Ok(Some((cached, item))) => {
            if !item.should_refresh_early(1.0) {
                debug!("Dashboard cache hit");
                return Ok(Json(cached));
            }
            debug!("Dashboard summary XFetch early expiration triggered");
            let lock_token = uuid::Uuid::new_v4().to_string();
            if let Ok(true) =
                try_acquire_redis_lock(&state.redis, lock_key, &lock_token, 10_000).await
            {
                let start = std::time::Instant::now();
                let (metrics, active_recovery_tasks, active_alerts) = tokio::join!(
                    state.metrics_exporter.get_metrics(),
                    state.error_manager.get_active_tasks(),
                    state.alert_manager.get_active_alerts(),
                );
                let data = DashboardData {
                    metrics,
                    active_recovery_tasks,
                    active_alerts,
                };
                let delta = start.elapsed().as_secs_f64();
                let _ = try_cache_set_xfetch(&state.redis, CACHE_KEY, &data, delta, CACHE_TTL_SECS)
                    .await;
                release_redis_lock(&state.redis, lock_key, &lock_token).await;
                return Ok(Json(data));
            } else {
                return Ok(Json(cached));
            }
        }
        Ok(None) => debug!("Dashboard cache miss"),
        Err(e) => warn!(error = %e, "Dashboard cache read failed; falling back to live data"),
    }

    let lock_token = uuid::Uuid::new_v4().to_string();
    let is_leader = try_acquire_redis_lock(&state.redis, lock_key, &lock_token, 10_000)
        .await
        .unwrap_or(true);

    if !is_leader {
        for _ in 0..20 {
            tokio::time::sleep(std::time::Duration::from_millis(50)).await;
            if let Ok(Some((cached, _))) =
                try_cache_get_xfetch::<DashboardData>(&state.redis, CACHE_KEY).await
            {
                return Ok(Json(cached));
            }
        }
    }

    let start = std::time::Instant::now();
    let (metrics, active_recovery_tasks, active_alerts) = tokio::join!(
        state.metrics_exporter.get_metrics(),
        state.error_manager.get_active_tasks(),
        state.alert_manager.get_active_alerts(),
    );

    let data = DashboardData {
        metrics,
        active_recovery_tasks,
        active_alerts,
    };
    let delta = start.elapsed().as_secs_f64();

    if let Err(e) =
        try_cache_set_xfetch(&state.redis, CACHE_KEY, &data, delta, CACHE_TTL_SECS).await
    {
        warn!(error = %e, "Failed to populate dashboard cache");
    }

    if is_leader {
        release_redis_lock(&state.redis, lock_key, &lock_token).await;
    }

    Ok(Json(data))
}

/// Compute live dashboard aggregate metrics from the database.
pub async fn compute_live_dashboard_metrics(
    pool: &PgPool,
) -> Result<DashboardMetrics, DashboardError> {
    let total_contracts: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM contracts")
        .fetch_one(pool)
        .await?;
    let total_transactions: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM transactions")
        .fetch_one(pool)
        .await?;
    let avg_processing_time_ms: Option<f64> = sqlx::query_scalar(
        "SELECT AVG(processing_time_ms) FROM transactions WHERE processing_time_ms IS NOT NULL",
    )
    .fetch_one(pool)
    .await?;
    let failed_transactions_24h: i64 = sqlx::query_scalar(
        "SELECT COUNT(*) FROM transactions WHERE status = 'failed' AND created_at >= NOW() - INTERVAL '24 hours'",
    )
    .fetch_one(pool)
    .await?;

    Ok(DashboardMetrics {
        total_contracts: total_contracts as u64,
        total_transactions: total_transactions as u64,
        avg_processing_time_ms: avg_processing_time_ms.unwrap_or(0.0),
        failed_transactions_24h: failed_transactions_24h as u64,
        timestamp: Utc::now(),
    })
}

/// `GET /api/v1/dashboard/metrics` — return aggregate contract and pipeline metrics.
///
/// Implements cache stampede prevention using the XFetch probabilistic early expiration
/// algorithm combined with distributed mutex locking, ensuring that only a single worker
/// recomputes metrics when the cache expires or approaches expiration.
#[utoipa::path(
    get,
    path = "/api/v1/dashboard/metrics",
    responses((status = 200, description = "Dashboard metrics", body = DashboardMetrics)),
    tag = "dashboard"
)]
#[tracing::instrument(skip(state))]
pub async fn get_dashboard_metrics(
    State(state): State<Arc<DashboardState>>,
) -> Result<impl IntoResponse, DashboardError> {
    let cache_key = "dashboard:metrics";
    let lock_key = "lock:dashboard:metrics:recompute";

    match try_cache_get_xfetch::<DashboardMetrics>(&state.redis, cache_key).await {
        Ok(Some((cached_metrics, item))) => {
            if !item.should_refresh_early(1.0) {
                debug!("Dashboard metrics cache hit (fresh)");
                return Ok(Json(cached_metrics));
            }

            debug!("Dashboard metrics XFetch early expiration triggered");
            let lock_token = uuid::Uuid::new_v4().to_string();
            if let Ok(true) =
                try_acquire_redis_lock(&state.redis, lock_key, &lock_token, 10_000).await
            {
                let start = std::time::Instant::now();
                match compute_live_dashboard_metrics(&state.db).await {
                    Ok(new_metrics) => {
                        let delta = start.elapsed().as_secs_f64();
                        let _ = try_cache_set_xfetch(
                            &state.redis,
                            cache_key,
                            &new_metrics,
                            delta,
                            CACHE_TTL_SECS,
                        )
                        .await;
                        release_redis_lock(&state.redis, lock_key, &lock_token).await;
                        return Ok(Json(new_metrics));
                    }
                    Err(e) => {
                        warn!(error = %e, "Metrics recomputation failed; serving stale cache fallback");
                        release_redis_lock(&state.redis, lock_key, &lock_token).await;
                        return Ok(Json(cached_metrics));
                    }
                }
            } else {
                debug!("Another worker is recomputing metrics; serving cached value to prevent stampede");
                return Ok(Json(cached_metrics));
            }
        }
        Ok(None) => debug!("Dashboard metrics cache miss"),
        Err(e) => {
            warn!(error = %e, "Dashboard metrics cache read failed; falling back to live data")
        }
    }

    let lock_token = uuid::Uuid::new_v4().to_string();
    let is_leader = try_acquire_redis_lock(&state.redis, lock_key, &lock_token, 10_000)
        .await
        .unwrap_or(true);

    if !is_leader {
        for _ in 0..20 {
            tokio::time::sleep(std::time::Duration::from_millis(50)).await;
            if let Ok(Some((cached_metrics, _))) =
                try_cache_get_xfetch::<DashboardMetrics>(&state.redis, cache_key).await
            {
                debug!("Retrieved metrics from cache populated by leader worker");
                return Ok(Json(cached_metrics));
            }
        }
        warn!("Timed out waiting for leader worker to compute metrics; computing directly");
    }

    let start = std::time::Instant::now();
    let metrics = compute_live_dashboard_metrics(&state.db).await?;
    let delta = start.elapsed().as_secs_f64();

    if let Err(e) =
        try_cache_set_xfetch(&state.redis, cache_key, &metrics, delta, CACHE_TTL_SECS).await
    {
        warn!(error = %e, "Failed to populate dashboard metrics cache");
    }

    if is_leader {
        release_redis_lock(&state.redis, lock_key, &lock_token).await;
    }

    Ok(Json(metrics))
}

/// Query aggregated stats for a batch of contract IDs using a single consolidated SQL query
/// with `GROUP BY` and window ranking, eliminating N+1 database queries.
pub async fn fetch_batch_contract_stats(
    pool: &PgPool,
    contract_ids: &[String],
) -> Result<Vec<ContractStats>, DashboardError> {
    if contract_ids.is_empty() {
        return Ok(Vec::new());
    }

    // Consolidated query joining contracts and transactions with GROUP BY and window function:
    // Computes invocation count, last invoked timestamp, and average gas cost for all requested contracts in a single pass.
    let rows: Vec<(String, i64, Option<DateTime<Utc>>, Option<f64>)> = sqlx::query_as(
        r#"
        WITH aggregated AS (
            SELECT 
                c.contract_id,
                COUNT(t.id) AS invocation_count,
                MAX(t.created_at) AS last_invoked,
                AVG(t.gas_cost) AS avg_gas_cost,
                ROW_NUMBER() OVER (PARTITION BY c.contract_id ORDER BY MAX(t.created_at) DESC NULLS LAST) AS _rn
            FROM contracts c
            LEFT JOIN transactions t ON c.contract_id = t.contract_id
            WHERE c.contract_id = ANY($1)
            GROUP BY c.contract_id
        )
        SELECT contract_id, invocation_count, last_invoked, avg_gas_cost
        FROM aggregated
        ORDER BY contract_id ASC
        "#,
    )
    .bind(contract_ids)
    .fetch_all(pool)
    .await?;

    let stats = rows
        .into_iter()
        .map(
            |(contract_id, count, last_invoked, avg_gas)| ContractStats {
                contract_id,
                invocation_count: count as u64,
                last_invoked,
                avg_gas_cost: avg_gas.unwrap_or(0.0),
            },
        )
        .collect();

    Ok(stats)
}

/// `GET /api/v1/dashboard/contracts/:contract_id/stats` — return contract usage statistics.
#[utoipa::path(
    get,
    path = "/api/v1/dashboard/contracts/{contract_id}/stats",
    params(
        ("contract_id" = String, Path, description = "Contract identifier")
    ),
    responses(
        (status = 200, description = "Contract stats", body = ContractStats),
        (status = 404, description = "Contract not found")
    ),
    tag = "dashboard"
)]
#[tracing::instrument(skip(state))]
pub async fn get_contract_stats(
    Path(contract_id): Path<String>,
    State(state): State<Arc<DashboardState>>,
) -> Result<impl IntoResponse, DashboardError> {
    let cache_key = format!("dashboard:contract_stats:{contract_id}");

    match try_cache_get(&state.redis, &cache_key).await {
        Ok(Some(cached)) => return Ok(Json(cached)),
        Ok(None) => debug!(contract_id = %contract_id, "Contract stats cache miss"),
        Err(e) => {
            warn!(error = %e, contract_id = %contract_id, "Contract stats cache read failed; falling back to live data")
        }
    }

    let batch = fetch_batch_contract_stats(&state.db, std::slice::from_ref(&contract_id)).await?;
    let stats = batch
        .into_iter()
        .next()
        .ok_or_else(|| DashboardError::NotFound(format!("Contract {contract_id} not found")))?;

    if let Err(e) = try_cache_set(&state.redis, &cache_key, &stats).await {
        warn!(error = %e, contract_id = %stats.contract_id, "Failed to populate contract stats cache");
    }

    Ok(Json(stats))
}

/// `POST /api/v1/dashboard/contracts/stats/batch` — return batch contract usage statistics in a single request.
#[utoipa::path(
    post,
    path = "/api/v1/dashboard/contracts/stats/batch",
    request_body = BatchContractStatsRequest,
    responses(
        (status = 200, description = "Batch contract stats", body = BatchContractStatsResponse)
    ),
    tag = "dashboard"
)]
#[tracing::instrument(skip(state))]
pub async fn get_batch_contract_stats(
    State(state): State<Arc<DashboardState>>,
    Json(payload): Json<BatchContractStatsRequest>,
) -> Result<impl IntoResponse, DashboardError> {
    let stats = get_batch_contract_stats_inner(&state, payload.contract_ids).await?;
    Ok(Json(BatchContractStatsResponse { stats }))
}

/// `GET /api/v1/dashboard/contracts/stats` — return batch contract usage statistics via query string.
#[tracing::instrument(skip(state))]
pub async fn get_batch_contract_stats_query(
    State(state): State<Arc<DashboardState>>,
    axum::extract::Query(query): axum::extract::Query<BatchContractStatsQuery>,
) -> Result<impl IntoResponse, DashboardError> {
    let contract_ids = query
        .contract_ids
        .map(|s| {
            s.split(',')
                .map(|item| item.trim().to_string())
                .filter(|s| !s.is_empty())
                .collect::<Vec<String>>()
        })
        .unwrap_or_default();

    let stats = get_batch_contract_stats_inner(&state, contract_ids).await?;
    Ok(Json(BatchContractStatsResponse { stats }))
}

async fn get_batch_contract_stats_inner(
    state: &DashboardState,
    contract_ids: Vec<String>,
) -> Result<Vec<ContractStats>, DashboardError> {
    if contract_ids.is_empty() {
        return Ok(Vec::new());
    }

    let mut unique_ids = Vec::new();
    let mut seen = std::collections::HashSet::new();
    for id in contract_ids {
        if seen.insert(id.clone()) {
            unique_ids.push(id);
        }
    }

    let mut results: Vec<ContractStats> = Vec::with_capacity(unique_ids.len());
    let mut uncached_ids: Vec<String> = Vec::new();

    // Check Redis cache for each contract
    for id in &unique_ids {
        let cache_key = format!("dashboard:contract_stats:{id}");
        match try_cache_get::<ContractStats>(&state.redis, &cache_key).await {
            Ok(Some(cached)) => {
                results.push(cached);
            }
            Ok(None) => {
                uncached_ids.push(id.clone());
            }
            Err(e) => {
                warn!(error = %e, contract_id = %id, "Contract stats cache read failed; fetching live");
                uncached_ids.push(id.clone());
            }
        }
    }

    // Single consolidated SQL query for all cache misses
    if !uncached_ids.is_empty() {
        let fetched = fetch_batch_contract_stats(&state.db, &uncached_ids).await?;

        for stats in fetched {
            let cache_key = format!("dashboard:contract_stats:{}", stats.contract_id);
            if let Err(e) = try_cache_set(&state.redis, &cache_key, &stats).await {
                warn!(error = %e, contract_id = %stats.contract_id, "Failed to populate contract stats cache");
            }
            results.push(stats);
        }
    }

    // Deterministic sort order by contract_id
    results.sort_by(|a, b| a.contract_id.cmp(&b.contract_id));

    Ok(results)
}

// ---------------------------------------------------------------------------
// Cache helpers & Stampede Prevention
// ---------------------------------------------------------------------------

/// Attempt to acquire a distributed mutex lock in Redis with an auto-expiring lease.
pub async fn try_acquire_redis_lock(
    redis: &RedisClient,
    lock_key: &str,
    token: &str,
    ttl_ms: u64,
) -> Result<bool, DashboardError> {
    let mut conn = redis.get_multiplexed_async_connection().await?;
    let res: Option<String> = redis::cmd("SET")
        .arg(lock_key)
        .arg(token)
        .arg("NX")
        .arg("PX")
        .arg(ttl_ms)
        .query_async(&mut conn)
        .await?;
    Ok(res.as_deref() == Some("OK"))
}

/// Safely release a distributed lock in Redis only if the token matches.
pub async fn release_redis_lock(redis: &RedisClient, lock_key: &str, token: &str) {
    if let Ok(mut conn) = redis.get_multiplexed_async_connection().await {
        let script = r#"
            if redis.call("get", KEYS[1]) == ARGV[1] then
                return redis.call("del", KEYS[1])
            else
                return 0
            end
        "#;
        let _: Result<i32, _> = redis::cmd("EVAL")
            .arg(script)
            .arg(1)
            .arg(lock_key)
            .arg(token)
            .query_async(&mut conn)
            .await;
    }
}

/// Retrieve a cached item and its XFetch metadata, supporting both XFetchEnvelope and legacy entries.
pub async fn try_cache_get_xfetch<T>(
    redis: &RedisClient,
    key: &str,
) -> Result<Option<(T, XFetchItem)>, DashboardError>
where
    T: DeserializeOwned,
{
    let mut conn = redis.get_multiplexed_async_connection().await?;
    let raw: Option<String> = conn.get(key).await?;
    match raw {
        Some(s) => {
            if let Ok(env) = serde_json::from_str::<XFetchEnvelope<T>>(&s) {
                let item = XFetchItem::new(env.created_at, env.delta_secs, env.ttl_secs);
                return Ok(Some((env.value, item)));
            }
            let val: T = serde_json::from_str(&s)?;
            let item = XFetchItem::new(Utc::now().timestamp(), 0.1, CACHE_TTL_SECS);
            Ok(Some((val, item)))
        }
        None => Ok(None),
    }
}

/// Store a value in the cache wrapped in an XFetchEnvelope.
pub async fn try_cache_set_xfetch<T>(
    redis: &RedisClient,
    key: &str,
    data: &T,
    delta_secs: f64,
    ttl_secs: u64,
) -> Result<(), DashboardError>
where
    T: Serialize + Clone,
{
    let envelope = XFetchEnvelope {
        value: data.clone(),
        created_at: Utc::now().timestamp(),
        delta_secs,
        ttl_secs,
    };
    let serialized = serde_json::to_string(&envelope)?;
    let mut conn = redis.get_multiplexed_async_connection().await?;
    let _: () = conn.set_ex(key, serialized, ttl_secs).await?;
    Ok(())
}

async fn try_cache_get<T>(redis: &RedisClient, key: &str) -> Result<Option<T>, DashboardError>
where
    T: DeserializeOwned,
{
    let mut conn = redis.get_multiplexed_async_connection().await?;
    let raw: Option<String> = conn.get(key).await?;
    match raw {
        Some(s) => Ok(Some(serde_json::from_str(&s)?)),
        None => Ok(None),
    }
}

async fn try_cache_set<T>(redis: &RedisClient, key: &str, data: &T) -> Result<(), DashboardError>
where
    T: Serialize,
{
    let serialized = serde_json::to_string(data)?;
    let mut conn = redis.get_multiplexed_async_connection().await?;
    let _: () = conn.set_ex(key, serialized, CACHE_TTL_SECS).await?;
    Ok(())
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use axum::{body::Body, http::Request, routing::get, Router};
    use sqlx::postgres::PgPoolOptions;
    use tower::ServiceExt;

    fn make_state() -> Arc<DashboardState> {
        Arc::new(DashboardState {
            metrics_exporter: Arc::new(MetricsExporter::new()),
            error_manager: Arc::new(ErrorManager::new()),
            alert_manager: Arc::new(AlertManager::new()),
            db: PgPoolOptions::new()
                .max_connections(1)
                .connect_lazy("postgres://postgres:postgres@localhost/crucible_test")
                .unwrap(),
            // Use a URL that will fail to connect — the handler degrades gracefully.
            redis: RedisClient::open("redis://127.0.0.1:1/").unwrap(),
        })
    }

    fn make_app(state: Arc<DashboardState>) -> Router {
        Router::new()
            .route("/api/dashboard", get(get_dashboard))
            .with_state(state)
    }

    #[tokio::test]
    async fn test_dashboard_returns_200_without_redis() {
        let app = make_app(make_state());

        let response = app
            .oneshot(
                Request::builder()
                    .uri("/api/dashboard")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();

        assert_eq!(response.status(), axum::http::StatusCode::OK);
    }

    #[tokio::test]
    async fn test_dashboard_response_shape() {
        let app = make_app(make_state());

        let response = app
            .oneshot(
                Request::builder()
                    .uri("/api/dashboard")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();

        let bytes = axum::body::to_bytes(response.into_body(), usize::MAX)
            .await
            .unwrap();
        let json: serde_json::Value = serde_json::from_slice(&bytes).unwrap();

        assert!(json.get("metrics").is_some());
        assert!(json.get("active_recovery_tasks").is_some());
        assert!(json.get("active_alerts").is_some());
    }

    #[tokio::test]
    async fn test_dashboard_metrics_fields() {
        let state = make_state();
        state
            .metrics_exporter
            .update_metrics(42.0, 2048, 120, 4096, 2048)
            .await;

        let app = make_app(state);
        let response = app
            .oneshot(
                Request::builder()
                    .uri("/api/dashboard")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();

        let bytes = axum::body::to_bytes(response.into_body(), usize::MAX)
            .await
            .unwrap();
        let json: serde_json::Value = serde_json::from_slice(&bytes).unwrap();

        assert_eq!(json["metrics"]["cpu_usage"], 42.0);
        assert_eq!(json["metrics"]["memory_usage"], 2048);
        assert_eq!(json["metrics"]["uptime"], 120);
    }

    #[tokio::test]
    async fn test_dashboard_includes_recovery_tasks() {
        use crate::services::error_recovery::RecoveryError;

        let state = make_state();
        state
            .error_manager
            .handle_error(RecoveryError::Internal("boom".into()), "worker_a")
            .await
            .unwrap();

        let app = make_app(state);
        let response = app
            .oneshot(
                Request::builder()
                    .uri("/api/dashboard")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();

        let bytes = axum::body::to_bytes(response.into_body(), usize::MAX)
            .await
            .unwrap();
        let json: serde_json::Value = serde_json::from_slice(&bytes).unwrap();

        let tasks = json["active_recovery_tasks"].as_array().unwrap();
        assert_eq!(tasks.len(), 1);
        assert_eq!(tasks[0]["name"], "worker_a");
    }

    #[test]
    fn test_dashboard_data_serialization_roundtrip() {
        let data = DashboardData {
            metrics: SystemMetrics::default(),
            active_recovery_tasks: vec![],
            active_alerts: vec![],
        };
        let json = serde_json::to_string(&data).unwrap();
        let back: DashboardData = serde_json::from_str(&json).unwrap();
        assert_eq!(back.active_recovery_tasks.len(), 0);
        assert_eq!(back.active_alerts.len(), 0);
    }

    #[test]
    fn test_xfetch_item_expired_always_triggers() {
        let now = Utc::now().timestamp();
        // Item created 40 seconds ago with 30s TTL is expired
        let item = XFetchItem::new(now - 40, 0.5, 30);
        assert!(item.should_refresh_early(1.0));
    }

    #[test]
    fn test_xfetch_item_fresh_does_not_trigger_prematurely() {
        let now = Utc::now().timestamp();
        // Item created just now with 0.001s computation delta and 3600s TTL
        let item = XFetchItem::new(now, 0.001, 3600);
        for _ in 0..10 {
            assert!(!item.should_refresh_early(1.0));
        }
    }

    #[test]
    fn test_xfetch_envelope_roundtrip() {
        let metrics = DashboardMetrics {
            total_contracts: 10,
            total_transactions: 100,
            avg_processing_time_ms: 12.34,
            failed_transactions_24h: 1,
            timestamp: Utc::now(),
        };

        let envelope = XFetchEnvelope {
            value: metrics.clone(),
            created_at: Utc::now().timestamp(),
            delta_secs: 0.045,
            ttl_secs: 30,
        };

        let json = serde_json::to_string(&envelope).unwrap();
        let back: XFetchEnvelope<DashboardMetrics> = serde_json::from_str(&json).unwrap();
        assert_eq!(back.value.total_contracts, 10);
        assert_eq!(back.value.total_transactions, 100);
        assert_eq!(back.delta_secs, 0.045);
        assert_eq!(back.ttl_secs, 30);
    }

    #[tokio::test]
    async fn test_get_dashboard_metrics_endpoint_without_redis() {
        let state = make_state();
        let app = Router::new()
            .route("/api/v1/dashboard/metrics", get(get_dashboard_metrics))
            .with_state(state);

        let response = app
            .oneshot(
                Request::builder()
                    .uri("/api/v1/dashboard/metrics")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();

        assert!(
            response.status() == axum::http::StatusCode::OK
                || response.status() == axum::http::StatusCode::INTERNAL_SERVER_ERROR
        );
    }

    #[tokio::test]
    async fn test_batch_contract_stats_empty_request() {
        let state = make_state();
        let app = Router::new()
            .route(
                "/api/dashboard/contracts/stats/batch",
                axum::routing::post(get_batch_contract_stats),
            )
            .with_state(state);

        let req_body = serde_json::to_vec(&BatchContractStatsRequest {
            contract_ids: vec![],
        })
        .unwrap();

        let response = app
            .oneshot(
                Request::builder()
                    .method("POST")
                    .uri("/api/dashboard/contracts/stats/batch")
                    .header("content-type", "application/json")
                    .body(Body::from(req_body))
                    .unwrap(),
            )
            .await
            .unwrap();

        assert_eq!(response.status(), axum::http::StatusCode::OK);
        let bytes = axum::body::to_bytes(response.into_body(), usize::MAX)
            .await
            .unwrap();
        let resp: BatchContractStatsResponse = serde_json::from_slice(&bytes).unwrap();
        assert_eq!(resp.stats.len(), 0);
    }

    #[tokio::test]
    async fn test_batch_contract_stats_query_empty() {
        let state = make_state();
        let app = Router::new()
            .route(
                "/api/dashboard/contracts/stats",
                axum::routing::get(get_batch_contract_stats_query),
            )
            .with_state(state);

        let response = app
            .oneshot(
                Request::builder()
                    .method("GET")
                    .uri("/api/dashboard/contracts/stats")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();

        assert_eq!(response.status(), axum::http::StatusCode::OK);
        let bytes = axum::body::to_bytes(response.into_body(), usize::MAX)
            .await
            .unwrap();
        let resp: BatchContractStatsResponse = serde_json::from_slice(&bytes).unwrap();
        assert_eq!(resp.stats.len(), 0);
    }

    #[test]
    fn test_batch_contract_stats_roundtrip() {
        let req = BatchContractStatsRequest {
            contract_ids: vec!["c1".to_string(), "c2".to_string()],
        };
        let json = serde_json::to_string(&req).unwrap();
        let back: BatchContractStatsRequest = serde_json::from_str(&json).unwrap();
        assert_eq!(back.contract_ids, vec!["c1", "c2"]);

        let resp = BatchContractStatsResponse {
            stats: vec![ContractStats {
                contract_id: "c1".to_string(),
                invocation_count: 5,
                last_invoked: None,
                avg_gas_cost: 123.45,
            }],
        };
        let resp_json = serde_json::to_string(&resp).unwrap();
        let resp_back: BatchContractStatsResponse = serde_json::from_str(&resp_json).unwrap();
        assert_eq!(resp_back.stats.len(), 1);
        assert_eq!(resp_back.stats[0].contract_id, "c1");
        assert_eq!(resp_back.stats[0].invocation_count, 5);
    }
}
