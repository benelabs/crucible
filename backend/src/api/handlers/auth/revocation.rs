//! Redis-backed JWT Token Revocation and Blocklist Service.

use redis::AsyncCommands;
use serde::{Deserialize, Serialize};
use std::sync::Arc;
use utoipa::ToSchema;
use crate::error::AppError;
use super::jwt::JwtKeyManager;
use jsonwebtoken::{decode, decode_header, Algorithm, DecodingKey, Validation};

/// Request payload to revoke a JWT token.
#[derive(Debug, Serialize, Deserialize, ToSchema)]
pub struct RevokeTokenRequest {
    pub token: String,
    pub reason: Option<String>,
}

#[derive(Deserialize)]
struct TokenClaims {
    jti: String,
    exp: u64,
}

fn verified_token_claims(
    token: &str,
    key_manager: &JwtKeyManager,
) -> Result<(String, u64), AppError> {
    let invalid_token = || AppError::Unauthorized("Invalid or expired JWT".to_string());
    let header = decode_header(token).map_err(|_| invalid_token())?;
    if header.alg != Algorithm::RS256 {
        return Err(invalid_token());
    }

    let kid = header.kid.ok_or_else(invalid_token)?;
    let key_pair = key_manager.get_key(&kid).ok_or_else(invalid_token)?;
    let decoding_key = DecodingKey::from_rsa_pem(key_pair.public_key_pem.as_bytes())
        .map_err(|_| invalid_token())?;
    let mut validation = Validation::new(Algorithm::RS256);
    validation.leeway = 0;

    let claims = decode::<TokenClaims>(token, &decoding_key, &validation)
        .map_err(|_| invalid_token())?
        .claims;
    if claims.jti.is_empty() {
        return Err(invalid_token());
    }

    let now = u64::try_from(chrono::Utc::now().timestamp()).map_err(|_| invalid_token())?;
    let ttl = claims.exp.checked_sub(now).filter(|ttl| *ttl > 0).ok_or_else(invalid_token)?;

    Ok((claims.jti, ttl))
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
    pub async fn revoke_token(
        &self,
        req: RevokeTokenRequest,
        key_manager: &JwtKeyManager,
    ) -> Result<(), AppError> {
        let (jti, ttl) = verified_token_claims(&req.token, key_manager)?;
        let entry = RevokedTokenEntry {
            jti: jti.clone(),
            revoked_at: chrono::Utc::now(),
            reason: req.reason.unwrap_or_else(|| "User logout or administrative revocation".to_string()),
        };

        let key = format!("token_blocklist:{jti}");
        let value = serde_json::to_string(&entry).map_err(AppError::Serialization)?;

        let mut conn = self.redis.get_multiplexed_async_connection().await.map_err(AppError::Redis)?;

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

#[cfg(test)]
mod tests {
    use super::*;
    use base64::{engine::general_purpose::URL_SAFE_NO_PAD, Engine as _};
    use jsonwebtoken::{encode, EncodingKey, Header};
    use serde::Serialize;

    #[derive(Serialize)]
    struct TestClaims {
        jti: String,
        exp: u64,
    }

    #[test]
    fn revocation_claims_require_a_valid_signature_and_use_signed_expiration() {
        let key_manager = JwtKeyManager::new();
        let key_pair = key_manager.active_key().unwrap();
        let expiration = chrono::Utc::now().timestamp() as u64 + 3600;
        let mut header = Header::new(Algorithm::RS256);
        header.kid = Some(key_pair.kid.clone());
        let token = encode(
            &header,
            &TestClaims {
                jti: "session-1".to_string(),
                exp: expiration,
            },
            &EncodingKey::from_rsa_pem(key_pair.private_key_pem.as_bytes()).unwrap(),
        )
        .unwrap();

        let (jti, ttl) = verified_token_claims(&token, &key_manager).unwrap();
        assert_eq!(jti, "session-1");
        assert!(ttl > 3_000 && ttl <= 3_600);

        let mut segments = token.split('.');
        let header = segments.next().unwrap();
        let payload = segments.next().unwrap();
        let signature = segments.next().unwrap();
        let mut claims: serde_json::Value = serde_json::from_slice(
            &URL_SAFE_NO_PAD.decode(payload).unwrap(),
        )
        .unwrap();
        claims["exp"] = serde_json::json!(chrono::Utc::now().timestamp() as u64 + 1);
        let payload = URL_SAFE_NO_PAD.encode(serde_json::to_vec(&claims).unwrap());
        let tampered_token = format!("{header}.{payload}.{signature}");

        assert!(verified_token_claims(&tampered_token, &key_manager).is_err());
    }
}
