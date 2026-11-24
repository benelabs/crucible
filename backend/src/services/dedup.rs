//! Build request deduplication service.
//!
//! The service protects expensive build submission paths from repeated work by
//! deriving a stable SHA-256 fingerprint from the request identity, acquiring a
//! short-lived Redis lock with `SET NX EX`, and optionally persisting the
//! deduplication record in PostgreSQL.

use axum::{
    extract::State,
    http::StatusCode,
    response::{IntoResponse, Response},
    routing::post,
    Json, Router,
};
use chrono::{DateTime, Utc};
use redis::AsyncCommands;
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use sha2::{Digest, Sha256};
use sqlx::PgPool;
use std::sync::Arc;
use thiserror::Error;
use tracing::{debug, info, instrument, warn};
use uuid::Uuid;

/// Default Redis namespace for build request deduplication keys.
pub const DEFAULT_NAMESPACE: &str = "build:dedup";

/// Configuration for deduplication key retention and queueing.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DedupConfig {
    /// Redis key namespace.
    pub namespace: String,
    /// Completed duplicate response TTL in seconds.
    pub result_ttl_seconds: usize,
    /// In-flight lock TTL in seconds (default base lock TTL).
    pub lock_ttl_seconds: usize,
    /// Additional lock TTL in seconds per crate dependency.
    pub ttl_per_dependency_seconds: usize,
    /// Maximum in-flight lock TTL in seconds.
    pub max_lock_ttl_seconds: usize,
    /// Default heartbeat renewal interval in seconds.
    pub heartbeat_interval_seconds: u64,
    /// Lock extension duration in seconds on heartbeat renewal.
    pub heartbeat_extension_seconds: usize,
    /// Redis list used to enqueue accepted build requests.
    pub queue_name: String,
}

impl DedupConfig {
    /// Computes dynamic lock TTL proportional to contract crate dependencies.
    ///
    /// TTL = clamp(lock_ttl_seconds + (dependency_count * ttl_per_dependency_seconds), lock_ttl_seconds, max_lock_ttl_seconds)
    pub fn compute_lock_ttl(&self, crate_dependency_count: usize) -> usize {
        let additional = crate_dependency_count.saturating_mul(self.ttl_per_dependency_seconds);
        let dynamic_ttl = self.lock_ttl_seconds.saturating_add(additional);
        let max_ttl = self.max_lock_ttl_seconds.max(self.lock_ttl_seconds);
        dynamic_ttl.clamp(self.lock_ttl_seconds, max_ttl)
    }
}

impl Default for DedupConfig {
    fn default() -> Self {
        Self {
            namespace: DEFAULT_NAMESPACE.to_string(),
            result_ttl_seconds: 86_400,
            lock_ttl_seconds: 900,
            ttl_per_dependency_seconds: 5,
            max_lock_ttl_seconds: 3_600,
            heartbeat_interval_seconds: 10,
            heartbeat_extension_seconds: 30,
            queue_name: "build:requests".to_string(),
        }
    }
}

/// A build request payload eligible for deduplication.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct BuildRequest {
    /// HTTP method or logical command name.
    pub method: String,
    /// HTTP path or logical build target.
    pub path: String,
    /// Optional caller identity.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub requester_id: Option<String>,
    /// Optional client supplied idempotency key.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub idempotency_key: Option<String>,
    /// Request body.
    #[serde(default)]
    pub body: Value,
}

impl BuildRequest {
    /// Creates a request value from route parts and JSON body.
    pub fn new(method: impl Into<String>, path: impl Into<String>, body: Value) -> Self {
        Self {
            method: method.into(),
            path: path.into(),
            requester_id: None,
            idempotency_key: None,
            body,
        }
    }

    /// Sets the requester identity.
    pub fn requester_id(mut self, requester_id: impl Into<String>) -> Self {
        self.requester_id = Some(requester_id.into());
        self
    }

    /// Sets the client supplied idempotency key.
    pub fn idempotency_key(mut self, idempotency_key: impl Into<String>) -> Self {
        self.idempotency_key = Some(idempotency_key.into());
        self
    }

    /// Extracts or computes crate dependency count from the request body if present.
    /// Inspects `dependency_count` (integer), `dependencies` (array or object count),
    /// or `crate_dependencies` (array or integer).
    pub fn crate_dependency_count(&self) -> usize {
        if let Some(count) = self.body.get("dependency_count").and_then(|v| v.as_u64()) {
            return count as usize;
        }
        if let Some(deps) = self.body.get("dependencies") {
            if let Some(arr) = deps.as_array() {
                return arr.len();
            }
            if let Some(obj) = deps.as_object() {
                return obj.len();
            }
        }
        if let Some(deps) = self.body.get("crate_dependencies") {
            if let Some(arr) = deps.as_array() {
                return arr.len();
            }
            if let Some(count) = deps.as_u64() {
                return count as usize;
            }
        }
        0
    }

    fn validate(&self) -> Result<(), DedupError> {
        if self.method.trim().is_empty() {
            return Err(DedupError::InvalidRequest("method is required".to_string()));
        }
        if self.path.trim().is_empty() {
            return Err(DedupError::InvalidRequest("path is required".to_string()));
        }
        Ok(())
    }
}

/// Deduplication state for a submitted build request.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum DedupStatus {
    /// The request is new and was accepted for build processing.
    Accepted,
    /// The same request is already being processed.
    InProgress,
    /// The same request was already accepted or completed.
    Duplicate,
}

/// Deduplication decision returned to callers.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct DedupDecision {
    /// Stable request fingerprint.
    pub fingerprint: String,
    /// Server generated request id for the accepted or existing request.
    pub request_id: Uuid,
    /// Decision status.
    pub status: DedupStatus,
    /// Whether a cached result is available.
    pub replayable: bool,
}

impl DedupDecision {
    fn accepted(fingerprint: String, request_id: Uuid) -> Self {
        Self {
            fingerprint,
            request_id,
            status: DedupStatus::Accepted,
            replayable: false,
        }
    }

    fn duplicate(fingerprint: String, request_id: Uuid, replayable: bool) -> Self {
        Self {
            fingerprint,
            request_id,
            status: DedupStatus::Duplicate,
            replayable,
        }
    }

    fn in_progress(fingerprint: String, request_id: Uuid) -> Self {
        Self {
            fingerprint,
            request_id,
            status: DedupStatus::InProgress,
            replayable: false,
        }
    }
}

/// Errors produced by the build request deduplication service.
#[derive(Debug, Error)]
pub enum DedupError {
    /// The request did not contain enough data to be fingerprinted safely.
    #[error("invalid request: {0}")]
    InvalidRequest(String),
    /// JSON serialization failed.
    #[error("serialization error: {0}")]
    Serialization(#[from] serde_json::Error),
    /// Redis operation failed.
    #[error("redis error: {0}")]
    Redis(#[from] redis::RedisError),
    /// PostgreSQL operation failed.
    #[error("database error: {0}")]
    Database(#[from] sqlx::Error),
}

impl IntoResponse for DedupError {
    fn into_response(self) -> Response {
        let status = match self {
            Self::InvalidRequest(_) => StatusCode::BAD_REQUEST,
            Self::Serialization(_) | Self::Redis(_) | Self::Database(_) => {
                StatusCode::INTERNAL_SERVER_ERROR
            }
        };

        (
            status,
            Json(json!({
                "error": self.to_string(),
                "code": status.as_u16()
            })),
        )
            .into_response()
    }
}

/// Production build request deduplicator.
#[derive(Clone)]
pub struct BuildDedupService {
    db: PgPool,
    redis: redis::Client,
    config: DedupConfig,
}

impl BuildDedupService {
    /// Creates a deduplication service.
    pub fn new(db: PgPool, redis: redis::Client) -> Self {
        Self::with_config(db, redis, DedupConfig::default())
    }

    /// Creates a deduplication service with explicit configuration.
    pub fn with_config(db: PgPool, redis: redis::Client, config: DedupConfig) -> Self {
        Self { db, redis, config }
    }

    /// Creates the persistence table required by this service.
    ///
    /// This mirrors `backend/migrations/20260528010000_build_request_dedup.sql`
    /// for isolated integration tests. Production deployments should apply the
    /// migration through the normal SQLx migration path.
    pub async fn ensure_schema(pool: &PgPool) -> Result<(), DedupError> {
        sqlx::query(
            r#"
            CREATE TABLE IF NOT EXISTS build_request_dedup (
                request_id UUID PRIMARY KEY,
                fingerprint TEXT NOT NULL UNIQUE,
                status TEXT NOT NULL,
                request_payload JSONB NOT NULL,
                response_payload JSONB,
                duplicate_count BIGINT NOT NULL DEFAULT 0,
                created_at TIMESTAMPTZ NOT NULL DEFAULT NOW(),
                updated_at TIMESTAMPTZ NOT NULL DEFAULT NOW(),
                expires_at TIMESTAMPTZ NOT NULL,
                CONSTRAINT build_request_dedup_status_check
                    CHECK (status IN ('accepted', 'completed', 'failed'))
            )
            "#,
        )
        .execute(pool)
        .await?;

        sqlx::query(
            r#"
            CREATE INDEX IF NOT EXISTS idx_build_request_dedup_expires_at
                ON build_request_dedup (expires_at)
            "#,
        )
        .execute(pool)
        .await?;

        sqlx::query(
            r#"
            CREATE INDEX IF NOT EXISTS idx_build_request_dedup_status
                ON build_request_dedup (status)
            "#,
        )
        .execute(pool)
        .await?;

        Ok(())
    }

    /// Computes the stable request fingerprint.
    pub fn fingerprint(request: &BuildRequest) -> Result<String, DedupError> {
        request.validate()?;

        let canonical = serde_json::to_vec(&json!({
            "method": request.method.trim().to_ascii_uppercase(),
            "path": request.path.trim(),
            "requester_id": &request.requester_id,
            "idempotency_key": &request.idempotency_key,
            "body": &request.body,
        }))?;

        let mut hasher = Sha256::new();
        hasher.update((canonical.len() as u64).to_be_bytes());
        hasher.update(canonical);
        Ok(to_hex(&hasher.finalize()))
    }

    /// Checks and registers a build request.
    ///
    /// Time complexity is O(n) for JSON serialization plus O(1) Redis and
    /// indexed PostgreSQL operations. Space complexity is O(n), where n is the
    /// serialized request size.
    /// Checks and registers a build request using dependency-proportional lock TTL.
    ///
    /// Time complexity is O(n) for JSON serialization plus O(1) Redis and
    /// indexed PostgreSQL operations. Space complexity is O(n), where n is the
    /// serialized request size.
    #[instrument(skip(self, request), fields(path = %request.path, method = %request.method))]
    pub async fn check_or_register(
        &self,
        request: BuildRequest,
    ) -> Result<DedupDecision, DedupError> {
        let lock_ttl = self
            .config
            .compute_lock_ttl(request.crate_dependency_count());
        self.check_or_register_with_ttl(request, lock_ttl).await
    }

    /// Checks and registers a build request with an explicit lock TTL.
    #[instrument(skip(self, request), fields(path = %request.path, method = %request.method))]
    pub async fn check_or_register_with_ttl(
        &self,
        request: BuildRequest,
        lock_ttl_seconds: usize,
    ) -> Result<DedupDecision, DedupError> {
        let fingerprint = Self::fingerprint(&request)?;
        let request_id = Uuid::new_v4();
        let lock_key = self.lock_key(&fingerprint);
        let result_key = self.result_key(&fingerprint);
        let payload = serde_json::to_string(&QueuedBuildRequest {
            request_id,
            fingerprint: fingerprint.clone(),
            request: request.clone(),
            queued_at: Utc::now(),
        })?;

        let mut conn = self.redis.get_multiplexed_async_connection().await?;

        if let Some(existing) = conn.get::<_, Option<String>>(&result_key).await? {
            if let Ok(existing_id) = Uuid::parse_str(&existing) {
                debug!(%fingerprint, %existing_id, "dedup cache hit");
                return Ok(DedupDecision::duplicate(fingerprint, existing_id, true));
            }
        }

        let lock_acquired: Option<String> = redis::cmd("SET")
            .arg(&lock_key)
            .arg(request_id.to_string())
            .arg("NX")
            .arg("EX")
            .arg(lock_ttl_seconds)
            .query_async(&mut conn)
            .await?;

        if lock_acquired.is_none() {
            let existing_id = conn
                .get::<_, Option<String>>(&lock_key)
                .await?
                .and_then(|id| Uuid::parse_str(&id).ok())
                .unwrap_or(request_id);
            warn!(%fingerprint, %existing_id, "duplicate build request is already in progress");
            return Ok(DedupDecision::in_progress(fingerprint, existing_id));
        }

        let persisted = self
            .persist_request(&fingerprint, request_id, &request)
            .await?;
        if persisted.request_id != request_id {
            self.release_lock(&lock_key, request_id).await?;
            return Ok(DedupDecision::duplicate(
                fingerprint,
                persisted.request_id,
                persisted.status == "completed",
            ));
        }

        conn.rpush::<_, _, usize>(&self.config.queue_name, payload)
            .await?;
        info!(%fingerprint, %request_id, "accepted unique build request");

        Ok(DedupDecision::accepted(fingerprint, request_id))
    }

    /// Extends the in-flight deduplication lock TTL in Redis if held by the specified request_id.
    ///
    /// Uses an atomic Lua script to verify lock ownership before calling EXPIRE.
    /// Returns Ok(true) if the lock was renewed, Ok(false) if the lock was lost or expired.
    #[instrument(skip(self))]
    pub async fn extend_lock(
        &self,
        fingerprint: &str,
        request_id: Uuid,
        extension_secs: usize,
    ) -> Result<bool, DedupError> {
        let lock_key = self.lock_key(fingerprint);
        let mut conn = self.redis.get_multiplexed_async_connection().await?;
        let result: i32 = redis::Script::new(
            r#"
            if redis.call("GET", KEYS[1]) == ARGV[1] then
                return redis.call("EXPIRE", KEYS[1], ARGV[2])
            end
            return 0
            "#,
        )
        .key(lock_key)
        .arg(request_id.to_string())
        .arg(extension_secs)
        .invoke_async(&mut conn)
        .await?;

        Ok(result == 1)
    }

    /// Starts a background heartbeat renewal task that periodically extends the deduplication lock.
    /// Returns a `DedupHeartbeatGuard` which cancels the heartbeat when dropped or explicitly stopped.
    pub fn start_heartbeat(
        self: &Arc<Self>,
        fingerprint: impl Into<String>,
        request_id: Uuid,
    ) -> DedupHeartbeatGuard {
        let interval = std::time::Duration::from_secs(self.config.heartbeat_interval_seconds);
        let extension = self.config.heartbeat_extension_seconds;
        self.start_heartbeat_with_params(fingerprint, request_id, interval, extension)
    }

    /// Starts a background heartbeat renewal task with custom interval and extension durations.
    pub fn start_heartbeat_with_params(
        self: &Arc<Self>,
        fingerprint: impl Into<String>,
        request_id: Uuid,
        interval: std::time::Duration,
        extension_secs: usize,
    ) -> DedupHeartbeatGuard {
        let (stop_tx, mut stop_rx) = tokio::sync::watch::channel(false);
        let service = Arc::clone(self);
        let fp = fingerprint.into();

        let handle = tokio::spawn(async move {
            let mut ticker = tokio::time::interval(interval);
            ticker.tick().await;

            loop {
                tokio::select! {
                    _ = stop_rx.changed() => {
                        if *stop_rx.borrow() {
                            break;
                        }
                    }
                    _ = ticker.tick() => {
                        match service.extend_lock(&fp, request_id, extension_secs).await {
                            Ok(true) => {
                                debug!(fingerprint = %fp, %request_id, "extended dedup lock heartbeat");
                            }
                            Ok(false) => {
                                warn!(fingerprint = %fp, %request_id, "failed to extend dedup lock heartbeat: lock lost or expired");
                                break;
                            }
                            Err(e) => {
                                warn!(fingerprint = %fp, %request_id, error = %e, "error extending dedup lock heartbeat");
                            }
                        }
                    }
                }
            }
        });

        DedupHeartbeatGuard {
            stop_tx: Some(stop_tx),
            handle: Some(handle),
        }
    }

    /// Marks a request completed and stores the replay id in Redis.
    #[instrument(skip(self, response))]
    pub async fn complete(
        &self,
        fingerprint: &str,
        request_id: Uuid,
        response: Value,
    ) -> Result<(), DedupError> {
        sqlx::query(
            r#"
            UPDATE build_request_dedup
            SET status = 'completed',
                response_payload = $3,
                updated_at = NOW()
            WHERE fingerprint = $1 AND request_id = $2
            "#,
        )
        .bind(fingerprint)
        .bind(request_id)
        .bind(response)
        .execute(&self.db)
        .await?;

        let mut conn = self.redis.get_multiplexed_async_connection().await?;
        conn.set_ex::<_, _, ()>(
            self.result_key(fingerprint),
            request_id.to_string(),
            self.config.result_ttl_seconds as u64,
        )
        .await?;
        self.release_lock(&self.lock_key(fingerprint), request_id)
            .await?;
        Ok(())
    }

    /// Marks a request failed and releases the in-flight lock.
    #[instrument(skip(self))]
    pub async fn fail(&self, fingerprint: &str, request_id: Uuid) -> Result<(), DedupError> {
        sqlx::query(
            r#"
            UPDATE build_request_dedup
            SET status = 'failed', updated_at = NOW()
            WHERE fingerprint = $1 AND request_id = $2
            "#,
        )
        .bind(fingerprint)
        .bind(request_id)
        .execute(&self.db)
        .await?;

        self.release_lock(&self.lock_key(fingerprint), request_id)
            .await?;
        Ok(())
    }

    fn lock_key(&self, fingerprint: &str) -> String {
        format!("{}:{}:lock", self.config.namespace, fingerprint)
    }

    fn result_key(&self, fingerprint: &str) -> String {
        format!("{}:{}:result", self.config.namespace, fingerprint)
    }

    async fn persist_request(
        &self,
        fingerprint: &str,
        request_id: Uuid,
        request: &BuildRequest,
    ) -> Result<PersistedDedupRecord, DedupError> {
        let expires_at =
            Utc::now() + chrono::Duration::seconds(self.config.result_ttl_seconds as i64);
        let payload = serde_json::to_value(request)?;

        let row = sqlx::query_as::<_, PersistedDedupRecord>(
            r#"
            INSERT INTO build_request_dedup (
                request_id,
                fingerprint,
                status,
                request_payload,
                expires_at
            )
            VALUES ($1, $2, 'accepted', $3, $4)
            ON CONFLICT (fingerprint) DO UPDATE
            SET duplicate_count = build_request_dedup.duplicate_count + 1,
                updated_at = NOW()
            RETURNING request_id, status
            "#,
        )
        .bind(request_id)
        .bind(fingerprint)
        .bind(payload)
        .bind(expires_at)
        .fetch_one(&self.db)
        .await?;

        Ok(row)
    }

    async fn release_lock(&self, lock_key: &str, request_id: Uuid) -> Result<(), DedupError> {
        let mut conn = self.redis.get_multiplexed_async_connection().await?;
        redis::Script::new(
            r#"
            if redis.call("GET", KEYS[1]) == ARGV[1] then
                return redis.call("DEL", KEYS[1])
            end
            return 0
            "#,
        )
        .key(lock_key)
        .arg(request_id.to_string())
        .invoke_async::<i32>(&mut conn)
        .await?;
        Ok(())
    }
}

/// RAII guard that manages an active deduplication lock heartbeat renewal task.
///
/// When dropped or stopped, the background renewal task terminates.
pub struct DedupHeartbeatGuard {
    stop_tx: Option<tokio::sync::watch::Sender<bool>>,
    handle: Option<tokio::task::JoinHandle<()>>,
}

impl DedupHeartbeatGuard {
    /// Manually stops the heartbeat renewal task.
    pub fn stop(&mut self) {
        if let Some(tx) = self.stop_tx.take() {
            let _ = tx.send(true);
        }
        if let Some(handle) = self.handle.take() {
            handle.abort();
        }
    }

    /// Checks whether the heartbeat is currently active.
    pub fn is_active(&self) -> bool {
        self.handle.as_ref().is_some_and(|h| !h.is_finished())
    }
}

impl Drop for DedupHeartbeatGuard {
    fn drop(&mut self) {
        self.stop();
    }
}

/// Shared Axum state for deduplicated build request routes.
#[derive(Clone)]
pub struct DedupState {
    /// Deduplication service.
    pub service: Arc<BuildDedupService>,
}

/// Builds an Axum router for deduplicated build submissions.
pub fn router(state: DedupState) -> Router {
    Router::new()
        .route("/build-requests", post(submit_build_request))
        .with_state(state)
}

/// Axum handler for a deduplicated build request submission.
pub async fn submit_build_request(
    State(state): State<DedupState>,
    Json(request): Json<BuildRequest>,
) -> Result<Json<DedupDecision>, DedupError> {
    state.service.check_or_register(request).await.map(Json)
}

#[derive(Debug, Serialize)]
struct QueuedBuildRequest {
    request_id: Uuid,
    fingerprint: String,
    request: BuildRequest,
    queued_at: DateTime<Utc>,
}

#[derive(Debug, sqlx::FromRow)]
struct PersistedDedupRecord {
    request_id: Uuid,
    status: String,
}

fn to_hex(bytes: &[u8]) -> String {
    const HEX: &[u8; 16] = b"0123456789abcdef";
    let mut out = String::with_capacity(bytes.len() * 2);
    for byte in bytes {
        out.push(HEX[(byte >> 4) as usize] as char);
        out.push(HEX[(byte & 0x0f) as usize] as char);
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use sqlx::postgres::PgPoolOptions;

    fn sample_request() -> BuildRequest {
        BuildRequest::new(
            "post",
            "/api/build",
            json!({
                "repo": "crucible",
                "target": "backend",
                "commit": "abc123"
            }),
        )
        .requester_id("user-1")
    }

    #[test]
    fn fingerprint_is_stable_for_same_request() {
        let first = BuildDedupService::fingerprint(&sample_request()).unwrap();
        let second = BuildDedupService::fingerprint(&sample_request()).unwrap();

        assert_eq!(first, second);
        assert_eq!(first.len(), 64);
    }

    #[test]
    fn fingerprint_changes_when_body_changes() {
        let first = BuildDedupService::fingerprint(&sample_request()).unwrap();
        let changed = BuildRequest::new(
            "post",
            "/api/build",
            json!({
                "repo": "crucible",
                "target": "contracts",
                "commit": "abc123"
            }),
        )
        .requester_id("user-1");

        let second = BuildDedupService::fingerprint(&changed).unwrap();
        assert_ne!(first, second);
    }

    #[test]
    fn fingerprint_normalizes_method_and_path_whitespace() {
        let first = BuildDedupService::fingerprint(&sample_request()).unwrap();
        let second = BuildDedupService::fingerprint(&BuildRequest {
            method: " POST ".to_string(),
            path: " /api/build ".to_string(),
            ..sample_request()
        })
        .unwrap();

        assert_eq!(first, second);
    }

    #[test]
    fn empty_method_is_rejected() {
        let err = BuildDedupService::fingerprint(&BuildRequest::new("", "/api/build", json!({})))
            .unwrap_err();

        assert!(matches!(err, DedupError::InvalidRequest(_)));
    }

    #[test]
    fn dedup_config_defaults_are_bounded() {
        let config = DedupConfig::default();

        assert_eq!(config.namespace, DEFAULT_NAMESPACE);
        assert!(config.lock_ttl_seconds > 0);
        assert!(config.lock_ttl_seconds < config.result_ttl_seconds);
        assert!(!config.queue_name.is_empty());
    }

    #[test]
    fn compute_lock_ttl_proportional_to_dependencies() {
        let config = DedupConfig {
            lock_ttl_seconds: 30,
            ttl_per_dependency_seconds: 5,
            max_lock_ttl_seconds: 120,
            ..DedupConfig::default()
        };

        // 0 dependencies -> base TTL 30s
        assert_eq!(config.compute_lock_ttl(0), 30);
        // 5 dependencies -> 30 + 5*5 = 55s
        assert_eq!(config.compute_lock_ttl(5), 55);
        // 10 dependencies -> 30 + 10*5 = 80s
        assert_eq!(config.compute_lock_ttl(10), 80);
        // 100 dependencies -> capped at max 120s
        assert_eq!(config.compute_lock_ttl(100), 120);
    }

    #[test]
    fn crate_dependency_count_extraction() {
        let req_empty = BuildRequest::new("POST", "/build", json!({}));
        assert_eq!(req_empty.crate_dependency_count(), 0);

        let req_array = BuildRequest::new(
            "POST",
            "/build",
            json!({
                "dependencies": ["syn", "quote", "proc-macro2", "serde"]
            }),
        );
        assert_eq!(req_array.crate_dependency_count(), 4);

        let req_obj = BuildRequest::new(
            "POST",
            "/build",
            json!({
                "dependencies": { "syn": "2.0", "serde": "1.0" }
            }),
        );
        assert_eq!(req_obj.crate_dependency_count(), 2);

        let req_num = BuildRequest::new(
            "POST",
            "/build",
            json!({
                "dependency_count": 15
            }),
        );
        assert_eq!(req_num.crate_dependency_count(), 15);

        let req_crate_deps = BuildRequest::new(
            "POST",
            "/build",
            json!({
                "crate_dependencies": ["dep1", "dep2", "dep3"]
            }),
        );
        assert_eq!(req_crate_deps.crate_dependency_count(), 3);
    }

    #[tokio::test]
    async fn heartbeat_guard_lifecycle() {
        let (stop_tx, _) = tokio::sync::watch::channel(false);
        let handle = tokio::spawn(async {
            tokio::time::sleep(std::time::Duration::from_secs(10)).await;
        });

        let mut guard = DedupHeartbeatGuard {
            stop_tx: Some(stop_tx),
            handle: Some(handle),
        };

        assert!(guard.is_active());
        guard.stop();
        assert!(!guard.is_active());
    }

    #[tokio::test]
    async fn handler_router_can_be_constructed() {
        let db = PgPoolOptions::new()
            .connect_lazy("postgres://postgres:postgres@localhost/crucible_test")
            .unwrap();
        let redis = redis::Client::open("redis://127.0.0.1/").unwrap();
        let state = DedupState {
            service: Arc::new(BuildDedupService::new(db, redis)),
        };

        let _router = router(state);
    }

    #[tokio::test]
    async fn check_or_register_accepts_then_deduplicates_request() {
        let database_url = std::env::var("DATABASE_URL")
            .unwrap_or_else(|_| "postgres://postgres:postgres@localhost/crucible_test".to_string());
        let redis_url =
            std::env::var("REDIS_URL").unwrap_or_else(|_| "redis://127.0.0.1/".to_string());
        let db = PgPoolOptions::new()
            .max_connections(1)
            .connect(&database_url)
            .await
            .unwrap();
        BuildDedupService::ensure_schema(&db).await.unwrap();

        let redis = redis::Client::open(redis_url).unwrap();
        let config = DedupConfig {
            namespace: format!("{}:test:{}", DEFAULT_NAMESPACE, Uuid::new_v4()),
            result_ttl_seconds: 60,
            lock_ttl_seconds: 30,
            queue_name: format!("build:requests:test:{}", Uuid::new_v4()),
            ..DedupConfig::default()
        };
        let service = BuildDedupService::with_config(db, redis, config);
        let request = sample_request().idempotency_key("ci-build-1");

        let accepted = service.check_or_register(request.clone()).await.unwrap();
        let duplicate = service.check_or_register(request).await.unwrap();

        assert_eq!(accepted.status, DedupStatus::Accepted);
        assert_eq!(duplicate.status, DedupStatus::InProgress);
        assert_eq!(accepted.request_id, duplicate.request_id);
    }

    #[tokio::test]
    async fn complete_marks_request_replayable_for_duplicates() {
        let database_url = std::env::var("DATABASE_URL")
            .unwrap_or_else(|_| "postgres://postgres:postgres@localhost/crucible_test".to_string());
        let redis_url =
            std::env::var("REDIS_URL").unwrap_or_else(|_| "redis://127.0.0.1/".to_string());
        let db = PgPoolOptions::new()
            .max_connections(1)
            .connect(&database_url)
            .await
            .unwrap();
        BuildDedupService::ensure_schema(&db).await.unwrap();

        let redis = redis::Client::open(redis_url).unwrap();
        let config = DedupConfig {
            namespace: format!("{}:test:{}", DEFAULT_NAMESPACE, Uuid::new_v4()),
            result_ttl_seconds: 60,
            lock_ttl_seconds: 30,
            queue_name: format!("build:requests:test:{}", Uuid::new_v4()),
            ..DedupConfig::default()
        };
        let service = BuildDedupService::with_config(db, redis, config);
        let request = sample_request().idempotency_key("ci-build-2");

        let accepted = service.check_or_register(request.clone()).await.unwrap();
        service
            .complete(
                &accepted.fingerprint,
                accepted.request_id,
                json!({ "build_id": "build-1" }),
            )
            .await
            .unwrap();
        let duplicate = service.check_or_register(request).await.unwrap();

        assert_eq!(duplicate.status, DedupStatus::Duplicate);
        assert_eq!(duplicate.request_id, accepted.request_id);
        assert!(duplicate.replayable);
    }
}
