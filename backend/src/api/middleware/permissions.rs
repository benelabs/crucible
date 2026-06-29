//! Role-based access control (RBAC) middleware for Axum.
//!
//! Provides permission checking for API endpoints based on user roles and permissions.
//! Integrates with PostgreSQL for permission storage and Redis for caching.

use axum::{
    body::Body,
    extract::{Request, State},
    http::StatusCode,
    middleware::Next,
    response::{IntoResponse, Response},
};
use redis::AsyncCommands;
use serde::{Deserialize, Serialize};
use sqlx::PgPool;
use std::sync::Arc;
use tracing::{debug, error, warn};

/// User role enumeration.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, sqlx::Type)]
#[sqlx(type_name = "user_role", rename_all = "lowercase")]
pub enum Role {
    Admin,
    User,
    Guest,
}

/// Permission definition.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Permission {
    pub resource: String,
    pub action: String,
}

impl Permission {
    pub fn new(resource: impl Into<String>, action: impl Into<String>) -> Self {
        Self {
            resource: resource.into(),
            action: action.into(),
        }
    }
}

/// Shared state for permission middleware.
#[derive(Clone)]
pub struct PermissionState {
    pub db: PgPool,
    pub redis: redis::Client,
}

/// Extension type to store authenticated user information in request.
#[derive(Debug, Clone)]
pub struct AuthUser {
    pub id: i32,
    pub address: String,
    pub role: Role,
}

/// Permission checker service with caching.
pub struct PermissionChecker {
    db: PgPool,
    redis: redis::Client,
    cache_ttl: u64,
}

impl PermissionChecker {
    pub fn new(db: PgPool, redis: redis::Client) -> Self {
        Self {
            db,
            redis,
            cache_ttl: 300, // 5 minutes
        }
    }

    /// Check if a user has a specific permission.
    #[tracing::instrument(skip(self))]
    pub async fn has_permission(
        &self,
        user_id: i32,
        permission: &Permission,
    ) -> Result<bool, crate::error::AppError> {
        let cache_key = format!("perm:{}:{}:{}", user_id, permission.resource, permission.action);

        // Try cache first
        if let Ok(cached) = self.check_cache(&cache_key).await {
            debug!("Permission cache hit for user {}", user_id);
            return Ok(cached);
        }

        // Query database
        let has_perm = self.check_db(user_id, permission).await?;

        // Cache result
        let _ = self.cache_result(&cache_key, has_perm).await;

        Ok(has_perm)
    }

    async fn check_cache(&self, key: &str) -> Result<bool, redis::RedisError> {
        let mut conn = self.redis.get_multiplexed_async_connection().await?;
        let value: Option<String> = conn.get(key).await?;
        value
            .and_then(|v| v.parse().ok())
            .ok_or(redis::RedisError::from((
                redis::ErrorKind::TypeError,
                "Cache miss",
            )))
    }

    async fn cache_result(&self, key: &str, value: bool) -> Result<(), redis::RedisError> {
        let mut conn = self.redis.get_multiplexed_async_connection().await?;
        conn.set_ex(key, value.to_string(), self.cache_ttl).await
    }

    async fn check_db(
        &self,
        user_id: i32,
        permission: &Permission,
    ) -> Result<bool, crate::error::AppError> {
        let result = sqlx::query_scalar::<_, bool>(
            r#"
            SELECT EXISTS(
                SELECT 1 FROM user_permissions up
                JOIN permissions p ON up.permission_id = p.id
                WHERE up.user_id = $1 
                  AND p.resource = $2 
                  AND p.action = $3
            )
            "#,
        )
        .bind(user_id)
        .bind(&permission.resource)
        .bind(&permission.action)
        .fetch_one(&self.db)
        .await?;

        Ok(result)
    }

    /// Get user role from database with caching.
    #[tracing::instrument(skip(self))]
    pub async fn get_user_role(&self, user_id: i32) -> Result<Role, crate::error::AppError> {
        let cache_key = format!("role:{}", user_id);

        // Try cache
        if let Ok(mut conn) = self.redis.get_multiplexed_async_connection().await {
            if let Ok(Some(cached)) = conn.get::<_, Option<String>>(&cache_key).await {
                if let Ok(role) = serde_json::from_str(&cached) {
                    return Ok(role);
                }
            }
        }

        // Query database
        let role = sqlx::query_scalar::<_, Role>("SELECT role FROM users WHERE id = $1")
            .bind(user_id)
            .fetch_one(&self.db)
            .await?;

        // Cache result
        if let Ok(mut conn) = self.redis.get_multiplexed_async_connection().await {
            let _: Result<(), _> = conn
                .set_ex(
                    cache_key,
                    serde_json::to_string(&role).unwrap(),
                    self.cache_ttl,
                )
                .await;
        }

        Ok(role)
    }

    /// Invalidate permission cache for a user.
    pub async fn invalidate_cache(&self, user_id: i32) -> Result<(), redis::RedisError> {
        let mut conn = self.redis.get_multiplexed_async_connection().await?;
        let pattern = format!("perm:{}:*", user_id);
        
        // Delete all permission keys for this user
        let keys: Vec<String> = redis::cmd("KEYS")
            .arg(&pattern)
            .query_async(&mut conn)
            .await?;
        
        if !keys.is_empty() {
            redis::cmd("DEL")
                .arg(&keys)
                .query_async::<()>(&mut conn)
                .await?;
        }

        // Also invalidate role cache
        let role_key = format!("role:{}", user_id);
        let _: () = conn.del(role_key).await?;

        Ok(())
    }
}

/// Middleware to require specific permission.
pub fn require_permission(
    resource: impl Into<String>,
    action: impl Into<String>,
) -> impl Fn(State<Arc<PermissionState>>, Request, Next) -> std::pin::Pin<Box<dyn std::future::Future<Output = Response> + Send>> + Clone {
    let permission = Permission::new(resource, action);
    
    move |State(state): State<Arc<PermissionState>>, request: Request, next: Next| {
        let permission = permission.clone();
        let state = state.clone();
        
        Box::pin(async move {
            // Extract user from request extensions
            let user = match request.extensions().get::<AuthUser>() {
                Some(user) => user.clone(),
                None => {
                    warn!("No authenticated user in request");
                    return (
                        StatusCode::UNAUTHORIZED,
                        "Authentication required",
                    ).into_response();
                }
            };

            let checker = PermissionChecker::new(state.db.clone(), state.redis.clone());

            match checker.has_permission(user.id, &permission).await {
                Ok(true) => {
                    debug!("Permission granted for user {} on {:?}", user.id, permission);
                    next.run(request).await
                }
                Ok(false) => {
                    warn!("Permission denied for user {} on {:?}", user.id, permission);
                    (
                        StatusCode::FORBIDDEN,
                        format!("Insufficient permissions: {} on {}", permission.action, permission.resource),
                    ).into_response()
                }
                Err(e) => {
                    error!("Permission check failed: {:?}", e);
                    (
                        StatusCode::INTERNAL_SERVER_ERROR,
                        "Permission check failed",
                    ).into_response()
                }
            }
        })
    }
}

/// Middleware to require specific role.
pub async fn require_role(
    State(state): State<Arc<PermissionState>>,
    required_role: Role,
    mut request: Request,
    next: Next,
) -> Response {
    let user = match request.extensions().get::<AuthUser>() {
        Some(user) => user.clone(),
        None => {
            return (StatusCode::UNAUTHORIZED, "Authentication required").into_response();
        }
    };

    if user.role == required_role || user.role == Role::Admin {
        next.run(request).await
    } else {
        (
            StatusCode::FORBIDDEN,
            format!("Role {:?} required", required_role),
        )
            .into_response()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use sqlx::PgPool;

    async fn setup_test_db() -> PgPool {
        let pool = PgPool::connect_lazy("postgres://localhost/test").unwrap();
        
        // Create test schema
        sqlx::query(
            r#"
            CREATE TABLE IF NOT EXISTS users (
                id SERIAL PRIMARY KEY,
                address TEXT UNIQUE NOT NULL,
                role user_role NOT NULL DEFAULT 'guest',
                created_at TIMESTAMP WITH TIME ZONE DEFAULT CURRENT_TIMESTAMP
            )
            "#,
        )
        .execute(&pool)
        .await
        .ok();

        sqlx::query(
            r#"
            CREATE TABLE IF NOT EXISTS permissions (
                id SERIAL PRIMARY KEY,
                resource TEXT NOT NULL,
                action TEXT NOT NULL,
                UNIQUE(resource, action)
            )
            "#,
        )
        .execute(&pool)
        .await
        .ok();

        sqlx::query(
            r#"
            CREATE TABLE IF NOT EXISTS user_permissions (
                user_id INTEGER REFERENCES users(id) ON DELETE CASCADE,
                permission_id INTEGER REFERENCES permissions(id) ON DELETE CASCADE,
                PRIMARY KEY (user_id, permission_id)
            )
            "#,
        )
        .execute(&pool)
        .await
        .ok();

        pool
    }

    #[tokio::test]
    async fn test_permission_new() {
        let perm = Permission::new("contracts", "read");
        assert_eq!(perm.resource, "contracts");
        assert_eq!(perm.action, "read");
    }

    #[tokio::test]
    async fn test_role_equality() {
        assert_eq!(Role::Admin, Role::Admin);
        assert_ne!(Role::Admin, Role::User);
    }

    #[test]
    fn test_auth_user_clone() {
        let user = AuthUser {
            id: 1,
            address: "test@example.com".to_string(),
            role: Role::User,
        };
        let cloned = user.clone();
        assert_eq!(user.id, cloned.id);
        assert_eq!(user.role, cloned.role);
    }
}
