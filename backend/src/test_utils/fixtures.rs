//! API TEST CLIENT APPROACH: Option A — builder pattern per request
//! Rationale: Standardizing fixture creation ensures test independence and clean dependency injection.

use crate::test_utils::client::ApiTestClient;
use axum::Router;
use redis::AsyncCommands;
use sqlx::{PgPool, Row};
use uuid::Uuid;

/// Contains external dependencies used to seed and teardown test data.
pub struct Fixtures {
    pub db: PgPool,
    pub redis: redis::Client,
}

/// Optional overrides to apply when seeding a user.
#[derive(Default)]
pub struct UserOverrides {
    pub email: Option<String>,
    pub password: Option<String>,
    pub is_admin: Option<bool>,
}

/// Represents a seeded user in the database.
#[derive(Debug, Clone, serde::Deserialize)]
pub struct User {
    pub id: Uuid,
    pub email: String,
    pub is_admin: bool,
}

/// Inserts a test user into the database and returns the populated struct.
pub async fn seed_user(pool: &PgPool, overrides: Option<UserOverrides>) -> User {
    let ov = overrides.unwrap_or_default();
    let id = Uuid::new_v4();
    let email = ov
        .email
        .unwrap_or_else(|| format!("test-{}@example.com", id));
    let password = ov.password.unwrap_or_else(|| "password123".to_string());
    let is_admin = ov.is_admin.unwrap_or(false);

    // Note: Assuming a standard Users table schema exists. Adjust if different.
    let row = sqlx::query(
        r#"
        INSERT INTO users (id, email, password_hash, is_admin)
        VALUES ($1, $2, $3, $4)
        RETURNING id, email, is_admin
        "#,
    )
    .bind(id)
    .bind(&email)
    .bind(&password)
    .bind(is_admin)
    .fetch_one(pool)
    .await
    .expect("Failed to seed user");

    User {
        id: row.get("id"),
        email: row.get("email"),
        is_admin: row.get("is_admin"),
    }
}

/// Seeds a user, conceptually authenticates them, and returns an ApiTestClient pre-configured with a valid Bearer token.
pub async fn seed_authenticated_client(app: Router, pool: &PgPool) -> (ApiTestClient, String) {
    let _user = seed_user(pool, None).await;
    // Note: In a real implementation, you would generate a valid JWT here matching your auth service format.
    let token = "mock.jwt.token".to_string();
    let client = ApiTestClient::with_db(app, pool.clone());

    (client, token)
}

/// Truncates all test tables in the correct dependency order.
///
/// Note: With the isolated schema strategy, truncation is rarely necessary, but
/// provided here for completeness if running within a shared schema context.
pub async fn cleanup(pool: &PgPool) {
    sqlx::query("TRUNCATE TABLE users, jobs, contracts CASCADE;")
        .execute(pool)
        .await
        .expect("Failed to cleanup database tables");
}

/// Clears all keys in the connected Redis instance.
pub async fn flush_redis(client: &redis::Client) {
    let mut conn = client
        .get_multiplexed_async_connection()
        .await
        .expect("Failed to connect to redis for flush");
    let _: () = redis::cmd("FLUSHDB")
        .query_async(&mut conn)
        .await
        .expect("Failed to flush redis");
}
