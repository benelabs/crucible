//! Auth Handlers providing Asymmetric JWT Key Rotation and Token Revocation endpoints.

pub mod jwt;
pub mod revocation;

pub use jwt::{JwkKey, JwksResponse, JwtKeyManager, JwtKeyPair};
pub use revocation::{RevokeTokenRequest, RevokedTokenEntry, TokenBlocklistService};

use axum::{
    extract::State,
    response::IntoResponse,
    routing::{get, post},
    Json, Router,
};
use std::sync::Arc;
use tracing::instrument;

#[derive(Clone)]
pub struct AuthState {
    pub key_manager: Arc<JwtKeyManager>,
    pub blocklist: Arc<TokenBlocklistService>,
}

#[utoipa::path(
    get,
    path = "/api/v1/auth/jwks",
    responses((status = 200, description = "JSON Web Key Set for JWT verification", body = JwksResponse)),
    tag = "auth"
)]
#[instrument(skip(state))]
pub async fn get_jwks(State(state): State<AuthState>) -> impl IntoResponse {
    Json(state.key_manager.get_jwks())
}

#[utoipa::path(
    post,
    path = "/api/v1/auth/revoke",
    request_body = RevokeTokenRequest,
    responses((status = 200, description = "JWT Token successfully revoked")),
    tag = "auth"
)]
#[instrument(skip(state))]
pub async fn revoke_token(
    State(state): State<AuthState>,
    Json(payload): Json<RevokeTokenRequest>,
) -> Result<impl IntoResponse, crate::error::AppError> {
    state.blocklist.revoke_token(payload).await?;
    Ok(axum::http::StatusCode::OK)
}

#[utoipa::path(
    post,
    path = "/api/v1/auth/rotate",
    responses((status = 200, description = "JWT Key pair rotated successfully", body = JwtKeyPair)),
    tag = "auth"
)]
#[instrument(skip(state))]
pub async fn rotate_keys(State(state): State<AuthState>) -> impl IntoResponse {
    let new_pair = state.key_manager.rotate_keys();
    Json(new_pair)
}

pub fn routes(state: AuthState) -> Router {
    Router::new()
        .route("/jwks", get(get_jwks))
        .route("/revoke", post(revoke_token))
        .route("/rotate", post(rotate_keys))
        .with_state(state)
}
