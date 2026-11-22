//! Test utilities module.
//!
//! This module provides mock services and factory functions for creating
//! domain objects in tests.

pub mod assertions;
pub mod client;
pub mod factories;
pub mod fixtures;
pub mod mocks;

#[cfg(test)]
pub mod tests;

pub use factories::*;

use axum::Router;
use sqlx::{postgres::PgPoolOptions, PgPool};
use std::env;
use uuid::Uuid;

/// Contains the isolated test state, database connections, and handles cleanup on Drop.
pub struct TestContext {
    pub app: Router,
    pub db: PgPool,
    pub redis: redis::Client,
    pub schema_name: String,
    // Store root pool to drop the schema later
    root_db: PgPool,
}

impl Drop for TestContext {
    fn drop(&mut self) {
        let schema_name = self.schema_name.clone();
        let root_db = self.root_db.clone();

        // Spawn a background task to forcefully drop the isolated schema once the test completes
        tokio::spawn(async move {
            let drop_query = format!("DROP SCHEMA IF EXISTS {} CASCADE;", schema_name);
            let _ = sqlx::query(&drop_query).execute(&root_db).await;
        });
    }
}

/// Spins up an isolated test context, executing migrations against a unique PostgreSQL schema.
pub async fn setup() -> TestContext {
    // 1. Resolve environment connections
    let db_url = env::var("TEST_DATABASE_URL")
        .unwrap_or_else(|_| "postgres://postgres:password@localhost/crucible_test".to_string());

    let redis_url =
        env::var("TEST_REDIS_URL").unwrap_or_else(|_| "redis://127.0.0.1/1".to_string()); // DB 1 for tests

    // 2. Generate isolated schema name
    let schema_name = format!("test_{}", Uuid::new_v4().to_string().replace('-', ""));

    // 3. Connect to root database and create schema
    let root_db = PgPoolOptions::new()
        .max_connections(2)
        .connect(&db_url)
        .await
        .expect("Failed to connect to root test database");

    sqlx::query(&format!("CREATE SCHEMA IF NOT EXISTS {};", schema_name))
        .execute(&root_db)
        .await
        .expect("Failed to create isolated schema");

    // 4. Create isolated pool mapped to the new search_path
    let isolated_db_url = format!("{}?options=-c%20search_path%3D{}", db_url, schema_name);
    let db = PgPoolOptions::new()
        .max_connections(5)
        .connect(&isolated_db_url)
        .await
        .expect("Failed to connect to isolated schema");

    // 5. Run migrations on the isolated schema
    sqlx::migrate!("./migrations")
        .run(&db)
        .await
        .expect("Failed to run migrations on isolated schema");

    // 6. Connect to Redis
    let redis = redis::Client::open(redis_url).expect("Failed to parse Redis URL");

    // 7. Build Axum Router (Assuming backend::api::app() exists and accepts dependencies)
    // Note: Adjust according to actual router constructor signature
    let app = Router::new();

    TestContext {
        app,
        db,
        redis,
        schema_name,
        root_db,
    }
}
pub mod db;

pub use db::*;
