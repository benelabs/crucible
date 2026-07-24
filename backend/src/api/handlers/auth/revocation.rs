//! Redis-backed JWT Token Revocation and Blocklist Service.

use redis::AsyncCommands;
use serde::{Deserialize, Serialize};
use std::sync::Arc;
use utoipa::ToSchema;
use crate::error::AppError;

/// Request payload to revoke a JWT token.
#[derive(Debug, Serialize, Deserialize, ToSchema)]
pub struct RevokeTokenRequest {
    pub jti: String,
    pub expires_in_seconds: u64,
    pub reason: Option<String>,
}

/// Revocation record stored in Redis token blocklist.
#[derive(Debug, Serialize, Deserialize, ToSchema)]
pub struct RevokedTokenEntry {
    pub jti: String,
    pub revoked_at: chrono::DateTime<chrono::Utc>,
    pub reason: String,
}

/// Token Blocklist Service enforcing immediate JWT revocation via Redis storage.
#[derive(Clone)]
pub struct TokenBlocklistService {
    redis: Arc<redis::Client>,
}

impl TokenBlocklistService {
    pub fn new(redis: Arc<redis::Client>) -> Self {
        Self { redis }
    }

    /// Adds a JWT token identifier (`jti`) to the Redis blocklist with TTL matching token expiration.
    pub async fn revoke_token(&self, req: RevokeTokenRequest) -> Result<(), AppError> {
        let entry = RevokedTokenEntry {
            jti: req.jti.clone(),
            revoked_at: chrono::Utc::now(),
            reason: req.reason.unwrap_or_else(|| "User logout or administrative revocation".to_string()),
        };

        let key = format!("token_blocklist:{}", req.jti);
        let value = serde_json::to_string(&entry).map_err(AppError::Serialization)?;

        let mut conn = self.redis.get_multiplexed_async_connection().await.map_err(AppError::Redis)?;

        let ttl = if req.expires_in_seconds == 0 { 86400 } else { req.expires_in_seconds };
        let _: () = conn.set_ex(key, value, ttl).await.map_err(AppError::Redis)?;

        Ok(())
    }

    /// Checks if a JWT token identifier (`jti`) is present in the revocation blocklist.
    pub async fn is_token_revoked(&self, jti: &str) -> Result<bool, AppError> {
        let key = format!("token_blocklist:{jti}");
        let mut conn = self.redis.get_multiplexed_async_connection().await.map_err(AppError::Redis)?;

        let exists: bool = conn.exists(key).await.map_err(AppError::Redis)?;
        Ok(exists)
    }
}
