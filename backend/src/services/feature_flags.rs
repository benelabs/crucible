//! Feature flag service with Redis caching and PostgreSQL persistence.

#![allow(dead_code)]

use crate::services::tracing::TracingService;
use arc_swap::ArcSwap;
use chrono::{DateTime, Utc};
use redis::{AsyncCommands, Client as RedisClient};
use serde::{Deserialize, Serialize};
use sqlx::PgPool;
use std::collections::HashMap;
use std::sync::Arc;
use thiserror::Error;
use tracing::{debug, info, instrument, warn};

// ---------------------------------------------------------------------------
// Error type
// ---------------------------------------------------------------------------

#[derive(Debug, Error)]
pub enum FlagError {
    #[error("Database error: {0}")]
    Database(#[from] sqlx::Error),
    #[error("Redis error: {0}")]
    Redis(#[from] redis::RedisError),
    #[error("Feature flag not found: {0}")]
    NotFound(String),
    #[error("Internal error: {0}")]
    Internal(String),
}

// ---------------------------------------------------------------------------
// Domain types
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct FeatureFlag {
    pub key: String,
    pub enabled: bool,
    pub description: String,
    pub updated_at: DateTime<Utc>,
}

// ---------------------------------------------------------------------------
// FeatureFlagService
// ---------------------------------------------------------------------------

pub struct FeatureFlagService {
    db: PgPool,
    redis: RedisClient,
    local_cache: Arc<ArcSwap<HashMap<String, bool>>>,
    pub defaults: Arc<HashMap<String, bool>>,
}

impl FeatureFlagService {
    pub fn new(db: PgPool, redis: RedisClient) -> Self {
        Self {
            db,
            redis,
            local_cache: Arc::new(ArcSwap::new(Arc::new(HashMap::new()))),
            defaults: Arc::new(Self::default_flags()),
        }
    }

    pub fn with_defaults(db: PgPool, redis: RedisClient, defaults: HashMap<String, bool>) -> Self {
        Self {
            db,
            redis,
            local_cache: Arc::new(ArcSwap::new(Arc::new(HashMap::new()))),
            defaults: Arc::new(defaults),
        }
    }

    fn default_flags() -> HashMap<String, bool> {
        let mut m = HashMap::new();
        m.insert("dark_mode".to_string(), true);
        m.insert("experimental_features".to_string(), false);
        m.insert("telemetry".to_string(), true);
        m
    }

    pub fn update_local_cache(&self, key: &str, enabled: bool) {
        let current = self.local_cache.load();
        let mut new_map = (**current).clone();
        new_map.insert(key.to_string(), enabled);
        self.local_cache.store(Arc::new(new_map));
    }

    pub fn get_local_cache(&self, key: &str) -> Option<bool> {
        self.local_cache.load().get(key).copied()
    }

    pub fn remove_local_cache(&self, key: &str) {
        let current = self.local_cache.load();
        let mut new_map = (**current).clone();
        new_map.remove(key);
        self.local_cache.store(Arc::new(new_map));
    }

    #[instrument(skip(self), fields(service.name = "FeatureFlagService", service.method = "is_enabled"))]
    pub async fn is_enabled(&self, key: &str) -> Result<bool, FlagError> {
        let cache_key = format!("flag:{key}");

        // 1. Try Redis cache first
        let redis_result: Result<Option<String>, redis::RedisError> = match self
            .redis
            .get_multiplexed_async_connection()
            .await
        {
            Ok(mut conn) => {
                let redis_span = TracingService::redis_command_span("GET", Some(&cache_key));
                let _redis_enter = redis_span.enter();
                let res = conn.get(&cache_key).await;
                drop(_redis_enter);
                res
            }
            Err(e) => {
                warn!(error = %e, key = %key, "Redis connection failed; attempting local fallback");
                Err(e)
            }
        };

        match redis_result {
            Ok(Some(val)) => {
                debug!(key = %key, "Feature flag cache hit in Redis");
                let enabled = val == "1";
                self.update_local_cache(key, enabled);
                return Ok(enabled);
            }
            Ok(None) => {
                debug!(key = %key, "Feature flag cache miss in Redis");
            }
            Err(_) => {
                // Redis is down or errored: check local in-memory cache first
                if let Some(cached) = self.get_local_cache(key) {
                    debug!(key = %key, enabled = cached, "Redis unavailable; used local in-memory fallback");
                    return Ok(cached);
                }
            }
        }

        // 2. Query database
        debug!(key = %key, "Querying database for feature flag");
        let db_span = TracingService::db_query_span(
            "SELECT enabled FROM feature_flags WHERE key = $1",
            "postgres",
            "SELECT",
        );
        let _db_enter = db_span.enter();
        let db_res: Result<Option<(bool,)>, sqlx::Error> =
            sqlx::query_as("SELECT enabled FROM feature_flags WHERE key = $1")
                .bind(key)
                .fetch_optional(&self.db)
                .await;
        drop(_db_enter);

        match db_res {
            Ok(Some((enabled,))) => {
                // Best-effort cache into Redis
                if let Ok(mut conn) = self.redis.get_multiplexed_async_connection().await {
                    let val = if enabled { "1" } else { "0" };
                    let _: Result<(), _> = conn.set_ex(&cache_key, val, 300).await;
                }
                self.update_local_cache(key, enabled);
                debug!(key = %key, enabled = enabled, "Cached feature flag");
                Ok(enabled)
            }
            Ok(None) => {
                // If flag not in DB, fall back to default if configured
                if let Some(&def) = self.defaults.get(key) {
                    debug!(key = %key, default = def, "Flag not in DB; using configured default");
                    return Ok(def);
                }
                Err(FlagError::NotFound(key.to_string()))
            }
            Err(e) => {
                // Database query failed: fall back to local in-memory cache or default
                if let Some(cached) = self.get_local_cache(key) {
                    warn!(key = %key, error = %e, "DB query failed; using local in-memory flag");
                    return Ok(cached);
                }
                if let Some(&def) = self.defaults.get(key) {
                    warn!(key = %key, error = %e, "DB query failed; using local default flag");
                    return Ok(def);
                }
                Err(FlagError::Database(e))
            }
        }
    }

    #[instrument(skip(self), fields(service.name = "FeatureFlagService", service.method = "get"))]
    pub async fn get(&self, key: &str) -> Result<FeatureFlag, FlagError> {
        let row: Option<(String, bool, String, DateTime<Utc>)> = sqlx::query_as(
            "SELECT key, enabled, description, updated_at FROM feature_flags WHERE key = $1",
        )
        .bind(key)
        .fetch_optional(&self.db)
        .await?;

        match row {
            Some((key, enabled, description, updated_at)) => Ok(FeatureFlag {
                key,
                enabled,
                description,
                updated_at,
            }),
            None => Err(FlagError::NotFound(key.to_string())),
        }
    }

    #[instrument(skip(self), fields(service.name = "FeatureFlagService", service.method = "list"))]
    pub async fn list(&self) -> Result<Vec<FeatureFlag>, FlagError> {
        let rows: Vec<(String, bool, String, DateTime<Utc>)> = sqlx::query_as(
            "SELECT key, enabled, description, updated_at FROM feature_flags ORDER BY key",
        )
        .fetch_all(&self.db)
        .await?;

        Ok(rows
            .into_iter()
            .map(|(key, enabled, description, updated_at)| FeatureFlag {
                key,
                enabled,
                description,
                updated_at,
            })
            .collect())
    }

    #[instrument(skip(self), fields(service.name = "FeatureFlagService", service.method = "set"))]
    pub async fn set(&self, key: &str, enabled: bool, description: &str) -> Result<(), FlagError> {
        sqlx::query(
            r#"
            INSERT INTO feature_flags (key, enabled, description, updated_at)
            VALUES ($1, $2, $3, $4)
            ON CONFLICT (key) DO UPDATE
            SET enabled = EXCLUDED.enabled,
                description = EXCLUDED.description,
                updated_at = EXCLUDED.updated_at
            "#,
        )
        .bind(key)
        .bind(enabled)
        .bind(description)
        .bind(Utc::now())
        .execute(&self.db)
        .await?;

        self.update_local_cache(key, enabled);
        let _ = self.invalidate_cache(key).await;
        info!(key = %key, enabled = enabled, "Feature flag updated");
        Ok(())
    }

    #[instrument(skip(self), fields(service.name = "FeatureFlagService", service.method = "delete"))]
    pub async fn delete(&self, key: &str) -> Result<(), FlagError> {
        let result = sqlx::query("DELETE FROM feature_flags WHERE key = $1")
            .bind(key)
            .execute(&self.db)
            .await?;

        if result.rows_affected() == 0 {
            return Err(FlagError::NotFound(key.to_string()));
        }

        self.remove_local_cache(key);
        let _ = self.invalidate_cache(key).await;
        info!(key = %key, "Feature flag deleted");
        Ok(())
    }

    async fn invalidate_cache(&self, key: &str) -> Result<(), FlagError> {
        let cache_key = format!("flag:{key}");
        match self.redis.get_multiplexed_async_connection().await {
            Ok(mut conn) => {
                let deleted: i32 = conn.del(&cache_key).await.unwrap_or(0);
                if deleted > 0 {
                    debug!(key = %key, "Invalidated feature flag cache");
                } else {
                    debug!(key = %key, "Cache key not found during invalidation");
                }
                Ok(())
            }
            Err(e) => {
                warn!(key = %key, error = %e, "Failed to connect to Redis for cache invalidation; local cache updated");
                Ok(())
            }
        }
    }

    #[instrument(skip(self), fields(service.name = "FeatureFlagService", service.method = "flush_cache"))]
    pub async fn flush_cache(&self) -> Result<usize, FlagError> {
        let mut conn = self.redis.get_multiplexed_async_connection().await?;
        let keys: Vec<String> = redis::cmd("KEYS")
            .arg("flag:*")
            .query_async(&mut conn)
            .await?;

        if keys.is_empty() {
            return Ok(0);
        }

        let count = keys.len();
        for key in keys {
            let _: () = conn.del(&key).await?;
        }

        info!(count = count, "Flushed feature flag cache");
        Ok(count)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_flag_error_display() {
        let err = FlagError::NotFound("test_flag".to_string());
        assert!(err.to_string().contains("test_flag"));
    }

    #[test]
    fn test_feature_flag_serialization() {
        let flag = FeatureFlag {
            key: "test".to_string(),
            enabled: true,
            description: "Test flag".to_string(),
            updated_at: Utc::now(),
        };
        let json = serde_json::to_string(&flag).unwrap();
        assert!(json.contains("test"));
        assert!(json.contains("true"));
    }

    #[test]
    fn test_feature_flag_deserialization() {
        let json = r#"{
            "key": "beta",
            "enabled": false,
            "description": "Beta features",
            "updated_at": "2024-01-01T00:00:00Z"
        }"#;
        let flag: FeatureFlag = serde_json::from_str(json).unwrap();
        assert_eq!(flag.key, "beta");
        assert!(!flag.enabled);
    }

    #[test]
    fn test_local_cache_atomic_swap_and_defaults() {
        let defaults = FeatureFlagService::default_flags();
        assert_eq!(defaults.get("dark_mode"), Some(&true));
        assert_eq!(defaults.get("experimental_features"), Some(&false));

        let local_cache = Arc::new(ArcSwap::new(Arc::new(HashMap::new())));
        assert_eq!(local_cache.load().get("dark_mode"), None);

        // Store
        let current = local_cache.load();
        let mut new_map = (**current).clone();
        new_map.insert("dark_mode".to_string(), false);
        local_cache.store(Arc::new(new_map));
        assert_eq!(local_cache.load().get("dark_mode"), Some(&false));

        // Remove
        let current = local_cache.load();
        let mut new_map = (**current).clone();
        new_map.remove("dark_mode");
        local_cache.store(Arc::new(new_map));
        assert_eq!(local_cache.load().get("dark_mode"), None);
    }
}
