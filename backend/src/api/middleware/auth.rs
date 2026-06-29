//! Authentication & authorization for privileged backend endpoints.
//!
//! The admin, config-reload, logs, maintenance and diagnostics routes must not
//! be reachable without a clear auth model. This module implements a simple,
//! self-contained **bearer-token** scheme used as a guard layer on those
//! routes:
//!
//! 1. Requests must present an `Authorization: Bearer <token>` header.
//! 2. The token is looked up in the configured registry ([`AdminAuthState`]).
//!    An unknown or missing token is rejected with `401 Unauthorized`.
//! 3. A recognised token maps to a [`Principal`] with a [`Role`]. Privileged
//!    routes require [`Role::Admin`]; any lower role is rejected with
//!    `403 Forbidden`.
//!
//! On success the resolved [`AuthUser`] is inserted into the request
//! extensions so downstream handlers and the finer-grained
//! [`super::permissions`] checks can read the authenticated identity.
//!
//! Tokens are provisioned out-of-band via environment variables
//! (`ADMIN_API_TOKEN`, optionally `OPERATOR_API_TOKEN`) so secrets never live
//! in code. If no admin token is configured the privileged routes are
//! effectively locked down (every request is rejected), which is the safe
//! default.

use std::collections::HashMap;
use std::sync::Arc;

use axum::{
    extract::{Request, State},
    http::{header, StatusCode},
    middleware::Next,
    response::{IntoResponse, Response},
    Json,
};
use serde_json::json;
use tracing::{debug, warn};

use super::permissions::{AuthUser, Role};

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

/// Registry mapping bearer tokens to the principals they authenticate.
///
/// Shared as `Arc<AdminAuthState>` and baked into the middleware via
/// [`axum::middleware::from_fn_with_state`].
#[derive(Clone, Default)]
pub struct AdminAuthState {
    tokens: HashMap<String, Principal>,
}

impl AdminAuthState {
    /// Create an empty registry (rejects everything).
    pub fn new() -> Self {
        Self::default()
    }

    /// Build a registry from environment variables.
    ///
    /// * `ADMIN_API_TOKEN` — grants [`Role::Admin`] (required to reach
    ///   privileged routes).
    /// * `OPERATOR_API_TOKEN` — optional, grants [`Role::User`]; useful for
    ///   tokens that authenticate but are intentionally *not* authorized for
    ///   admin actions.
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

        state
    }

    /// Register a token → principal mapping.
    pub fn insert_token(&mut self, token: impl Into<String>, principal: Principal) {
        self.tokens.insert(token.into(), principal);
    }

    /// Resolve a token to its principal, if registered.
    pub fn principal(&self, token: &str) -> Option<&Principal> {
        self.tokens.get(token)
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
    (status, Json(json!({ "code": code, "message": message }))).into_response()
}

/// Axum middleware enforcing authentication **and** admin authorization on the
/// routes it wraps.
///
/// * Missing / malformed / unknown token → `401 Unauthorized`.
/// * Recognised token without [`Role::Admin`] → `403 Forbidden`.
/// * Admin token → request proceeds with [`AuthUser`] injected.
pub async fn require_admin_auth(
    State(auth): State<Arc<AdminAuthState>>,
    mut request: Request,
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

    match auth.principal(&token) {
        None => {
            warn!("Privileged request rejected: unknown token");
            reject(
                StatusCode::UNAUTHORIZED,
                "unauthorized",
                "Invalid authentication credentials",
            )
        }
        Some(principal) if principal.role == Role::Admin => {
            let user = principal.to_auth_user();
            debug!(user_id = user.id, "Admin access granted");
            request.extensions_mut().insert(user);
            next.run(request).await
        }
        Some(principal) => {
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
}
