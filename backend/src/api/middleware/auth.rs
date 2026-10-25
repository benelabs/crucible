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
    http::{header, HeaderValue, StatusCode},
    middleware::Next,
    response::Response,
};
use base64::Engine;
use hmac::{Hmac, Mac};
use redis::AsyncCommands;
use serde::Deserialize;
use sha2::Sha256;
use sqlx::PgPool;
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
    pub organization_id: Option<i32>,
}

impl Principal {
    /// Convenience constructor for an admin principal.
    pub fn admin(id: i32, address: impl Into<String>) -> Self {
        Self {
            id,
            address: address.into(),
            role: Role::Admin,
            organization_id: None,
        }
    }

    /// Attach an organization / tenant ID to this principal.
    pub fn with_org(mut self, org_id: i32) -> Self {
        self.organization_id = Some(org_id);
        self
    }

    fn to_auth_user(&self) -> AuthUser {
        AuthUser {
            id: self.id,
            address: self.address.clone(),
            role: self.role,
            organization_id: self.organization_id,
            permissions: Some(self.role.default_permissions()),
        }
    }
}

/// Claims extracted from a verified HS256 JWT access token.
#[derive(Debug, Clone, Deserialize)]
pub(crate) struct JwtClaims {
    /// JWT ID — used for Redis revocation lookups.
    pub(crate) jti: Option<String>,
    /// Expiration as seconds since the Unix epoch.
    pub(crate) exp: Option<i64>,
    /// Optional subject / user id.
    #[serde(default)]
    pub(crate) sub: Option<String>,
    /// Optional role claim (`admin`, `user`, …).
    #[serde(default)]
    pub(crate) role: Option<String>,
    /// Optional organization / tenant ID claim.
    #[serde(default)]
    pub(crate) org_id: Option<i32>,
    /// Optional issuer claim (`iss`).
    #[serde(default)]
    pub(crate) iss: Option<String>,
    /// Optional audience claim (`aud`).
    #[serde(default)]
    pub(crate) aud: Option<String>,
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
    /// Expected issuer claim (`iss`) for JWT verification.
    jwt_issuer: Option<String>,
    /// Expected audience claim (`aud`) for JWT verification.
    jwt_audience: Option<String>,
    /// Redis client used for `jti` blocklist lookups. When unset, JWT
    /// revocation checks are skipped (signature/expiry are still enforced).
    redis: Option<redis::Client>,
    /// Database used as the durable source of truth for account status and role.
    db: Option<PgPool>,
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

    /// Attach the database used to enforce current account status and role.
    pub fn with_db(mut self, db: PgPool) -> Self {
        self.db = Some(db);
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

    /// Attach the expected JWT issuer claim (`iss`).
    pub fn with_jwt_issuer(mut self, issuer: impl Into<String>) -> Self {
        let issuer = issuer.into();
        if !issuer.trim().is_empty() {
            self.jwt_issuer = Some(issuer);
        }
        self
    }

    /// Attach the expected JWT audience claim (`aud`).
    pub fn with_jwt_audience(mut self, audience: impl Into<String>) -> Self {
        let audience = audience.into();
        if !audience.trim().is_empty() {
            self.jwt_audience = Some(audience);
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
                // Registry credentials are service credentials, not user accounts.
                state.insert_token(token, Principal::admin(-1, "admin-service"));
            }
            _ => {
                warn!(
                    "ADMIN_API_TOKEN is not set; privileged admin/config endpoints                      are locked down and will reject all requests"
                );
            }
        }

        if let Ok(token) = std::env::var("OPERATOR_API_TOKEN") {
            if !token.trim().is_empty() {
                state.insert_token(
                    token,
                    Principal {
                        id: -2,
                        address: "operator-service".to_string(),
                        role: Role::User,
                        organization_id: None,
                    },
                );
            }
        }

        if let Ok(secret) = std::env::var("JWT_SECRET") {
            state = state.with_jwt_secret(secret);
        }

        if let Ok(issuer) = std::env::var("JWT_ISSUER") {
            state = state.with_jwt_issuer(issuer);
        }

        if let Ok(audience) = std::env::var("JWT_AUDIENCE") {
            state = state.with_jwt_audience(audience);
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
    pub async fn is_jti_revoked(&self, jti: &str) -> bool {
        self.is_token_or_user_revoked(jti, None).await
    }

    /// Returns true when `jti` or the user subject identifier is present in the Redis revocation blocklist.
    pub async fn is_token_or_user_revoked(&self, jti: &str, sub: Option<&str>) -> bool {
        let Some(redis) = &self.redis else {
            return false;
        };
        match redis.get_multiplexed_async_connection().await {
            Ok(mut conn) => {
                let jti_key = format!("{TOKEN_BLOCKLIST_PREFIX}{jti}");
                if conn.exists::<_, bool>(jti_key).await.unwrap_or(false) {
                    return true;
                }
                if let Some(user_id) = sub {
                    let user_key = format!("{TOKEN_BLOCKLIST_PREFIX}user:{user_id}");
                    if conn.exists::<_, bool>(user_key).await.unwrap_or(false) {
                        return true;
                    }
                    let blocklist_key = format!("user_blocklist:{user_id}");
                    if conn.exists::<_, bool>(blocklist_key).await.unwrap_or(false) {
                        return true;
                    }
                }
                false
            }
            Err(err) => {
                // Fail closed when Redis is configured but unreachable: a
                // revoked token must not slip through during an outage.
                warn!(error = %err, "JWT revocation lookup failed; denying request");
                true
            }
        }
    }

    /// Returns true when `user_id` is marked suspended in Redis.
    pub async fn is_user_suspended(&self, user_id: i32) -> bool {
        let Some(redis) = &self.redis else {
            return false;
        };
        let key = format!("user_suspended:{user_id}");
        match redis.get_multiplexed_async_connection().await {
            Ok(mut conn) => match conn.exists::<_, bool>(key).await {
                Ok(exists) => exists,
                Err(err) => {
                    warn!(error = %err, "User suspension lookup failed; denying request");
                    true
                }
            },
            Err(err) => {
                warn!(error = %err, "User suspension lookup failed; denying request");
                true
            }
        }
    }

    /// Refresh a JWT principal's role from PostgreSQL and fail closed if the
    /// account is absent, inactive, or the database cannot be queried.
    async fn refresh_active_principal(&self, principal: &mut Principal) -> bool {
        let Some(db) = &self.db else {
            warn!(
                user_id = principal.id,
                "JWT account lookup unavailable; denying request"
            );
            return false;
        };

        let result = if principal.id > 0 {
            sqlx::query_as::<_, (i32, bool, String, String)>(
                "SELECT id, is_active, role::text, address FROM users WHERE id = $1",
            )
            .bind(principal.id)
            .fetch_optional(db)
            .await
        } else {
            sqlx::query_as::<_, (i32, bool, String, String)>(
                "SELECT id, is_active, role::text, address FROM users WHERE address = $1",
            )
            .bind(&principal.address)
            .fetch_optional(db)
            .await
        };

        match result {
            Ok(Some((id, true, role, address))) => {
                principal.id = id;
                principal.address = address;
                principal.role = role_from_claim(Some(&role));
                true
            }
            Ok(Some((_, false, _, _))) | Ok(None) => false,
            Err(err) => {
                warn!(error = %err, user_id = principal.id, "JWT account lookup failed; denying request");
                false
            }
        }
    }

    async fn track_user_session(
        &self,
        user_id: i32,
        jti: &str,
        expires_at: i64,
    ) -> Result<(), String> {
        let Some(redis) = &self.redis else {
            return Err("Redis is required for JWT session tracking".to_string());
        };
        let ttl = expires_at
            .saturating_sub(chrono::Utc::now().timestamp())
            .min(86_400)
            .max(1) as u64;
        let sessions_key = format!("user_sessions:{user_id}");
        let mut conn = redis
            .get_multiplexed_async_connection()
            .await
            .map_err(|error| error.to_string())?;
        let script = redis::Script::new(
            r#"
            redis.call("SADD", KEYS[1], ARGV[1])
            redis.call("EXPIRE", KEYS[1], ARGV[2])
            return 1
            "#,
        );
        let _: i32 = script
            .key(sessions_key)
            .arg(jti)
            .arg(ttl)
            .invoke_async(&mut conn)
            .await
            .map_err(|error| error.to_string())?;
        Ok(())
    }

    /// Check whether a token is valid (either as a registered static token or valid JWT).
    pub async fn is_token_valid(&self, token: &str) -> bool {
        if self.tokens.contains_key(token) {
            return true;
        }

        if let Some(secret) = self.jwt_secret.as_deref() {
            if let Some(claims) = verify_hs256_jwt(
                token,
                secret,
                self.jwt_issuer.as_deref(),
                self.jwt_audience.as_deref(),
            ) {
                if let Some(jti) = claims.jti.as_deref() {
                    if self.is_jti_revoked(jti).await {
                        return false;
                    }
                }
                if let Some(sub) = claims.sub.as_deref() {
                    if self.is_token_or_user_revoked("", Some(sub)).await {
                        return false;
                    }
                }
                return true;
            }
        }

        false
    }
}

/// Errors that can occur when extracting or validating the `Authorization` header.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum AuthHeaderError {
    /// The Authorization header was missing from the request.
    Missing,
    /// The Authorization header contained non-ASCII / invalid character bytes.
    NonAscii,
    /// The Authorization header was missing the required `Bearer ` prefix.
    MissingBearerPrefix,
    /// The Bearer token was empty or contained only whitespace.
    EmptyToken,
}

impl std::fmt::Display for AuthHeaderError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Missing => write!(f, "Authentication required: missing Authorization header"),
            Self::NonAscii => write!(
                f,
                "Malformed Authorization header: contains non-ASCII characters"
            ),
            Self::MissingBearerPrefix => {
                write!(
                    f,
                    "Malformed Authorization header: missing 'Bearer ' prefix"
                )
            }
            Self::EmptyToken => write!(f, "Malformed Authorization header: empty bearer token"),
        }
    }
}

impl std::error::Error for AuthHeaderError {}

/// Gracefully extract the bearer token from the `Authorization` header, returning
/// a descriptive [`AuthHeaderError`] if the header is malformed.
pub fn extract_bearer_token(request: &Request) -> Result<String, AuthHeaderError> {
    let header_value = match request.headers().get(header::AUTHORIZATION) {
        Some(val) => val,
        None => return Err(AuthHeaderError::Missing),
    };

    let header_str = match header_value.to_str() {
        Ok(s) => s,
        Err(_) => return Err(AuthHeaderError::NonAscii),
    };

    let trimmed = header_str.trim_start();
    let token = match trimmed.strip_prefix("Bearer ") {
        Some(rest) => rest,
        None if trimmed.eq_ignore_ascii_case("Bearer") => "",
        None => return Err(AuthHeaderError::MissingBearerPrefix),
    }
    .trim();
    if token.is_empty() {
        return Err(AuthHeaderError::EmptyToken);
    }

    Ok(token.to_string())
}

/// Extract the bearer token from the `Authorization` header, if well-formed.
fn bearer_token(request: &Request) -> Option<String> {
    extract_bearer_token(request).ok()
}

fn reject(status: StatusCode, code: &str, message: &str) -> Response {
    crate::api::errors::make_problem_response(status, code, message)
}

fn b64url_decode(input: &str) -> Option<Vec<u8>> {
    base64::engine::general_purpose::URL_SAFE_NO_PAD
        .decode(input)
        .or_else(|_| base64::engine::general_purpose::URL_SAFE.decode(input))
        .ok()
}

/// Verify an HS256 JWT and return its claims, or `None` on any failure.
/// When `expected_issuer` or `expected_audience` are specified, matching claims are strictly required.
pub(crate) fn verify_hs256_jwt(
    token: &str,
    secret: &str,
    expected_issuer: Option<&str>,
    expected_audience: Option<&str>,
) -> Option<JwtClaims> {
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

    let exp = claims.exp?;
    let now = chrono::Utc::now().timestamp();
    if now >= exp {
        return None;
    }

    if let Some(expected_iss) = expected_issuer {
        if claims.iss.as_deref() != Some(expected_iss) {
            return None;
        }
    }

    if let Some(expected_aud) = expected_audience {
        if claims.aud.as_deref() != Some(expected_aud) {
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
        organization_id: claims.org_id,
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
    let token = match extract_bearer_token(&request) {
        Ok(t) => t,
        Err(err) => {
            warn!(error = %err, "Privileged request rejected: malformed or missing authorization header");
            let (code, message) = match err {
                AuthHeaderError::Missing => (
                    "unauthorized",
                    "Authentication required: provide a Bearer token",
                ),
                AuthHeaderError::NonAscii => (
                    "invalid_authorization_header",
                    "Malformed Authorization header: contains non-ASCII characters",
                ),
                AuthHeaderError::MissingBearerPrefix => (
                    "invalid_authorization_header",
                    "Malformed Authorization header: expected 'Bearer <token>'",
                ),
                AuthHeaderError::EmptyToken => (
                    "invalid_authorization_header",
                    "Malformed Authorization header: empty bearer token",
                ),
            };
            return reject(StatusCode::UNAUTHORIZED, code, message);
        }
    };

    // Static registry tokens (provisioned via env) take precedence.
    if let Some(principal) = auth.principal(&token).cloned() {
        if principal.id > 0 && auth.is_user_suspended(principal.id).await {
            return reject(
                StatusCode::UNAUTHORIZED,
                "unauthorized",
                "User account has been suspended",
            );
        }
        return authorize_principal(principal, request, next).await;
    }

    // JWT path: verify signature/expiry/iss/aud, then consult the Redis jti blocklist.
    if let Some(secret) = auth.jwt_secret.as_deref() {
        if let Some(claims) = verify_hs256_jwt(
            &token,
            secret,
            auth.jwt_issuer.as_deref(),
            auth.jwt_audience.as_deref(),
        ) {
            if let Some(jti) = claims.jti.as_deref() {
                if auth
                    .is_token_or_user_revoked(jti, claims.sub.as_deref())
                    .await
                {
                    warn!(jti, sub = ?claims.sub, "Privileged request rejected: JWT jti or user session is revoked");
                    return reject(
                        StatusCode::UNAUTHORIZED,
                        "unauthorized",
                        "Token or session has been revoked",
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

            let mut principal = principal_from_jwt_claims(&claims);
            if !auth.refresh_active_principal(&mut principal).await
                || auth.is_user_suspended(principal.id).await
            {
                warn!(
                    user_id = principal.id,
                    "Privileged request rejected: user account is suspended or unavailable"
                );
                return reject(
                    StatusCode::UNAUTHORIZED,
                    "unauthorized",
                    "User account is suspended or unavailable",
                );
            }

            if let (Some(jti), Some(exp)) = (claims.jti.as_deref(), claims.exp) {
                if let Err(err) = auth.track_user_session(principal.id, jti, exp).await {
                    warn!(error = %err, user_id = principal.id, "Session tracking failed; denying request");
                    return reject(
                        StatusCode::SERVICE_UNAVAILABLE,
                        "service_unavailable",
                        "Session validation is temporarily unavailable",
                    );
                }
            }

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
                organization_id: None,
            },
        );
        auth
    }

    fn guarded_app(auth: AdminAuthState) -> Router {
        Router::new()
            .route("/admin", get(|| async { "ok" }))
            .route_layer(middleware::from_fn_with_state(
                Arc::new(auth),
                require_admin_auth,
            ))
    }

    async fn response_for(auth_header: Option<HeaderValue>) -> Response {
        let mut builder = Request::builder().uri("/admin");
        if let Some(value) = auth_header {
            builder = builder.header(header::AUTHORIZATION, value);
        }
        let request = builder.body(Body::empty()).unwrap();
        guarded_app(registry()).oneshot(request).await.unwrap()
    }

    async fn status_for(auth_header: Option<&str>) -> StatusCode {
        let header_val = auth_header.map(|s| HeaderValue::from_str(s).unwrap());
        response_for(header_val).await.status()
    }

    fn sign_jwt(secret: &str, payload_json: &str) -> String {
        let header = base64::engine::general_purpose::URL_SAFE_NO_PAD
            .encode(r#"{"alg":"HS256","typ":"JWT"}"#);
        let payload = base64::engine::general_purpose::URL_SAFE_NO_PAD.encode(payload_json);
        let signing_input = format!("{header}.{payload}");
        let mut mac = HmacSha256::new_from_slice(secret.as_bytes()).unwrap();
        mac.update(signing_input.as_bytes());
        let sig =
            base64::engine::general_purpose::URL_SAFE_NO_PAD.encode(mac.finalize().into_bytes());
        format!("{signing_input}.{sig}")
    }

    #[tokio::test]
    async fn unauthenticated_request_is_rejected() {
        let resp = response_for(None).await;
        assert_eq!(resp.status(), StatusCode::UNAUTHORIZED);
        assert_eq!(
            resp.headers().get(header::CONTENT_TYPE).unwrap(),
            "application/problem+json"
        );
    }

    #[tokio::test]
    async fn malformed_header_is_rejected() {
        let resp = response_for(Some(HeaderValue::from_static("Basic abc"))).await;
        assert_eq!(resp.status(), StatusCode::UNAUTHORIZED);
        assert_eq!(
            resp.headers().get(header::CONTENT_TYPE).unwrap(),
            "application/problem+json"
        );
    }

    #[tokio::test]
    async fn malformed_header_with_non_ascii_is_rejected() {
        let non_ascii_val = HeaderValue::from_bytes(b"Bearer \xFF\xFE\xFA\xFB").unwrap();
        let resp = response_for(Some(non_ascii_val)).await;
        assert_eq!(resp.status(), StatusCode::UNAUTHORIZED);
        assert_eq!(
            resp.headers().get(header::CONTENT_TYPE).unwrap(),
            "application/problem+json"
        );
    }

    #[tokio::test]
    async fn malformed_header_with_empty_token_is_rejected() {
        let resp = response_for(Some(HeaderValue::from_static("Bearer   "))).await;
        assert_eq!(resp.status(), StatusCode::UNAUTHORIZED);
        assert_eq!(
            resp.headers().get(header::CONTENT_TYPE).unwrap(),
            "application/problem+json"
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
        let resp = response_for(Some(HeaderValue::from_static("Bearer operator-secret"))).await;
        assert_eq!(resp.status(), StatusCode::FORBIDDEN);
        assert_eq!(
            resp.headers().get(header::CONTENT_TYPE).unwrap(),
            "application/problem+json"
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
    fn extract_bearer_token_variants() {
        let req_valid = Request::builder()
            .header(header::AUTHORIZATION, "Bearer test-token-123")
            .body(Body::empty())
            .unwrap();
        assert_eq!(
            extract_bearer_token(&req_valid),
            Ok("test-token-123".to_string())
        );

        let req_missing = Request::builder().body(Body::empty()).unwrap();
        assert_eq!(
            extract_bearer_token(&req_missing),
            Err(AuthHeaderError::Missing)
        );

        let req_non_ascii = Request::builder()
            .header(
                header::AUTHORIZATION,
                HeaderValue::from_bytes(b"Bearer \x80\x81").unwrap(),
            )
            .body(Body::empty())
            .unwrap();
        assert_eq!(
            extract_bearer_token(&req_non_ascii),
            Err(AuthHeaderError::NonAscii)
        );

        let req_basic = Request::builder()
            .header(header::AUTHORIZATION, "Basic dXNlcjpwYXNz")
            .body(Body::empty())
            .unwrap();
        assert_eq!(
            extract_bearer_token(&req_basic),
            Err(AuthHeaderError::MissingBearerPrefix)
        );

        let req_empty = Request::builder()
            .header(header::AUTHORIZATION, "Bearer   ")
            .body(Body::empty())
            .unwrap();
        assert_eq!(
            extract_bearer_token(&req_empty),
            Err(AuthHeaderError::EmptyToken)
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
        let claims = verify_hs256_jwt(&token, secret, None, None).expect("valid jwt");
        assert_eq!(claims.jti.as_deref(), Some("abc-123"));
    }

    #[test]
    fn verify_hs256_jwt_rejects_expired_token() {
        let secret = "test-secret";
        let exp = chrono::Utc::now().timestamp() - 10;
        let token = sign_jwt(secret, &format!(r#"{{"jti":"abc-123","exp":{exp}}}"#));
        assert!(verify_hs256_jwt(&token, secret, None, None).is_none());
    }

    #[test]
    fn verify_hs256_jwt_enforces_issuer_and_audience() {
        let secret = "test-secret";
        let exp = chrono::Utc::now().timestamp() + 3600;
        let token = sign_jwt(
            secret,
            &format!(
                r#"{{"jti":"iss-aud","exp":{exp},"iss":"crucible-auth","aud":"crucible-api"}}"#
            ),
        );

        // Matching issuer and audience
        assert!(
            verify_hs256_jwt(&token, secret, Some("crucible-auth"), Some("crucible-api")).is_some()
        );

        // Mismatched issuer
        assert!(
            verify_hs256_jwt(&token, secret, Some("wrong-issuer"), Some("crucible-api")).is_none()
        );

        // Mismatched audience
        assert!(verify_hs256_jwt(
            &token,
            secret,
            Some("crucible-auth"),
            Some("wrong-audience")
        )
        .is_none());
    }

    #[test]
    fn verify_hs256_jwt_rejects_token_without_expiration() {
        let secret = "test-secret";
        let token = sign_jwt(secret, r#"{"jti":"no-exp","sub":"1","role":"admin"}"#);
        assert!(verify_hs256_jwt(&token, secret, None, None).is_none());
    }

    #[tokio::test]
    async fn jwt_admin_token_is_rejected_without_authoritative_account_store() {
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
        assert_eq!(status, StatusCode::UNAUTHORIZED);
    }

    #[tokio::test]
    async fn redis_connection_failures_fail_closed_for_revocation_checks() {
        let auth =
            AdminAuthState::new().with_redis(redis::Client::open("redis://127.0.0.1:1").unwrap());

        assert!(auth.is_jti_revoked("jti-unavailable").await);
        assert!(auth.is_user_suspended(42).await);
    }

    #[test]
    fn principal_from_jwt_claims_extracts_numeric_user_id() {
        let claims = JwtClaims {
            jti: Some("jti-123".to_string()),
            exp: Some(chrono::Utc::now().timestamp() + 3600),
            sub: Some("42".to_string()),
            role: Some("admin".to_string()),
            org_id: None,
            iss: None,
            aud: None,
        };
        let principal = principal_from_jwt_claims(&claims);
        assert_eq!(principal.id, 42);
        assert_eq!(principal.role, Role::Admin);
    }

    #[test]
    fn principal_from_jwt_claims_handles_non_numeric_sub() {
        let claims = JwtClaims {
            jti: Some("jti-456".to_string()),
            exp: Some(chrono::Utc::now().timestamp() + 3600),
            sub: Some("non-numeric-wallet".to_string()),
            role: Some("user".to_string()),
            org_id: None,
            iss: None,
            aud: None,
        };
        let principal = principal_from_jwt_claims(&claims);
        assert_eq!(principal.id, 0);
        assert_eq!(principal.address, "non-numeric-wallet");
        assert_eq!(principal.role, Role::User);
    }

    #[tokio::test]
    async fn jwt_admin_token_enforces_configured_issuer_and_audience() {
        let secret = "jwt-secret";
        let exp = chrono::Utc::now().timestamp() + 3600;
        let valid_token = sign_jwt(
            secret,
            &format!(
                r#"{{"jti":"valid-jti","exp":{exp},"sub":"9","role":"admin","iss":"crucible-prod","aud":"crucible-admin"}}"#
            ),
        );
        let staging_token = sign_jwt(
            secret,
            &format!(
                r#"{{"jti":"staging-jti","exp":{exp},"sub":"9","role":"admin","iss":"crucible-staging","aud":"crucible-admin"}}"#
            ),
        );
        let wrong_aud_token = sign_jwt(
            secret,
            &format!(
                r#"{{"jti":"other-aud","exp":{exp},"sub":"9","role":"admin","iss":"crucible-prod","aud":"crucible-readonly"}}"#
            ),
        );

        let auth = AdminAuthState::new()
            .with_jwt_secret(secret)
            .with_jwt_issuer("crucible-prod")
            .with_jwt_audience("crucible-admin");

        // A token with the expected issuer and audience passes header validation and
        // reaches the account-lookup stage. No database is configured here, so the
        // fail-closed guard denies it there instead of rejecting it as a bad credential.
        let (status_valid, detail_valid) = guarded_response(auth.clone(), &valid_token).await;
        assert_eq!(status_valid, StatusCode::UNAUTHORIZED);
        assert!(
            detail_valid.contains("suspended or unavailable"),
            "issuer/audience mismatch rejected the token: {detail_valid}"
        );

        // Wrong issuer and wrong audience never pass signature validation.
        for token in [&staging_token, &wrong_aud_token] {
            let (status, detail) = guarded_response(auth.clone(), token).await;
            assert_eq!(status, StatusCode::UNAUTHORIZED);
            assert!(
                detail.contains("Invalid authentication credentials"),
                "expected a credential rejection, got: {detail}"
            );
        }
    }

    async fn guarded_response(auth: AdminAuthState, token: &str) -> (StatusCode, String) {
        let request = Request::builder()
            .uri("/admin")
            .method("GET")
            .header(header::AUTHORIZATION, format!("Bearer {token}"))
            .body(Body::empty())
            .unwrap();
        let response = guarded_app(auth).oneshot(request).await.unwrap();
        let status = response.status();
        let bytes = axum::body::to_bytes(response.into_body(), usize::MAX)
            .await
            .unwrap();
        (status, String::from_utf8_lossy(&bytes).to_string())
    }
}
