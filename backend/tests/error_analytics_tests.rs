//! Integration tests for the Build Error Analytics Dashboard API

use axum::http::StatusCode;
use axum::Router;
use backend::api::handlers::errors::{error_analytics_routes, BuildErrorDetail};
use redis::Client as RedisClient;
use sqlx::{Executor, PgPool};
use tower::ServiceExt; // for `oneshot` method

#[tokio::test]
async fn test_build_error_analytics_empty() {
    let pool = PgPool::connect("postgres://postgres:postgres@localhost/crucible_test")
        .await
        .unwrap();
    let redis = RedisClient::open("redis://127.0.0.1/").unwrap();
    let app = error_analytics_routes(pool.clone(), redis.clone());

    // Ensure DB is empty
    sqlx::query("DELETE FROM build_errors")
        .execute(&pool)
        .await
        .unwrap();

    let response = app
        .oneshot(
            axum::http::Request::builder()
                .uri("/dashboard/build-errors")
                .body(axum::body::Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::OK);
    let body = axum::body::to_bytes(response.into_body(), usize::MAX)
        .await
        .unwrap();
    let analytics: serde_json::Value = serde_json::from_slice(&body).unwrap();
    assert_eq!(analytics["total_errors"], 0);
}

#[tokio::test]
async fn test_build_error_analytics_with_data() {
    let pool = PgPool::connect("postgres://postgres:postgres@localhost/crucible_test")
        .await
        .unwrap();
    let redis = RedisClient::open("redis://127.0.0.1/").unwrap();
    let app = error_analytics_routes(pool.clone(), redis.clone());

    // Insert test data
    sqlx::query("DELETE FROM build_errors")
        .execute(&pool)
        .await
        .unwrap();
    sqlx::query(
        "INSERT INTO build_errors (error_type, message, occurred_at) VALUES ($1, $2, NOW())",
    )
    .bind("TypeA")
    .bind("Error A")
    .execute(&pool)
    .await
    .unwrap();
    sqlx::query(
        "INSERT INTO build_errors (error_type, message, occurred_at) VALUES ($1, $2, NOW())",
    )
    .bind("TypeB")
    .bind("Error B")
    .execute(&pool)
    .await
    .unwrap();

    let response = app
        .oneshot(
            axum::http::Request::builder()
                .uri("/dashboard/build-errors")
                .body(axum::body::Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::OK);
    let body = axum::body::to_bytes(response.into_body(), usize::MAX)
        .await
        .unwrap();
    let analytics: serde_json::Value = serde_json::from_slice(&body).unwrap();
    assert_eq!(analytics["total_errors"], 2);
    assert!(analytics["error_types"].as_array().unwrap().len() > 0);
    assert!(analytics["recent_errors"].as_array().unwrap().len() > 0);
}
