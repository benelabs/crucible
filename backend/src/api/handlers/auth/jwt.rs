//! Asymmetric JWT Signing with Automated Key Rotation and JWKS Public Key Distribution.

use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::sync::{Arc, RwLock};
use chrono::{DateTime, Utc};
use utoipa::ToSchema;

/// JSON Web Key (JWK) representation for public key distribution via JWKS.
#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
pub struct JwkKey {
    pub kty: String,
    pub use_: String,
    pub alg: String,
    pub kid: String,
    pub n: String,
    pub e: String,
}

/// JSON Web Key Set (JWKS) response containing active and historical public keys.
#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
pub struct JwksResponse {
    pub keys: Vec<JwkKey>,
}

/// Key metadata for an asymmetric key pair used in JWT signing and verification.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct JwtKeyPair {
    pub kid: String,
    pub public_key_pem: String,
    pub private_key_pem: String,
    pub created_at: DateTime<Utc>,
    pub expires_at: DateTime<Utc>,
    pub is_active: bool,
}

/// Key Manager handling automated key rotation, JWKS generation, and active signing key retrieval.
#[derive(Clone)]
pub struct JwtKeyManager {
    keys: Arc<RwLock<HashMap<String, JwtKeyPair>>>,
    active_kid: Arc<RwLock<String>>,
}

impl Default for JwtKeyManager {
    fn default() -> Self {
        Self::new()
    }
}

impl JwtKeyManager {
    pub fn new() -> Self {
        let initial_pair = Self::generate_key_pair("key-v1");
        let kid = initial_pair.kid.clone();

        let mut keys_map = HashMap::new();
        keys_map.insert(kid.clone(), initial_pair);

        Self {
            keys: Arc::new(RwLock::new(keys_map)),
            active_kid: Arc::new(RwLock::new(kid)),
        }
    }

    /// Generates a mock asymmetric key pair structure with key ID `kid`.
    pub fn generate_key_pair(kid: &str) -> JwtKeyPair {
        let now = Utc::now();
        JwtKeyPair {
            kid: kid.to_string(),
            public_key_pem: format!("-----BEGIN PUBLIC KEY-----\nMockPublicKeyData_{kid}\n-----END PUBLIC KEY-----"),
            private_key_pem: format!("-----BEGIN RSA PRIVATE KEY-----\nMockPrivateKeyData_{kid}\n-----END RSA PRIVATE KEY-----"),
            created_at: now,
            expires_at: now + chrono::Duration::days(30),
            is_active: true,
        }
    }

    /// Rotates the active JWT signing key by generating a new key pair and marking previous keys as passive.
    pub fn rotate_keys(&self) -> JwtKeyPair {
        let next_id = format!("key-v{}", Utc::now().timestamp());
        let new_pair = Self::generate_key_pair(&next_id);
        let kid = new_pair.kid.clone();

        let mut keys = self.keys.write().unwrap();

        // Deactivate previous active keys for signing (they remain valid for verification until expired)
        for key in keys.values_mut() {
            key.is_active = false;
        }

        keys.insert(kid.clone(), new_pair.clone());
        let mut active_kid = self.active_kid.write().unwrap();
        *active_kid = kid;

        new_pair
    }

    /// Retrieves the current active signing key pair.
    pub fn active_key(&self) -> Option<JwtKeyPair> {
        let active_kid = self.active_kid.read().unwrap();
        let keys = self.keys.read().unwrap();
        keys.get(&*active_kid).cloned()
    }

    /// Finds a public/private key pair by Key ID (`kid`).
    pub fn get_key(&self, kid: &str) -> Option<JwtKeyPair> {
        let keys = self.keys.read().unwrap();
        keys.get(kid).cloned()
    }

    /// Generates the JWKS payload containing public keys for external validation.
    pub fn get_jwks(&self) -> JwksResponse {
        let keys = self.keys.read().unwrap();
        let jwk_keys = keys
            .values()
            .map(|kp| JwkKey {
                kty: "RSA".to_string(),
                use_: "sig".to_string(),
                alg: "RS256".to_string(),
                kid: kp.kid.clone(),
                n: base64::Engine::encode(&base64::engine::general_purpose::URL_SAFE_NO_PAD, kp.public_key_pem.as_bytes()),
                e: "AQAB".to_string(),
            })
            .collect();

        JwksResponse { keys: jwk_keys }
    }
}
