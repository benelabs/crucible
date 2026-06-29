//! Feature flag service with Redis caching and PostgreSQL persistence.

#![allow(dead_code)]

use crate::services::tracing::TracingService;
use chrono::{DateTime, Utc};
use redis::{AsyncCommands, Client as RedisClient};
use serde::{Deserialize, Serialize};
use sqlx::PgPool;
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
}

impl FeatureFlagService {
    pub fn new(db: PgPool, redis: RedisClient) -> Self {
        Self { db, redis }
    }

    #[instrument(skip(self), fields(service.name = "FeatureFlagService", service.method = "is_enabled"))]
    pub async fn is_enabled(&self, key: &str) -> Result<bool, FlagError> {
        let cache_key = format!("flag:{key}");

        let redis_span = TracingService::redis_command_span("GET", Some(&cache_key));
        let _redis_enter = redis_span.enter();
        let mut conn = self.redis.get_multiplexed_async_connection().await?;
        let cached: Option<String> = conn.get(&cache_key).await?;
        drop(_redis_enter);

        if let Some(val) = cached {
            debug!(key = %key, "Feature flag cache hit");
            return Ok(val == "1");
        }

        debug!(key = %key, "Feature flag cache miss – querying database");
        
        let db_span = TracingService::db_query_span(
            "SELECT enabled FROM feature_flags WHERE key = $1",
            "postgres",
            "SELECT",
        );
        let _db_enter = db_span.enter();
        let row: Option<(bool,)> =
            sqlx::query_as("SELECT enabled FROM feature_flags WHERE key = $1")
                .bind(key)
                .fetch_optional(&self.db)
                .await?;
        drop(_db_enter);

        match row {
            Some((enabled,)) => {
                let val = if enabled { "1" } else { "0" };
                let _: () = conn.set_ex(&cache_key, val, 300).await?;
                debug!(key = %key, enabled = enabled, "Cached feature flag");
                Ok(enabled)
            }
            None => Err(FlagError::NotFound(key.to_string())),
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

        self.invalidate_cache(key).await?;
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

        self.invalidate_cache(key).await?;
        info!(key = %key, "Feature flag deleted");
        Ok(())
    }

    async fn invalidate_cache(&self, key: &str) -> Result<(), FlagError> {
        let cache_key = format!("flag:{key}");
        let mut conn = self.redis.get_multiplexed_async_connection().await?;
        let deleted: i32 = conn.del(&cache_key).await?;
        if deleted > 0 {
            debug!(key = %key, "Invalidated feature flag cache");
        } else {
            warn!(key = %key, "Cache key not found during invalidation");
        }
        Ok(())
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
}
