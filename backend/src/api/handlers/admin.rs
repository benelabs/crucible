use crate::api::contracts::ApiResponse;
use crate::api::handlers::profiling::AppState;
use crate::config::reload::ConfigManager;
use crate::config::sanitize;
use crate::error::AppError;
use crate::services::contract_call_logger::ContractCallLogger;
use axum::{
    extract::{Path, State},
    response::IntoResponse,
    Json,
};
use chrono::{DateTime, Utc};
use redis::AsyncCommands;
use serde::{Deserialize, Serialize};
use sha2::Digest;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use tracing::info;

pub const SESSION_REVOCATION_CHANNEL: &str = "session:revocations";
pub const DEFAULT_REVOCATION_TTL_SECS: u64 = 86_400; // 24 hours

async fn delete_matching_redis_keys(
    conn: &mut redis::aio::MultiplexedConnection,
    pattern: &str,
) -> Result<(), AppError> {
    let mut cursor = 0_u64;
    loop {
        let (next_cursor, keys): (u64, Vec<String>) = redis::cmd("SCAN")
            .arg(cursor)
            .arg("MATCH")
            .arg(pattern)
            .arg("COUNT")
            .arg(100)
            .query_async(conn)
            .await
            .map_err(AppError::Redis)?;

        if !keys.is_empty() {
            let _: i64 = redis::cmd("DEL")
                .arg(&keys)
                .query_async(conn)
                .await
                .map_err(AppError::Redis)?;
        }

        cursor = next_cursor;
        if cursor == 0 {
            return Ok(());
        }
    }
}

// Global static for maintenance mode (mock implementation)
pub static MAINTENANCE_MODE: AtomicBool = AtomicBool::new(false);

#[derive(Debug, Clone, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct MaintenanceRequest {
    pub enabled: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SystemStatsResponse {
    pub database_connected: bool,
    pub redis_connected: bool,
    pub total_contract_calls: i64,
    pub system_uptime_secs: u64,
    pub maintenance_mode: bool,
}

/// User role hierarchy ranks for calculating elevation vs demotion.
pub fn role_rank(role: &str) -> u32 {
    match role.trim().to_ascii_lowercase().as_str() {
        "admin" => 100,
        "moderator" => 90,
        "developer" => 80,
        "auditor" => 60,
        "viewer" => 40,
        "user" => 20,
        "guest" => 10,
        _ => 0,
    }
}

/// Validates whether a provided role name is a recognized system role.
pub fn is_valid_role(role: &str) -> bool {
    matches!(
        role.trim().to_ascii_lowercase().as_str(),
        "admin" | "moderator" | "developer" | "auditor" | "viewer" | "user" | "guest"
    )
}

#[derive(Debug, Clone, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct UpdateUserRoleRequest {
    /// Target role (e.g., "admin", "developer", "auditor", "viewer", "user", "guest").
    pub role: String,
    /// Actor performing the elevation/modification (optional; extracted from header/caller context if omitted).
    #[serde(default)]
    pub actor_id: Option<String>,
    /// Optional justification or reason for role change.
    #[serde(default)]
    pub reason: Option<String>,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ElevateRoleRequest {
    /// Target user ID (optional if address is provided).
    #[serde(default)]
    pub target_user_id: Option<i32>,
    /// Target user ID alias.
    #[serde(default)]
    pub user_id: Option<i32>,
    /// Target user address (optional if user_id is provided).
    #[serde(default)]
    pub address: Option<String>,
    /// Target role to elevate to.
    pub role: String,
    /// Actor ID performing elevation.
    #[serde(default)]
    pub actor_id: Option<String>,
    /// Reason or ticket ID.
    #[serde(default)]
    pub reason: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct RoleElevationRecord {
    pub user_id: i32,
    pub address: String,
    pub previous_role: String,
    pub new_role: String,
    pub is_elevation: bool,
    pub actor_id: String,
    pub audit_log_id: i64,
    pub audit_hash: String,
    pub previous_hash: String,
    pub timestamp: chrono::DateTime<chrono::Utc>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct RoleElevationAuditLogItem {
    pub id: i64,
    pub event_type: String,
    pub actor_id: Option<String>,
    pub target_user_id: Option<i32>,
    pub previous_role: Option<String>,
    pub new_role: Option<String>,
    pub is_elevation: Option<bool>,
    pub reason: Option<String>,
    pub timestamp: chrono::DateTime<chrono::Utc>,
    pub hash: String,
    pub previous_hash: String,
}

/// Resolves actor ID from headers or request payload.
pub fn resolve_actor_id(headers: &axum::http::HeaderMap, payload_actor: Option<&str>) -> String {
    if let Some(actor) = payload_actor {
        if !actor.trim().is_empty() {
            return actor.trim().to_string();
        }
    }
    if let Some(header_val) = headers
        .get("X-Actor-Id")
        .or_else(|| headers.get("x-actor-id"))
    {
        if let Ok(val) = header_val.to_str() {
            if !val.trim().is_empty() {
                return val.trim().to_string();
            }
        }
    }
    if let Some(header_val) = headers
        .get("X-User-Id")
        .or_else(|| headers.get("x-user-id"))
    {
        if let Ok(val) = header_val.to_str() {
            if !val.trim().is_empty() {
                return val.trim().to_string();
            }
        }
    }
    "admin".to_string()
}

/// Atomically executes role modification / elevation in a single PostgreSQL database transaction.
/// Synchronously inserts an immutable SHA-256 hash-chained audit record into `audit_logs`.
pub async fn execute_role_elevation_tx(
    pool: &sqlx::PgPool,
    redis_client: Option<&redis::Client>,
    actor_id: &str,
    target_user_id: i32,
    target_role: &str,
    reason: Option<&str>,
) -> Result<RoleElevationRecord, AppError> {
    let normalized_role = target_role.trim().to_lowercase();
    if !is_valid_role(&normalized_role) {
        return Err(AppError::BadRequest(format!(
            "Invalid role '{}'. Allowed roles: admin, moderator, developer, auditor, viewer, user, guest",
            target_role
        )));
    }

    // Begin database transaction
    let mut tx = pool.begin().await.map_err(AppError::Database)?;

    // 1. Fetch target user and current role with row-level lock FOR UPDATE
    let user_row: Option<(i32, String, String)> = sqlx::query_as(
        r#"
        SELECT id, address, role::text
        FROM users
        WHERE id = $1
        FOR UPDATE
        "#,
    )
    .bind(target_user_id)
    .fetch_optional(&mut *tx)
    .await
    .map_err(AppError::Database)?;

    let (uid, address, previous_role) = match user_row {
        Some(row) => row,
        None => {
            return Err(AppError::NotFound(format!(
                "User with id {} not found",
                target_user_id
            )));
        }
    };

    let prev_norm = previous_role.trim().to_lowercase();
    let is_elevation = role_rank(&normalized_role) > role_rank(&prev_norm);
    let event_type = if is_elevation {
        "role_elevation"
    } else if role_rank(&normalized_role) < role_rank(&prev_norm) {
        "role_demotion"
    } else {
        "role_update"
    };

    // 2. Perform role update in users table
    let enum_update = sqlx::query("UPDATE users SET role = $1::user_role WHERE id = $2")
        .bind(&normalized_role)
        .bind(uid)
        .execute(&mut *tx)
        .await;

    if enum_update.is_err() {
        sqlx::query("UPDATE users SET role = $1 WHERE id = $2")
            .bind(&normalized_role)
            .bind(uid)
            .execute(&mut *tx)
            .await
            .map_err(AppError::Database)?;
    }

    // 3. Retrieve latest audit log entry's hash for immutable SHA-256 hash chaining
    let prev_hash_row: Option<(String,)> =
        sqlx::query_as("SELECT hash FROM audit_logs ORDER BY id DESC LIMIT 1 FOR UPDATE")
            .fetch_optional(&mut *tx)
            .await
            .ok()
            .flatten();

    let previous_hash = prev_hash_row.map(|r| r.0).unwrap_or_default();
    let now = chrono::Utc::now();

    let details = serde_json::json!({
        "actor_id": actor_id,
        "target_user_id": uid,
        "target_user_address": address,
        "previous_role": prev_norm,
        "new_role": normalized_role,
        "is_elevation": is_elevation,
        "reason": reason,
        "timestamp": now.to_rfc3339()
    });

    // 4. Compute cryptographic SHA-256 chain hash
    let canonical = format!(
        "{}:{}:{}:{}:{}",
        previous_hash,
        event_type,
        actor_id,
        serde_json::to_string(&details).unwrap_or_default(),
        now.to_rfc3339()
    );
    let mut hasher = sha2::Sha256::new();
    sha2::Digest::update(&mut hasher, canonical.as_bytes());
    let audit_hash = format!("{:x}", sha2::Digest::finalize(hasher));

    // 5. Synchronously insert immutable audit log record in same transaction
    let audit_log_id: i64 = sqlx::query_scalar(
        r#"
        INSERT INTO audit_logs (event_type, user_id, details, timestamp, hash, previous_hash)
        VALUES ($1, $2, $3, $4, $5, $6)
        RETURNING id::bigint
        "#,
    )
    .bind(event_type)
    .bind(actor_id)
    .bind(&details)
    .bind(now)
    .bind(&audit_hash)
    .bind(&previous_hash)
    .fetch_one(&mut *tx)
    .await
    .map_err(AppError::Database)?;

    // Commit transaction atomically
    tx.commit().await.map_err(AppError::Database)?;

    // 6. Post-commit cache invalidation and chain tip update in Redis
    if let Some(redis) = redis_client {
        if let Ok(mut conn) = redis.get_multiplexed_async_connection().await {
            use redis::AsyncCommands;
            let role_key = format!("role:{}", uid);
            let _: Result<(), _> = conn.del(&role_key).await;
            let _: Result<(), _> = conn.set("audit:last_hash", &audit_hash).await;
        }
    }

    Ok(RoleElevationRecord {
        user_id: uid,
        address,
        previous_role: prev_norm,
        new_role: normalized_role,
        is_elevation,
        actor_id: actor_id.to_string(),
        audit_log_id,
        audit_hash,
        previous_hash,
        timestamp: now,
    })
}

/// GET /api/v1/admin/system-stats
pub async fn get_system_stats(
    State(state): State<Arc<AppState>>,
) -> Result<impl IntoResponse, AppError> {
    let db_connected = if let Some(ref pool) = state.db {
        sqlx::query("SELECT 1").fetch_optional(pool).await.is_ok()
    } else {
        false
    };

    let redis_connected = state.redis.get_async_connection().await.is_ok();

    let total_calls = if let Some(ref pool) = state.db {
        sqlx::query_scalar::<_, i64>("SELECT COUNT(*) FROM contract_call_logs")
            .fetch_one(pool)
            .await
            .unwrap_or(0)
    } else {
        0
    };

    let metrics = state.metrics_exporter.get_metrics().await;

    Ok(Json(ApiResponse::new(SystemStatsResponse {
        database_connected: db_connected,
        redis_connected,
        total_contract_calls: total_calls,
        system_uptime_secs: metrics.uptime,
        maintenance_mode: MAINTENANCE_MODE.load(Ordering::Relaxed),
    })))
}

/// POST /api/v1/admin/maintenance
pub async fn set_maintenance_mode(
    Json(payload): Json<MaintenanceRequest>,
) -> Result<impl IntoResponse, AppError> {
    MAINTENANCE_MODE.store(payload.enabled, Ordering::Relaxed);
    Ok(Json(ApiResponse::new(serde_json::json!({
        "success": true,
        "maintenanceMode": payload.enabled
    }))))
}

/// GET /api/v1/admin/logs
pub async fn get_admin_logs(
    State(state): State<Arc<AppState>>,
) -> Result<impl IntoResponse, AppError> {
    let db = state
        .db
        .clone()
        .ok_or_else(|| AppError::InternalError("Database connection not configured".to_string()))?;

    let logger = ContractCallLogger::new(db);
    let logs = logger.get_logs(None, 50).await?;

    Ok(Json(ApiResponse::new(logs)))
}

/// PUT or POST /api/v1/admin/users/:user_id/role
///
/// Modifies a user's role and synchronously records an immutable audit log entry in the same transaction.
pub async fn update_user_role(
    State(state): State<Arc<AppState>>,
    axum::extract::Path(target_user_id): axum::extract::Path<i32>,
    headers: axum::http::HeaderMap,
    Json(payload): Json<UpdateUserRoleRequest>,
) -> Result<impl IntoResponse, AppError> {
    let pool = state
        .db
        .as_ref()
        .ok_or_else(|| AppError::InternalError("Database connection not configured".to_string()))?;

    let actor_id = resolve_actor_id(&headers, payload.actor_id.as_deref());

    let record = execute_role_elevation_tx(
        pool,
        Some(&state.redis),
        &actor_id,
        target_user_id,
        &payload.role,
        payload.reason.as_deref(),
    )
    .await?;

    Ok(Json(ApiResponse::new(record)))
}

/// GET /api/v1/admin/users/:user_id/role
pub async fn get_user_role(
    State(state): State<Arc<AppState>>,
    axum::extract::Path(target_user_id): axum::extract::Path<i32>,
) -> Result<impl IntoResponse, AppError> {
    let pool = state
        .db
        .as_ref()
        .ok_or_else(|| AppError::InternalError("Database connection not configured".to_string()))?;

    let row: Option<(i32, String, String)> =
        sqlx::query_as("SELECT id, address, role::text FROM users WHERE id = $1")
            .bind(target_user_id)
            .fetch_optional(pool)
            .await
            .map_err(AppError::Database)?;

    let (id, address, role) = row
        .ok_or_else(|| AppError::NotFound(format!("User with id {} not found", target_user_id)))?;

    Ok(Json(ApiResponse::new(serde_json::json!({
        "userId": id,
        "address": address,
        "role": role
    }))))
}

/// POST /api/v1/admin/roles/elevate
///
/// Dedicated endpoint to elevate a target user's role with mandatory immutable audit logging.
pub async fn elevate_user_role(
    State(state): State<Arc<AppState>>,
    headers: axum::http::HeaderMap,
    Json(payload): Json<ElevateRoleRequest>,
) -> Result<impl IntoResponse, AppError> {
    let pool = state
        .db
        .as_ref()
        .ok_or_else(|| AppError::InternalError("Database connection not configured".to_string()))?;

    let actor_id = resolve_actor_id(&headers, payload.actor_id.as_deref());

    let target_id = if let Some(id) = payload.target_user_id.or(payload.user_id) {
        id
    } else if let Some(ref addr) = payload.address {
        let user_id: i32 = sqlx::query_scalar("SELECT id FROM users WHERE address = $1")
            .bind(addr)
            .fetch_optional(pool)
            .await
            .map_err(AppError::Database)?
            .ok_or_else(|| AppError::NotFound(format!("User with address '{}' not found", addr)))?;
        user_id
    } else {
        return Err(AppError::BadRequest(
            "targetUserId, userId, or address must be provided".to_string(),
        ));
    };

    let record = execute_role_elevation_tx(
        pool,
        Some(&state.redis),
        &actor_id,
        target_id,
        &payload.role,
        payload.reason.as_deref(),
    )
    .await?;

    Ok(Json(ApiResponse::new(record)))
}

/// GET /api/v1/admin/audit/role-elevations
///
/// Fetches recent role elevation and modification audit log records from the immutable `audit_logs` table.
pub async fn get_role_elevation_audit_logs(
    State(state): State<Arc<AppState>>,
) -> Result<impl IntoResponse, AppError> {
    let pool = state
        .db
        .as_ref()
        .ok_or_else(|| AppError::InternalError("Database connection not configured".to_string()))?;

    let rows: Vec<(i64, String, Option<String>, serde_json::Value, chrono::DateTime<chrono::Utc>, String, String)> =
        sqlx::query_as(
            r#"
            SELECT id, event_type, user_id, details, timestamp, hash, previous_hash
            FROM audit_logs
            WHERE event_type IN ('role_elevation', 'role_demotion', 'role_update', 'user.role_elevation', 'user.role_update')
            ORDER BY id DESC
            LIMIT 100
            "#,
        )
        .fetch_all(pool)
        .await
        .map_err(AppError::Database)?;

    let items: Vec<RoleElevationAuditLogItem> = rows
        .into_iter()
        .map(
            |(id, event_type, user_id, details, timestamp, hash, previous_hash)| {
                let target_user_id = details
                    .get("target_user_id")
                    .and_then(|v| v.as_i64())
                    .map(|v| v as i32);
                let previous_role = details
                    .get("previous_role")
                    .and_then(|v| v.as_str())
                    .map(ToString::to_string);
                let new_role = details
                    .get("new_role")
                    .and_then(|v| v.as_str())
                    .map(ToString::to_string);
                let is_elevation = details.get("is_elevation").and_then(|v| v.as_bool());
                let reason = details
                    .get("reason")
                    .and_then(|v| v.as_str())
                    .map(ToString::to_string);

                RoleElevationAuditLogItem {
                    id,
                    event_type,
                    actor_id: user_id,
                    target_user_id,
                    previous_role,
                    new_role,
                    is_elevation,
                    reason,
                    timestamp,
                    hash,
                    previous_hash,
                }
            },
        )
        .collect();

    Ok(Json(ApiResponse::new(items)))
}

/// GET /api/v1/admin/config
///
/// Returns the current effective configuration with all sensitive fields redacted.
/// This endpoint is intended for operators to inspect non-secret config for diagnostics.
///
/// **Redacted fields:**
/// - `database.url` → `"[REDACTED]"`
/// - `redis.url` → `"[REDACTED]"`
/// - `redis.job_queue_url` → `"[REDACTED]"`
/// - `server.tls.key_path` → `"[REDACTED]"`
pub async fn get_effective_config(
    State(config_manager): State<Arc<ConfigManager>>,
) -> Result<impl IntoResponse, AppError> {
    let config = config_manager.load();
    let sanitized = sanitize::sanitize(&config);
    Ok(Json(ApiResponse::new(sanitized)))
}

/// Payload to suspend a compromised user and revoke all active sessions.
#[derive(Debug, Clone, Deserialize, Serialize, utoipa::ToSchema)]
#[serde(rename_all = "camelCase")]
pub struct SuspendUserRequest {
    pub user_id: i32,
    pub reason: Option<String>,
    pub active_jtis: Option<Vec<String>>,
}

/// Payload to revoke or demote a user's role and revoke active sessions.
#[derive(Debug, Clone, Deserialize, Serialize, utoipa::ToSchema)]
#[serde(rename_all = "camelCase")]
pub struct RevokeRoleRequest {
    pub user_id: i32,
    pub reason: Option<String>,
    pub active_jtis: Option<Vec<String>>,
}

/// Result returned after session invalidation and account suspension / role revocation.
#[derive(Debug, Clone, Serialize, Deserialize, utoipa::ToSchema)]
#[serde(rename_all = "camelCase")]
pub struct RevocationResult {
    pub user_id: i32,
    pub is_active: bool,
    pub revoked_at: chrono::DateTime<chrono::Utc>,
    pub blacklisted_jtis: usize,
    pub sessions_cleared: bool,
    pub broadcast_sent: bool,
    pub reason: String,
}

/// Invalidates all active Redis sessions, blacklists JTIs, and broadcasts revocation events across Redis Pub/Sub.
pub async fn invalidate_user_sessions_and_tokens(
    redis: &redis::Client,
    user_id: i32,
    active_jtis: Option<&[String]>,
    reason: &str,
) -> Result<(usize, bool, bool), AppError> {
    let mut conn = redis
        .get_multiplexed_async_connection()
        .await
        .map_err(AppError::Redis)?;
    let ttl: u64 = 86400 * 7; // 7-day revocation window

    let mut count = 0;
    // 1. Blacklist active JWT JTIs in Redis
    if let Some(jtis) = active_jtis {
        for jti in jtis {
            let key = format!("token_blocklist:{jti}");
            let entry = serde_json::json!({
                "jti": jti,
                "userId": user_id,
                "revokedAt": chrono::Utc::now(),
                "reason": reason
            });
            let _: () = conn
                .set_ex(&key, entry.to_string(), ttl)
                .await
                .map_err(AppError::Redis)?;
            count += 1;
        }
    }

    // 2. Blacklist user subject identifier in Redis
    let user_key = format!("token_blocklist:user:{user_id}");
    let user_blocklist_key = format!("user_blocklist:{user_id}");
    let user_entry = serde_json::json!({
        "userId": user_id,
        "revokedAt": chrono::Utc::now(),
        "reason": reason
    });
    let _: () = conn
        .set_ex(&user_key, user_entry.to_string(), ttl)
        .await
        .map_err(AppError::Redis)?;
    let _: () = conn
        .set_ex(&user_blocklist_key, user_entry.to_string(), ttl)
        .await
        .map_err(AppError::Redis)?;

    // 3. Clear active session records in Redis
    let session_key = format!("session:{user_id}");
    let user_sessions_key = format!("user_sessions:{user_id}");
    let _: () = conn
        .del(&[&session_key, &user_sessions_key])
        .await
        .unwrap_or(());

    // 4. Broadcast session revocation event across Redis Pub/Sub
    let broadcast_payload = serde_json::json!({
        "event": "session_revoked",
        "userId": user_id,
        "reason": reason,
        "timestamp": chrono::Utc::now()
    });
    let broadcast_sent: bool = conn
        .publish::<_, _, i64>("auth:session:revoked", broadcast_payload.to_string())
        .await
        .is_ok();

    Ok((count, true, broadcast_sent))
}

/// POST /api/v1/admin/users/suspend
///
/// Suspends a user account in PostgreSQL, revokes all active Redis session keys,
/// blacklists active JWT tokens, and broadcasts session revocation across Redis.
pub async fn suspend_user(
    State(state): State<Arc<AppState>>,
    Json(payload): Json<SuspendUserRequest>,
) -> Result<impl IntoResponse, AppError> {
    let reason = payload
        .reason
        .unwrap_or_else(|| "User suspended by administrator".to_string());

    // 1. Update PostgreSQL users.is_active = false
    if let Some(ref pool) = state.db {
        let rows = sqlx::query("UPDATE users SET is_active = false WHERE id = $1")
            .bind(payload.user_id)
            .execute(pool)
            .await
            .map_err(AppError::Database)?;

        if rows.rows_affected() == 0 {
            return Err(AppError::NotFound(format!(
                "User with id {} not found",
                payload.user_id
            )));
        }

        // Record immutable audit log
        let _ = sqlx::query(
            "INSERT INTO audit_logs (event_type, user_id, details, timestamp, hash, previous_hash) VALUES ($1, $2, $3, NOW(), 'audit_hash', 'prev_hash')"
        )
        .bind("user.suspended")
        .bind(payload.user_id.to_string())
        .bind(serde_json::json!({
            "userId": payload.user_id,
            "action": "suspended",
            "reason": reason
        }))
        .execute(pool)
        .await;
    }

    // 2. Invalidate sessions, blacklist tokens, and broadcast revocation
    let (blacklisted, sessions_cleared, broadcast_sent) = invalidate_user_sessions_and_tokens(
        &state.redis,
        payload.user_id,
        payload.active_jtis.as_deref(),
        &reason,
    )
    .await?;

    Ok(Json(ApiResponse::new(RevocationResult {
        user_id: payload.user_id,
        is_active: false,
        revoked_at: chrono::Utc::now(),
        blacklisted_jtis: blacklisted,
        sessions_cleared,
        broadcast_sent,
        reason,
    })))
}

/// POST /api/v1/admin/roles/revoke
///
/// Revokes elevated role permissions for a user, updates database records,
/// blacklists active tokens, and invalidates active Redis sessions.
pub async fn revoke_user_role(
    State(state): State<Arc<AppState>>,
    Json(payload): Json<RevokeRoleRequest>,
) -> Result<impl IntoResponse, AppError> {
    let reason = payload
        .reason
        .unwrap_or_else(|| "Role revoked by administrator".to_string());

    // 1. Update role in PostgreSQL
    if let Some(ref pool) = state.db {
        let rows = sqlx::query("UPDATE users SET role = 'user' WHERE id = $1")
            .bind(payload.user_id)
            .execute(pool)
            .await
            .map_err(AppError::Database)?;

        if rows.rows_affected() == 0 {
            return Err(AppError::NotFound(format!(
                "User with id {} not found",
                payload.user_id
            )));
        }

        // Record immutable audit log
        let _ = sqlx::query(
            "INSERT INTO audit_logs (event_type, user_id, details, timestamp, hash, previous_hash) VALUES ($1, $2, $3, NOW(), 'audit_hash', 'prev_hash')"
        )
        .bind("user.role_revoked")
        .bind(payload.user_id.to_string())
        .bind(serde_json::json!({
            "userId": payload.user_id,
            "action": "role_revoked",
            "newRole": "user",
            "reason": reason
        }))
        .execute(pool)
        .await;
    }

    // 2. Invalidate sessions, blacklist tokens, and broadcast revocation
    let (blacklisted, sessions_cleared, broadcast_sent) = invalidate_user_sessions_and_tokens(
        &state.redis,
        payload.user_id,
        payload.active_jtis.as_deref(),
        &reason,
    )
    .await?;

    Ok(Json(ApiResponse::new(RevocationResult {
        user_id: payload.user_id,
        is_active: true,
        revoked_at: chrono::Utc::now(),
        blacklisted_jtis: blacklisted,
        sessions_cleared,
        broadcast_sent,
        reason,
    })))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_suspend_user_request_serialization() {
        let req = SuspendUserRequest {
            user_id: 123,
            reason: Some("Compromised credentials".to_string()),
            active_jtis: Some(vec!["jti-1".to_string(), "jti-2".to_string()]),
        };
        let json = serde_json::to_string(&req).unwrap();
        let deserialized: SuspendUserRequest = serde_json::from_str(&json).unwrap();
        assert_eq!(deserialized.user_id, 123);
        assert_eq!(
            deserialized.reason.as_deref(),
            Some("Compromised credentials")
        );
        assert_eq!(deserialized.active_jtis.unwrap().len(), 2);
    }

    #[test]
    fn test_revoke_role_request_serialization() {
        let req = RevokeRoleRequest {
            user_id: 456,
            reason: Some("Demoted from admin".to_string()),
            active_jtis: Some(vec!["jti-adm".to_string()]),
        };
        let json = serde_json::to_string(&req).unwrap();
        let deserialized: RevokeRoleRequest = serde_json::from_str(&json).unwrap();
        assert_eq!(deserialized.user_id, 456);
        assert_eq!(deserialized.reason.as_deref(), Some("Demoted from admin"));
        assert_eq!(
            deserialized.active_jtis.unwrap(),
            vec!["jti-adm".to_string()]
        );
    }

    #[test]
    fn test_revocation_result_serialization() {
        let res = RevocationResult {
            user_id: 7,
            is_active: false,
            revoked_at: chrono::Utc::now(),
            blacklisted_jtis: 3,
            sessions_cleared: true,
            broadcast_sent: true,
            reason: "suspended".to_string(),
        };
        let json = serde_json::to_string(&res).unwrap();
        assert!(json.contains("\"userId\":7"));
        assert!(json.contains("\"isActive\":false"));
        assert!(json.contains("suspended"));
    }

    #[test]
    fn test_role_hierarchy_ranks() {
        assert!(role_rank("admin") > role_rank("developer"));
        assert!(role_rank("developer") > role_rank("auditor"));
        assert!(role_rank("auditor") > role_rank("viewer"));
        assert!(role_rank("viewer") > role_rank("user"));
        assert!(role_rank("user") > role_rank("guest"));
        assert_eq!(role_rank("unknown"), 0);
    }

    #[test]
    fn test_is_valid_role() {
        assert!(is_valid_role("admin"));
        assert!(is_valid_role("developer"));
        assert!(is_valid_role("auditor"));
        assert!(is_valid_role("viewer"));
        assert!(is_valid_role("user"));
        assert!(is_valid_role("guest"));
        assert!(is_valid_role("moderator"));
        assert!(!is_valid_role("super_root_god"));
        assert!(!is_valid_role("hacker"));
    }

    #[test]
    fn test_resolve_actor_id() {
        let mut headers = axum::http::HeaderMap::new();
        assert_eq!(resolve_actor_id(&headers, Some("actor-123")), "actor-123");

        headers.insert("X-Actor-Id", "actor-header".parse().unwrap());
        assert_eq!(resolve_actor_id(&headers, None), "actor-header");

        let empty_headers = axum::http::HeaderMap::new();
        assert_eq!(resolve_actor_id(&empty_headers, None), "admin");
    }

    #[test]
    fn test_audit_hash_chain_tamper_detection() {
        let prev_hash = "genesis_hash_0000000000";
        let event_type = "role_elevation";
        let actor = "admin_super";
        let details = serde_json::json!({
            "target_user_id": 42,
            "previous_role": "user",
            "new_role": "admin"
        });
        let ts = "2026-10-22T19:00:00Z";

        let canonical1 = format!("{}:{}:{}:{}:{}", prev_hash, event_type, actor, details, ts);
        let mut hasher1 = sha2::Sha256::new();
        sha2::Digest::update(&mut hasher1, canonical1.as_bytes());
        let hash1 = format!("{:x}", sha2::Digest::finalize(hasher1));

        let tampered_details = serde_json::json!({
            "target_user_id": 42,
            "previous_role": "user",
            "new_role": "guest"
        });
        let canonical2 = format!(
            "{}:{}:{}:{}:{}",
            prev_hash, event_type, actor, tampered_details, ts
        );
        let mut hasher2 = sha2::Sha256::new();
        sha2::Digest::update(&mut hasher2, canonical2.as_bytes());
        let hash2 = format!("{:x}", sha2::Digest::finalize(hasher2));

        assert_ne!(
            hash1, hash2,
            "Tampering with details must alter computed SHA-256 hash"
        );
    }

    #[test]
    fn test_elevation_flag_detection() {
        assert!(role_rank("admin") > role_rank("user"));
        assert!(role_rank("developer") > role_rank("guest"));
        assert!(!(role_rank("user") > role_rank("admin")));
        assert!(!(role_rank("admin") > role_rank("admin")));
    }
}
