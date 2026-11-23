//! Role-based access control (RBAC) middleware for Axum.
//!
//! Provides permission checking for API endpoints based on user roles and permissions.
//! Integrates with PostgreSQL for permission storage and Redis for caching.

use axum::{
    extract::{Request, State},
    http::StatusCode,
    middleware::Next,
    response::Response,
};
use redis::AsyncCommands;
use serde::{Deserialize, Serialize};
use sqlx::PgPool;
use std::sync::Arc;
use tracing::{debug, error, warn};

/// User role enumeration with granular enterprise capabilities.
#[derive(
    Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize, sqlx::Type,
)]
#[sqlx(type_name = "user_role", rename_all = "lowercase")]
pub enum Role {
    Admin,
    Developer,
    Auditor,
    Viewer,
    User,
    Guest,
}

/// Granular permission bitmask for high-performance permission checks.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
pub struct PermissionBitmask(pub u32);

impl PermissionBitmask {
    pub const NONE: Self = Self(0);
    pub const READ: Self = Self(1 << 0);
    pub const WRITE: Self = Self(1 << 1);
    pub const EXECUTE: Self = Self(1 << 2);
    pub const DELETE: Self = Self(1 << 3);
    pub const ADMIN: Self = Self(1 << 4);
    pub const AUDIT: Self = Self(1 << 5);
    pub const ALL: Self = Self(0xFFFFFFFF);

    #[inline]
    pub const fn contains(&self, other: Self) -> bool {
        (self.0 & other.0) == other.0
    }

    #[inline]
    pub const fn has_any(&self, other: Self) -> bool {
        (self.0 & other.0) != 0
    }

    #[inline]
    pub fn add(&mut self, other: Self) {
        self.0 |= other.0;
    }

    #[inline]
    pub fn remove(&mut self, other: Self) {
        self.0 &= !other.0;
    }
}

impl Default for PermissionBitmask {
    fn default() -> Self {
        Self::NONE
    }
}

impl std::ops::BitOr for PermissionBitmask {
    type Output = Self;
    fn bitor(self, rhs: Self) -> Self::Output {
        Self(self.0 | rhs.0)
    }
}

impl std::ops::BitAnd for PermissionBitmask {
    type Output = Self;
    fn bitand(self, rhs: Self) -> Self::Output {
        Self(self.0 & rhs.0)
    }
}

impl Role {
    /// Return the default permission bitmask associated with this role.
    pub fn default_permissions(&self) -> PermissionBitmask {
        match self {
            Role::Admin => PermissionBitmask::ALL,
            Role::Developer => {
                PermissionBitmask::READ | PermissionBitmask::WRITE | PermissionBitmask::EXECUTE
            }
            Role::Auditor => PermissionBitmask::READ | PermissionBitmask::AUDIT,
            Role::Viewer => PermissionBitmask::READ,
            Role::User => PermissionBitmask::READ | PermissionBitmask::EXECUTE,
            Role::Guest => PermissionBitmask::READ,
        }
    }
}

/// Organization model for multi-tenant enterprise isolation.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Organization {
    pub id: i32,
    pub name: String,
    pub slug: String,
}

/// Scoped workspace model belonging to an organization.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Workspace {
    pub id: i32,
    pub organization_id: i32,
    pub name: String,
    pub slug: String,
}

/// Role permission mapping model.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct RolePermission {
    pub role: Role,
    pub resource: String,
    pub action: String,
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
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct AuthUser {
    pub id: i32,
    pub address: String,
    pub role: Role,
    pub organization_id: Option<i32>,
    pub permissions: Option<PermissionBitmask>,
}

impl AuthUser {
    /// Create a new authenticated user with default role permissions.
    pub fn new(id: i32, address: impl Into<String>, role: Role) -> Self {
        Self {
            id,
            address: address.into(),
            role,
            organization_id: None,
            permissions: Some(role.default_permissions()),
        }
    }

    /// Associate an organization / tenant ID with this user.
    pub fn with_org(mut self, organization_id: i32) -> Self {
        self.organization_id = Some(organization_id);
        self
    }

    /// Configure specific permission bitmask overrides.
    pub fn with_permissions(mut self, permissions: PermissionBitmask) -> Self {
        self.permissions = Some(permissions);
        self
    }

    /// Return effective permission bitmask (explicit permissions or fallback to role defaults).
    pub fn effective_permissions(&self) -> PermissionBitmask {
        self.permissions
            .unwrap_or_else(|| self.role.default_permissions())
    }

    /// Check if user belongs to the designated organization.
    pub fn belongs_to_org(&self, org_id: i32) -> bool {
        self.organization_id == Some(org_id)
    }
}

/// Detailed error returned when a permission or tenant isolation check fails.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum PermissionCheckError {
    Unauthenticated,
    InsufficientPermissions {
        required: PermissionBitmask,
        actual: PermissionBitmask,
    },
    TenantIsolationViolation {
        user_org: Option<i32>,
        target_org: i32,
    },
}

impl std::fmt::Display for PermissionCheckError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Unauthenticated => write!(f, "Authentication required"),
            Self::InsufficientPermissions { required, actual } => {
                write!(
                    f,
                    "Insufficient permissions: required bitmask {:#x}, actual bitmask {:#x}",
                    required.0, actual.0
                )
            }
            Self::TenantIsolationViolation {
                user_org,
                target_org,
            } => {
                write!(
                    f,
                    "Cross-tenant access forbidden: user organization {:?} cannot access organization {}",
                    user_org, target_org
                )
            }
        }
    }
}

impl std::error::Error for PermissionCheckError {}

/// Map an action string to its canonical permission bitmask flag.
pub fn action_to_bitmask(action: &str) -> PermissionBitmask {
    match action.to_ascii_lowercase().as_str() {
        "read" | "get" | "list" | "view" | "describe" => PermissionBitmask::READ,
        "write" | "post" | "put" | "patch" | "create" | "update" => PermissionBitmask::WRITE,
        "delete" | "destroy" | "remove" => PermissionBitmask::DELETE,
        "execute" | "run" | "invoke" | "deploy" => PermissionBitmask::EXECUTE,
        "admin" | "manage" | "grant" | "revoke" => PermissionBitmask::ADMIN,
        "audit" | "compliance" | "log" => PermissionBitmask::AUDIT,
        _ => PermissionBitmask::READ,
    }
}

/// Extract target organization / tenant ID from request URI or headers.
///
/// Supports:
/// - Header `X-Organization-ID` or `X-Tenant-ID`
/// - Path segments `/organizations/{id}/`, `/orgs/{id}/`, `/tenants/{id}/`, `/workspaces/{id}/`
/// - Query string `?org_id={id}`, `?organization_id={id}`, `?tenant_id={id}`
pub fn extract_target_tenant_id(uri: &str, headers: Option<&axum::http::HeaderMap>) -> Option<i32> {
    // 1. Inspect HTTP headers for tenant context
    if let Some(headers) = headers {
        if let Some(val) = headers
            .get("x-organization-id")
            .or_else(|| headers.get("x-tenant-id"))
        {
            if let Ok(s) = val.to_str() {
                if let Ok(id) = s.trim().parse::<i32>() {
                    return Some(id);
                }
            }
        }
    }

    // 2. Parse URI path segments
    let path = uri.split('?').next().unwrap_or(uri);
    let segments: Vec<&str> = path.split('/').filter(|s| !s.is_empty()).collect();
    for (i, segment) in segments.iter().enumerate() {
        let seg_lower = segment.to_ascii_lowercase();
        if (seg_lower == "organizations"
            || seg_lower == "orgs"
            || seg_lower == "tenants"
            || seg_lower == "org")
            && i + 1 < segments.len()
        {
            if let Ok(id) = segments[i + 1].parse::<i32>() {
                return Some(id);
            }
        }
    }

    // 3. Parse query string parameters
    if let Some(query) = uri.split('?').nth(1) {
        for pair in query.split('&') {
            let mut parts = pair.split('=');
            if let (Some(k), Some(v)) = (parts.next(), parts.next()) {
                let k_lower = k.to_ascii_lowercase();
                if k_lower == "org_id" || k_lower == "organization_id" || k_lower == "tenant_id" {
                    if let Ok(id) = v.parse::<i32>() {
                        return Some(id);
                    }
                }
            }
        }
    }

    None
}

/// Cross-references authenticated user permissions against required bitmask and enforces tenant isolation.
///
/// Ensures:
/// 1. The user possesses the required permission bitmask for the operation (Admins bypass bitmask check).
/// 2. If the resource URI or request headers designate a target organization/tenant ID,
///    non-admin users are strictly restricted to accessing resources belonging to their own `organization_id`.
pub fn check_permission(
    user: &AuthUser,
    required_bitmask: PermissionBitmask,
    resource_uri: &str,
    headers: Option<&axum::http::HeaderMap>,
) -> Result<(), PermissionCheckError> {
    // 1. Bitmask verification
    let user_perms = user.effective_permissions();
    if user.role != Role::Admin && !user_perms.contains(required_bitmask) {
        warn!(
            user_id = user.id,
            role = ?user.role,
            required = required_bitmask.0,
            actual = user_perms.0,
            "Insufficient permission bitmask"
        );
        return Err(PermissionCheckError::InsufficientPermissions {
            required: required_bitmask,
            actual: user_perms,
        });
    }

    // 2. Tenant isolation verification against target organization/tenant ID in resource URI / headers
    if let Some(target_org_id) = extract_target_tenant_id(resource_uri, headers) {
        // Role::Admin possesses cross-tenant authority across all organizations.
        // All other roles must strictly match their registered organization_id.
        if user.role != Role::Admin {
            match user.organization_id {
                Some(user_org) if user_org == target_org_id => {
                    // Valid tenant match
                }
                user_org => {
                    warn!(
                        user_id = user.id,
                        user_org = ?user_org,
                        target_org = target_org_id,
                        resource_uri = resource_uri,
                        "Tenant isolation bypass blocked: cross-tenant access denied"
                    );
                    return Err(PermissionCheckError::TenantIsolationViolation {
                        user_org,
                        target_org: target_org_id,
                    });
                }
            }
        }
    }

    Ok(())
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
        let cache_key = format!(
            "perm:{}:{}:{}",
            user_id, permission.resource, permission.action
        );

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
) -> impl Fn(
    State<Arc<PermissionState>>,
    Request,
    Next,
) -> std::pin::Pin<Box<dyn std::future::Future<Output = Response> + Send>>
       + Clone {
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
                    return crate::api::errors::make_error_response(
                        StatusCode::UNAUTHORIZED,
                        "unauthorized",
                        "Authentication required",
                    );
                }
            };

            // Enforce tenant ownership validation alongside permission bitmask checks
            let required_bitmask = action_to_bitmask(&permission.action);
            let resource_uri = request
                .uri()
                .path_and_query()
                .map(|pq| pq.as_str())
                .unwrap_or_else(|| request.uri().path());

            if let Err(check_err) = check_permission(
                &user,
                required_bitmask,
                resource_uri,
                Some(request.headers()),
            ) {
                warn!(
                    user_id = user.id,
                    error = %check_err,
                    resource = %permission.resource,
                    action = %permission.action,
                    "Access denied by check_permission"
                );
                return crate::api::errors::make_error_response(
                    StatusCode::FORBIDDEN,
                    "forbidden",
                    &check_err.to_string(),
                );
            }

            let checker = PermissionChecker::new(state.db.clone(), state.redis.clone());

            match checker.has_permission(user.id, &permission).await {
                Ok(true) => {
                    debug!(
                        "Permission granted for user {} on {:?}",
                        user.id, permission
                    );
                    next.run(request).await
                }
                Ok(false) => {
                    warn!("Permission denied for user {} on {:?}", user.id, permission);
                    crate::api::errors::make_error_response(
                        StatusCode::FORBIDDEN,
                        "forbidden",
                        &format!(
                            "Insufficient permissions: {} on {}",
                            permission.action, permission.resource
                        ),
                    )
                }
                Err(e) => {
                    error!("Permission check failed: {:?}", e);
                    crate::api::errors::make_error_response(
                        StatusCode::INTERNAL_SERVER_ERROR,
                        "internal_error",
                        "Permission check failed",
                    )
                }
            }
        })
    }
}

/// Middleware to require specific role.
pub async fn require_role(
    State(_state): State<Arc<PermissionState>>,
    required_role: Role,
    request: Request,
    next: Next,
) -> Response {
    let user = match request.extensions().get::<AuthUser>() {
        Some(user) => user.clone(),
        None => {
            return crate::api::errors::make_error_response(
                StatusCode::UNAUTHORIZED,
                "unauthorized",
                "Authentication required",
            );
        }
    };

    if user.role == required_role || user.role == Role::Admin {
        next.run(request).await
    } else {
        crate::api::errors::make_error_response(
            StatusCode::FORBIDDEN,
            "forbidden",
            &format!("Role {:?} required", required_role),
        )
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use sqlx::PgPool;

    #[allow(dead_code)]
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
        assert_ne!(Role::Developer, Role::Auditor);
        assert_ne!(Role::Auditor, Role::Viewer);
    }

    #[test]
    fn test_permission_bitmask_operations() {
        let mut mask = PermissionBitmask::READ | PermissionBitmask::WRITE;
        assert!(mask.contains(PermissionBitmask::READ));
        assert!(mask.contains(PermissionBitmask::WRITE));
        assert!(!mask.contains(PermissionBitmask::EXECUTE));
        assert!(!mask.contains(PermissionBitmask::ADMIN));

        mask.add(PermissionBitmask::EXECUTE);
        assert!(mask.contains(PermissionBitmask::EXECUTE));

        mask.remove(PermissionBitmask::WRITE);
        assert!(!mask.contains(PermissionBitmask::WRITE));
        assert!(mask.contains(PermissionBitmask::READ));
        assert!(mask.contains(PermissionBitmask::EXECUTE));
    }

    #[test]
    fn test_role_default_bitmask_permissions() {
        assert!(Role::Admin
            .default_permissions()
            .contains(PermissionBitmask::ALL));
        assert!(Role::Developer
            .default_permissions()
            .contains(PermissionBitmask::READ | PermissionBitmask::WRITE));
        assert!(!Role::Viewer
            .default_permissions()
            .contains(PermissionBitmask::WRITE));
        assert!(Role::Auditor
            .default_permissions()
            .contains(PermissionBitmask::AUDIT));
    }

    #[test]
    fn test_extract_target_tenant_id() {
        // Path segment
        assert_eq!(
            extract_target_tenant_id("/api/v1/organizations/42/contracts", None),
            Some(42)
        );
        assert_eq!(
            extract_target_tenant_id("/api/v1/orgs/101/workspaces", None),
            Some(101)
        );
        assert_eq!(
            extract_target_tenant_id("/tenants/77/dashboard", None),
            Some(77)
        );
        assert_eq!(extract_target_tenant_id("/contracts/analyze", None), None);

        // Query param
        assert_eq!(
            extract_target_tenant_id("/api/v1/contracts?org_id=55", None),
            Some(55)
        );
        assert_eq!(
            extract_target_tenant_id("/api/v1/contracts?tenant_id=88", None),
            Some(88)
        );

        // Header
        let mut headers = axum::http::HeaderMap::new();
        headers.insert("x-organization-id", "99".parse().unwrap());
        assert_eq!(
            extract_target_tenant_id("/api/v1/contracts", Some(&headers)),
            Some(99)
        );
    }

    #[test]
    fn test_auth_user_clone() {
        let user = AuthUser {
            id: 1,
            address: "test@example.com".to_string(),
            role: Role::Developer,
            organization_id: Some(10),
            permissions: Some(Role::Developer.default_permissions()),
        };
        let cloned = user.clone();
        assert_eq!(user.id, cloned.id);
        assert_eq!(user.role, cloned.role);
        assert_eq!(user.organization_id, cloned.organization_id);
    }

    #[tokio::test]
    async fn test_cross_organization_permission_denial() {
        let user_org_a = AuthUser {
            id: 101,
            address: "dev_org_a@crucible.io".to_string(),
            role: Role::Developer,
            organization_id: Some(1),
            permissions: Some(Role::Developer.default_permissions()),
        };
        let target_workspace_org_b_uri = "/api/v1/organizations/999/workspaces/42";

        // User from Org A attempting access to Org B workspace should be denied
        let denial = check_permission(
            &user_org_a,
            PermissionBitmask::READ,
            target_workspace_org_b_uri,
            None,
        );
        assert_eq!(
            denial,
            Err(PermissionCheckError::TenantIsolationViolation {
                user_org: Some(1),
                target_org: 999,
            })
        );
    }

    #[tokio::test]
    async fn test_same_organization_permission_success() {
        let user_org_a = AuthUser {
            id: 101,
            address: "dev_org_a@crucible.io".to_string(),
            role: Role::Developer,
            organization_id: Some(1),
            permissions: Some(Role::Developer.default_permissions()),
        };
        let target_uri_org_a = "/api/v1/organizations/1/workspaces/42";

        let allowed =
            check_permission(&user_org_a, PermissionBitmask::READ, target_uri_org_a, None);
        assert!(allowed.is_ok());
    }

    #[tokio::test]
    async fn test_admin_cross_tenant_bypass() {
        let admin = AuthUser::new(1, "admin@crucible.io", Role::Admin).with_org(1);
        let target_uri_org_b = "/api/v1/organizations/999/workspaces/42";

        // Admin has superuser authority across all tenants
        let admin_access = check_permission(&admin, PermissionBitmask::ALL, target_uri_org_b, None);
        assert!(admin_access.is_ok());
    }

    #[tokio::test]
    async fn test_insufficient_bitmask_permission_denial() {
        let viewer = AuthUser::new(2, "viewer@crucible.io", Role::Viewer).with_org(1);
        let uri = "/api/v1/organizations/1/contracts/deploy";

        // Viewer lacks WRITE/EXECUTE permissions
        let result = check_permission(&viewer, PermissionBitmask::WRITE, uri, None);
        assert!(matches!(
            result,
            Err(PermissionCheckError::InsufficientPermissions { .. })
        ));
    }

    #[tokio::test]
    async fn test_unscoped_resource_permission_success() {
        let developer = AuthUser::new(3, "dev@crucible.io", Role::Developer);
        let uri = "/api/v1/health";

        // Unscoped global resource should succeed for authorized role
        let result = check_permission(&developer, PermissionBitmask::READ, uri, None);
        assert!(result.is_ok());
    }

    #[tokio::test]
    async fn test_header_based_tenant_isolation() {
        let user = AuthUser::new(10, "user@crucible.io", Role::User).with_org(5);
        let mut headers = axum::http::HeaderMap::new();
        headers.insert("x-organization-id", "9".parse().unwrap());

        // Header claims org 9 while user belongs to org 5 -> reject
        let result = check_permission(
            &user,
            PermissionBitmask::READ,
            "/api/v1/resource",
            Some(&headers),
        );
        assert_eq!(
            result,
            Err(PermissionCheckError::TenantIsolationViolation {
                user_org: Some(5),
                target_org: 9,
            })
        );
    }
}
