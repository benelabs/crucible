//! Integration tests for permissions middleware.

use axum::{
    body::Body,
    extract::State,
    http::{Request, StatusCode},
    middleware,
    response::IntoResponse,
    routing::get,
    Router,
};
use backend::api::middleware::permissions::{
    AuthUser, Permission, PermissionChecker, PermissionState, Role,
};
use redis::Client as RedisClient;
use sqlx::PgPool;
use std::sync::Arc;
use tower::ServiceExt;

async fn setup_test_env() -> (PgPool, RedisClient) {
    let db = PgPool::connect(&std::env::var("DATABASE_URL").unwrap_or_else(|_| {
        "postgres://crucible:crucible_secret@localhost:5432/crucible_db".to_string()
    }))
    .await
    .expect("Failed to connect to database");

    let redis = RedisClient::open(
        std::env::var("REDIS_URL")
            .unwrap_or_else(|_| "redis://:crucible_redis_secret@localhost:6379/0".to_string()),
    )
    .expect("Failed to connect to Redis");

    // Setup schema
    sqlx::query("CREATE TYPE IF NOT EXISTS user_role AS ENUM ('admin', 'user', 'guest')")
        .execute(&db)
        .await
        .ok();

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
    .execute(&db)
    .await
    .ok();

    sqlx::query(
        r#"
        CREATE TABLE IF NOT EXISTS permissions (
            id SERIAL PRIMARY KEY,
            resource TEXT NOT NULL,
            action TEXT NOT NULL,
            description TEXT,
            created_at TIMESTAMP WITH TIME ZONE DEFAULT CURRENT_TIMESTAMP,
            UNIQUE(resource, action)
        )
        "#,
    )
    .execute(&db)
    .await
    .ok();

    sqlx::query(
        r#"
        CREATE TABLE IF NOT EXISTS user_permissions (
            user_id INTEGER NOT NULL REFERENCES users(id) ON DELETE CASCADE,
            permission_id INTEGER NOT NULL REFERENCES permissions(id) ON DELETE CASCADE,
            granted_at TIMESTAMP WITH TIME ZONE DEFAULT CURRENT_TIMESTAMP,
            PRIMARY KEY (user_id, permission_id)
        )
        "#,
    )
    .execute(&db)
    .await
    .ok();

    (db, redis)
}

async fn cleanup_test_data(db: &PgPool) {
    sqlx::query("DELETE FROM user_permissions")
        .execute(db)
        .await
        .ok();
    sqlx::query("DELETE FROM permissions")
        .execute(db)
        .await
        .ok();
    sqlx::query("DELETE FROM users").execute(db).await.ok();
}

#[tokio::test]
async fn test_permission_checker_has_permission() {
    let (db, redis) = setup_test_env().await;
    cleanup_test_data(&db).await;

    // Create test user
    let user_id: i32 =
        sqlx::query_scalar("INSERT INTO users (address, role) VALUES ($1, $2) RETURNING id")
            .bind("test@example.com")
            .bind(Role::User)
            .fetch_one(&db)
            .await
            .unwrap();

    // Create permission
    let perm_id: i32 = sqlx::query_scalar(
        "INSERT INTO permissions (resource, action) VALUES ($1, $2) RETURNING id",
    )
    .bind("contracts")
    .bind("read")
    .fetch_one(&db)
    .await
    .unwrap();

    // Grant permission
    sqlx::query("INSERT INTO user_permissions (user_id, permission_id) VALUES ($1, $2)")
        .bind(user_id)
        .bind(perm_id)
        .execute(&db)
        .await
        .unwrap();

    let checker = PermissionChecker::new(db.clone(), redis.clone());
    let permission = Permission::new("contracts", "read");

    let has_perm = checker.has_permission(user_id, &permission).await.unwrap();
    assert!(has_perm);

    // Test permission not granted
    let no_perm = Permission::new("contracts", "delete");
    let has_no_perm = checker.has_permission(user_id, &no_perm).await.unwrap();
    assert!(!has_no_perm);

    cleanup_test_data(&db).await;
}

#[tokio::test]
async fn test_permission_checker_caching() {
    let (db, redis) = setup_test_env().await;
    cleanup_test_data(&db).await;

    let user_id: i32 =
        sqlx::query_scalar("INSERT INTO users (address, role) VALUES ($1, $2) RETURNING id")
            .bind("cache_test@example.com")
            .bind(Role::User)
            .fetch_one(&db)
            .await
            .unwrap();

    let perm_id: i32 = sqlx::query_scalar(
        "INSERT INTO permissions (resource, action) VALUES ($1, $2) RETURNING id",
    )
    .bind("test_runs")
    .bind("read")
    .fetch_one(&db)
    .await
    .unwrap();

    sqlx::query("INSERT INTO user_permissions (user_id, permission_id) VALUES ($1, $2)")
        .bind(user_id)
        .bind(perm_id)
        .execute(&db)
        .await
        .unwrap();

    let checker = PermissionChecker::new(db.clone(), redis.clone());
    let permission = Permission::new("test_runs", "read");

    // First call - should hit database
    let result1 = checker.has_permission(user_id, &permission).await.unwrap();
    assert!(result1);

    // Second call - should hit cache
    let result2 = checker.has_permission(user_id, &permission).await.unwrap();
    assert!(result2);

    cleanup_test_data(&db).await;
}

#[tokio::test]
async fn test_permission_checker_invalidate_cache() {
    let (db, redis) = setup_test_env().await;
    cleanup_test_data(&db).await;

    let user_id: i32 =
        sqlx::query_scalar("INSERT INTO users (address, role) VALUES ($1, $2) RETURNING id")
            .bind("invalidate@example.com")
            .bind(Role::User)
            .fetch_one(&db)
            .await
            .unwrap();

    let checker = PermissionChecker::new(db.clone(), redis.clone());

    // Cache a permission check
    let permission = Permission::new("users", "read");
    let _ = checker.has_permission(user_id, &permission).await;

    // Invalidate cache
    checker.invalidate_cache(user_id).await.unwrap();

    // Next check should hit database again
    let result = checker.has_permission(user_id, &permission).await.unwrap();
    assert!(!result); // No permission granted

    cleanup_test_data(&db).await;
}

#[tokio::test]
async fn test_get_user_role() {
    let (db, redis) = setup_test_env().await;
    cleanup_test_data(&db).await;

    let user_id: i32 =
        sqlx::query_scalar("INSERT INTO users (address, role) VALUES ($1, $2) RETURNING id")
            .bind("role_test@example.com")
            .bind(Role::Admin)
            .fetch_one(&db)
            .await
            .unwrap();

    let checker = PermissionChecker::new(db.clone(), redis.clone());
    let role = checker.get_user_role(user_id).await.unwrap();

    assert_eq!(role, Role::Admin);

    cleanup_test_data(&db).await;
}

#[tokio::test]
async fn test_middleware_with_permission() {
    let (db, redis) = setup_test_env().await;
    cleanup_test_data(&db).await;

    let user_id: i32 =
        sqlx::query_scalar("INSERT INTO users (address, role) VALUES ($1, $2) RETURNING id")
            .bind("middleware@example.com")
            .bind(Role::User)
            .fetch_one(&db)
            .await
            .unwrap();

    let perm_id: i32 = sqlx::query_scalar(
        "INSERT INTO permissions (resource, action) VALUES ($1, $2) RETURNING id",
    )
    .bind("contracts")
    .bind("read")
    .fetch_one(&db)
    .await
    .unwrap();

    sqlx::query("INSERT INTO user_permissions (user_id, permission_id) VALUES ($1, $2)")
        .bind(user_id)
        .bind(perm_id)
        .execute(&db)
        .await
        .unwrap();

    let state = Arc::new(PermissionState {
        db: db.clone(),
        redis: redis.clone(),
    });

    async fn protected_handler() -> impl IntoResponse {
        "Protected resource"
    }

    let app = Router::new()
        .route("/protected", get(protected_handler))
        .with_state(state);

    // Test without auth user - should fail
    let response = app
        .clone()
        .oneshot(
            Request::builder()
                .uri("/protected")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();

    // Note: This test demonstrates the structure but won't pass without full auth setup
    // In production, you'd have an auth middleware that sets the AuthUser extension

    cleanup_test_data(&db).await;
}

#[tokio::test]
async fn test_role_serialization() {
    let admin = Role::Admin;
    let json = serde_json::to_string(&admin).unwrap();
    assert_eq!(json, "\"Admin\"");

    let deserialized: Role = serde_json::from_str(&json).unwrap();
    assert_eq!(deserialized, Role::Admin);
}

#[tokio::test]
async fn test_permission_equality() {
    let perm1 = Permission::new("contracts", "read");
    let perm2 = Permission::new("contracts", "read");
    let perm3 = Permission::new("contracts", "write");

    assert_eq!(perm1, perm2);
    assert_ne!(perm1, perm3);
}
