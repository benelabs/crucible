//! Authentication & authorization for privileged backend endpoints.
//!
//! The admin, config-reload, logs, maintenance and diagnostics routes must not
//! be reachable without a clear auth model. This module implements a
//! **bearer-token / JWT** scheme used as a guard layer on those routes:
//!
//! 1. Requests must present an `Authorization: Bearer <token>` header.
//! 2. Static registry tokens ([`AdminAuthState`]) are resolved first.
//! 3. JWT bearer tokens are verified (HS256 signature + `exp`) and the `jti`
//!    claim is checked against the Redis revocation blocklist before access
//!    is granted. A revoked / logged-out token is rejected with `401`.
//! 4. Privileged routes require [`Role::Admin`]; any lower role is rejected
//!    with `403 Forbidden`.
//!
//! On success the resolved [`AuthUser`] is inserted into the request
//! extensions so downstream handlers and the finer-grained
//! [`super::permissions`] checks can read the authenticated identity.
//!
//! Tokens are provisioned out-of-band via environment variables
//! (`ADMIN_API_TOKEN`, optionally `OPERATOR_API_TOKEN`, `JWT_SECRET`,
//! `REDIS_URL`) so secrets never live in code. If no admin token is
//! configured the privileged routes are effectively locked down (every
//! request is rejected), which is the safe default.

use std::collections::HashMap;
use std::sync::Arc;

use axum::{
    extract::{Request, State},
    http::{header, StatusCode},
    middleware::Next,
    response::Response,
};
use base64::Engine;
use hmac::{Hmac, Mac};
use redis::AsyncCommands;
use serde::Deserialize;
use sha2::Sha256;
use tracing::{debug, warn};

use super::permissions::{AuthUser, Role};

type HmacSha256 = Hmac<Sha256>;

/// Redis key prefix used by [`crate::api::handlers::auth::TokenBlocklistService`].
const TOKEN_BLOCKLIST_PREFIX: &str = "token_blocklist:";

/// An authenticated identity associated with a bearer token.
#[derive(Clone, Debug)]
pub struct Principal {
    pub id: i32,
    pub address: String,
    pub role: Role,
}

impl Principal {
    /// Convenience constructor for an admin principal.
    pub fn admin(id: i32, address: impl Into<String>) -> Self {
        Self {
            id,
            address: address.into(),
            role: Role::Admin,
        }
    }

    fn to_auth_user(&self) -> AuthUser {
        AuthUser {
            id: self.id,
            address: self.address.clone(),
            role: self.role,
        }
    }
}

/// Claims extracted from a verified HS256 JWT access token.
#[derive(Debug, Clone, Deserialize)]
struct JwtClaims {
    /// JWT ID — used for Redis revocation lookups.
    jti: Option<String>,
    /// Expiration as seconds since the Unix epoch.
    exp: Option<i64>,
    /// Optional subject / user id.
    #[serde(default)]
    sub: Option<String>,
    /// Optional role claim (`admin`, `user`, …).
    #[serde(default)]
    role: Option<String>,
}

/// Registry mapping bearer tokens to the principals they authenticate, plus
/// optional JWT verification / Redis revocation state.
///
/// Shared as `Arc<AdminAuthState>` and baked into the middleware via
/// [`axum::middleware::from_fn_with_state`].
#[derive(Clone, Default)]
pub struct AdminAuthState {
    tokens: HashMap<String, Principal>,
    /// HS256 secret used to verify JWT bearer tokens. When unset, only the
    /// static token registry is consulted.
    jwt_secret: Option<String>,
    /// Redis client used for `jti` blocklist lookups. When unset, JWT
    /// revocation checks are skipped (signature/expiry are still enforced).
    redis: Option<redis::Client>,
}

impl AdminAuthState {
    /// Create an empty registry (rejects everything).
    pub fn new() -> Self {
        Self::default()
    }

    /// Attach a Redis client used for JWT `jti` revocation checks.
    pub fn with_redis(mut self, redis: redis::Client) -> Self {
        self.redis = Some(redis);
        self
    }

    /// Attach the HS256 JWT signing secret.
    pub fn with_jwt_secret(mut self, secret: impl Into<String>) -> Self {
        let secret = secret.into();
        if !secret.trim().is_empty() {
            self.jwt_secret = Some(secret);
        }
        self
    }

    /// Build a registry from environment variables.
    ///
    /// * `ADMIN_API_TOKEN` — grants [`Role::Admin`] (required to reach
    ///   privileged routes).
    /// * `OPERATOR_API_TOKEN` — optional, grants [`Role::User`]; useful for
    ///   tokens that authenticate but are intentionally *not* authorized for
    ///   admin actions.
    /// * `JWT_SECRET` — optional HS256 secret for JWT bearer verification.
    /// * `REDIS_URL` — optional Redis endpoint for `jti` blocklist lookups.
    pub fn from_env() -> Self {
        let mut state = Self::new();

        match std::env::var("ADMIN_API_TOKEN") {
            Ok(token) if !token.trim().is_empty() => {
                state.insert_token(token, Principal::admin(1, "admin"));
            }
            _ => {
                warn!(
                    "ADMIN_API_TOKEN is not set; privileged admin/config endpoints \
                     are locked down and will reject all requests"
                );
            }
        }

        if let Ok(token) = std::env::var("OPERATOR_API_TOKEN") {
            if !token.trim().is_empty() {
                state.insert_token(
                    token,
                    Principal {
                        id: 2,
                        address: "operator".to_string(),
                        role: Role::User,
                    },
                );
            }
        }

        if let Ok(secret) = std::env::var("JWT_SECRET") {
            state = state.with_jwt_secret(secret);
        }

        if let Ok(url) = std::env::var("REDIS_URL") {
            if !url.trim().is_empty() {
                match redis::Client::open(url) {
                    Ok(client) => state = state.with_redis(client),
                    Err(err) => warn!(error = %err, "Failed to configure Redis for JWT revocation"),
                }
            }
        }

        state
    }

    /// Register a token → principal mapping.
    pub fn insert_token(&mut self, token: impl Into<String>, principal: Principal) {
        self.tokens.insert(token.into(), principal);
    }

    /// Resolve a static registry token to its principal, if registered.
    pub fn principal(&self, token: &str) -> Option<&Principal> {
        self.tokens.get(token)
    }

    /// Returns true when `jti` is present in the Redis revocation blocklist.
    async fn is_jti_revoked(&self, jti: &str) -> bool {
        let Some(redis) = &self.redis else {
            return false;
        };
        let key = format!("{TOKEN_BLOCKLIST_PREFIX}{jti}");
        match redis.get_multiplexed_async_connection().await {
            Ok(mut conn) => conn.exists::<_, bool>(key).await.unwrap_or(false),
            Err(err) => {
                // Fail closed when Redis is configured but unreachable: a
                // revoked token must not slip through during an outage.
                warn!(error = %err, "JWT revocation lookup failed; denying request");
                true
            }
        }
    }
}

/// Extract the bearer token from the `Authorization` header, if well-formed.
fn bearer_token(request: &Request) -> Option<String> {
    let value = request.headers().get(header::AUTHORIZATION)?.to_str().ok()?;
    let token = value.strip_prefix("Bearer ")?.trim();
    if token.is_empty() {
        None
    } else {
        Some(token.to_string())
    }
}

fn reject(status: StatusCode, code: &str, message: &str) -> Response {
    crate::api::errors::make_error_response(status, code, message)
}

fn b64url_decode(input: &str) -> Option<Vec<u8>> {
    base64::engine::general_purpose::URL_SAFE_NO_PAD
        .decode(input)
        .or_else(|_| base64::engine::general_purpose::URL_SAFE.decode(input))
        .ok()
}

/// Verify an HS256 JWT and return its claims, or `None` on any failure.
fn verify_hs256_jwt(token: &str, secret: &str) -> Option<JwtClaims> {
    let mut parts = token.split('.');
    let header = parts.next()?;
    let payload = parts.next()?;
    let signature = parts.next()?;
    if parts.next().is_some() {
        return None;
    }

    let signing_input = format!("{header}.{payload}");
    let mut mac = HmacSha256::new_from_slice(secret.as_bytes()).ok()?;
    mac.update(signing_input.as_bytes());
    let expected = mac.finalize().into_bytes();
    let actual = b64url_decode(signature)?;
    if expected.as_slice() != actual.as_slice() {
        return None;
    }

    let claims_bytes = b64url_decode(payload)?;
    let claims: JwtClaims = serde_json::from_slice(&claims_bytes).ok()?;

    if let Some(exp) = claims.exp {
        let now = chrono::Utc::now().timestamp();
        if now >= exp {
            return None;
        }
    }

    Some(claims)
}

fn role_from_claim(role: Option<&str>) -> Role {
    match role.map(|r| r.to_ascii_lowercase()).as_deref() {
        Some("admin") => Role::Admin,
        Some("developer") => Role::Developer,
        Some("auditor") => Role::Auditor,
        Some("viewer") => Role::Viewer,
        Some("guest") => Role::Guest,
        _ => Role::User,
    }
}

fn principal_from_jwt_claims(claims: &JwtClaims) -> Principal {
    let id = claims
        .sub
        .as_deref()
        .and_then(|s| s.parse::<i32>().ok())
        .unwrap_or(0);
    let address = claims
        .sub
        .clone()
        .unwrap_or_else(|| "jwt-subject".to_string());
    Principal {
        id,
        address,
        role: role_from_claim(claims.role.as_deref()),
    }
}

/// Axum middleware enforcing authentication **and** admin authorization on the
/// routes it wraps.
///
/// * Missing / malformed / unknown token → `401 Unauthorized`.
/// * JWT with a revoked `jti` → `401 Unauthorized`.
/// * Recognised token without [`Role::Admin`] → `403 Forbidden`.
/// * Admin token → request proceeds with [`AuthUser`] injected.
pub async fn require_admin_auth(
    State(auth): State<Arc<AdminAuthState>>,
    request: Request,
    next: Next,
) -> Response {
    let Some(token) = bearer_token(&request) else {
        debug!("Privileged request rejected: missing bearer token");
        return reject(
            StatusCode::UNAUTHORIZED,
            "unauthorized",
            "Authentication required: provide a Bearer token",
        );
    };

    // Static registry tokens (provisioned via env) take precedence.
    if let Some(principal) = auth.principal(&token).cloned() {
        return authorize_principal(principal, request, next).await;
    }

    // JWT path: verify signature/expiry, then consult the Redis jti blocklist.
    if let Some(secret) = auth.jwt_secret.as_deref() {
        if let Some(claims) = verify_hs256_jwt(&token, secret) {
            if let Some(jti) = claims.jti.as_deref() {
                if auth.is_jti_revoked(jti).await {
                    warn!(jti, "Privileged request rejected: JWT jti is revoked");
                    return reject(
                        StatusCode::UNAUTHORIZED,
                        "unauthorized",
                        "Token has been revoked",
                    );
                }
            } else {
                warn!("Privileged request rejected: JWT missing jti claim");
                return reject(
                    StatusCode::UNAUTHORIZED,
                    "unauthorized",
                    "Invalid authentication credentials",
                );
            }

            let principal = principal_from_jwt_claims(&claims);
            return authorize_principal(principal, request, next).await;
        }
    }

    warn!("Privileged request rejected: unknown token");
    reject(
        StatusCode::UNAUTHORIZED,
        "unauthorized",
        "Invalid authentication credentials",
    )
}

async fn authorize_principal(principal: Principal, mut request: Request, next: Next) -> Response {
    if principal.role == Role::Admin {
        let user = principal.to_auth_user();
        debug!(user_id = user.id, "Admin access granted");
        request.extensions_mut().insert(user);
        next.run(request).await
    } else {
        warn!(
            user_id = principal.id,
            role = ?principal.role,
            "Privileged request rejected: insufficient role"
        );
        reject(
            StatusCode::FORBIDDEN,
            "forbidden",
            "Admin role required for this endpoint",
        )
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use axum::{body::Body, middleware, routing::get, Router};
    use tower::ServiceExt; // for `oneshot`

    fn registry() -> AdminAuthState {
        let mut auth = AdminAuthState::new();
        auth.insert_token("admin-secret", Principal::admin(1, "admin"));
        auth.insert_token(
            "operator-secret",
            Principal {
                id: 2,
                address: "operator".into(),
                role: Role::User,
            },
        );
        auth
    }

    fn guarded_app(auth: AdminAuthState) -> Router {
        Router::new().route("/admin", get(|| async { "ok" })).route_layer(
            middleware::from_fn_with_state(Arc::new(auth), require_admin_auth),
        )
    }

    async fn status_for(auth_header: Option<&str>) -> StatusCode {
        let mut builder = Request::builder().uri("/admin");
        if let Some(value) = auth_header {
            builder = builder.header(header::AUTHORIZATION, value);
        }
        let request = builder.body(Body::empty()).unwrap();
        guarded_app(registry())
            .oneshot(request)
            .await
            .unwrap()
            .status()
    }

    fn sign_jwt(secret: &str, payload_json: &str) -> String {
        let header = base64::engine::general_purpose::URL_SAFE_NO_PAD
            .encode(r#"{"alg":"HS256","typ":"JWT"}"#);
        let payload = base64::engine::general_purpose::URL_SAFE_NO_PAD.encode(payload_json);
        let signing_input = format!("{header}.{payload}");
        let mut mac = HmacSha256::new_from_slice(secret.as_bytes()).unwrap();
        mac.update(signing_input.as_bytes());
        let sig = base64::engine::general_purpose::URL_SAFE_NO_PAD.encode(mac.finalize().into_bytes());
        format!("{signing_input}.{sig}")
    }

    #[tokio::test]
    async fn unauthenticated_request_is_rejected() {
        assert_eq!(status_for(None).await, StatusCode::UNAUTHORIZED);
    }

    #[tokio::test]
    async fn malformed_header_is_rejected() {
        assert_eq!(
            status_for(Some("Basic abc")).await,
            StatusCode::UNAUTHORIZED
        );
    }

    #[tokio::test]
    async fn unknown_token_is_rejected() {
        assert_eq!(
            status_for(Some("Bearer not-a-real-token")).await,
            StatusCode::UNAUTHORIZED
        );
    }

    #[tokio::test]
    async fn non_admin_token_is_forbidden() {
        assert_eq!(
            status_for(Some("Bearer operator-secret")).await,
            StatusCode::FORBIDDEN
        );
    }

    #[tokio::test]
    async fn admin_token_is_authorized() {
        assert_eq!(
            status_for(Some("Bearer admin-secret")).await,
            StatusCode::OK
        );
    }

    #[test]
    fn empty_admin_registry_rejects_lookup() {
        let auth = AdminAuthState::new();
        assert!(auth.principal("anything").is_none());
    }

    #[test]
    fn verify_hs256_jwt_accepts_valid_token() {
        let secret = "test-secret";
        let exp = chrono::Utc::now().timestamp() + 3600;
        let token = sign_jwt(
            secret,
            &format!(r#"{{"jti":"abc-123","exp":{exp},"sub":"1","role":"admin"}}"#),
        );
        let claims = verify_hs256_jwt(&token, secret).expect("valid jwt");
        assert_eq!(claims.jti.as_deref(), Some("abc-123"));
    }

    #[test]
    fn verify_hs256_jwt_rejects_expired_token() {
        let secret = "test-secret";
        let exp = chrono::Utc::now().timestamp() - 10;
        let token = sign_jwt(secret, &format!(r#"{{"jti":"abc-123","exp":{exp}}}"#));
        assert!(verify_hs256_jwt(&token, secret).is_none());
    }

    #[tokio::test]
    async fn jwt_admin_token_is_authorized_when_not_revoked() {
        let secret = "jwt-secret";
        let exp = chrono::Utc::now().timestamp() + 3600;
        let token = sign_jwt(
            secret,
            &format!(r#"{{"jti":"live-jti","exp":{exp},"sub":"9","role":"admin"}}"#),
        );
        let auth = AdminAuthState::new().with_jwt_secret(secret);
        let request = Request::builder()
            .uri("/admin")
            .header(header::AUTHORIZATION, format!("Bearer {token}"))
            .body(Body::empty())
            .unwrap();
        let status = guarded_app(auth).oneshot(request).await.unwrap().status();
        assert_eq!(status, StatusCode::OK);
    }
}
