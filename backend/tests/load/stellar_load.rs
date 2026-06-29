//! Concurrent load tests for the `GET /.well-known/stellar.toml` endpoint.
//!
//! These tests verify that the Stellar SEP-1 handler remains stable and
//! correct under concurrent load. The handler is stateless so no mock
//! infrastructure is required.
//!
//! # Running
//!
//! ```bash
//! cargo test -p backend --test load_tests load::stellar_load -- --nocapture
//! ```

use std::time::Instant;

use axum::http::StatusCode;
use axum::{body::to_bytes, routing::get, Router};
use hyper::Request;
use tower::ServiceExt;

use backend::api::handlers::stellar::get_stellar_toml;

use crate::load::framework::{assert_load_result, LoadConfig, LoadResult};

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

/// Build a test router wired to the Stellar TOML handler.
fn build_app() -> Router {
    Router::new().route("/.well-known/stellar.toml", get(get_stellar_toml))
}

/// Run a full load test using the framework and return the [`LoadResult`].
async fn run_framework_load(concurrency: usize, requests_per_task: usize) -> LoadResult {
    use crate::load::framework::run_load;

    let cfg = LoadConfig::new(concurrency, requests_per_task);
    run_load(cfg, || async {
        let app = build_app();
        let start = Instant::now();
        let resp = app
            .oneshot(
                Request::builder()
                    .uri("/.well-known/stellar.toml")
                    .body(axum::body::Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        (resp.status(), start.elapsed())
    })
    .await
}

// ---------------------------------------------------------------------------
// Basic correctness
// ---------------------------------------------------------------------------

/// Handler returns 200 OK.
#[tokio::test]
async fn test_stellar_toml_returns_200() {
    let app = build_app();
    let resp = app
        .oneshot(
            Request::builder()
                .uri("/.well-known/stellar.toml")
                .body(axum::body::Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();

    assert_eq!(resp.status(), StatusCode::OK);
}

/// Response includes the required `Access-Control-Allow-Origin: *` header (SEP-1).
#[tokio::test]
async fn test_stellar_toml_cors_header() {
    let app = build_app();
    let resp = app
        .oneshot(
            Request::builder()
                .uri("/.well-known/stellar.toml")
                .body(axum::body::Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();

    let cors = resp
        .headers()
        .get("access-control-allow-origin")
        .expect("Access-Control-Allow-Origin header must be present");
    assert_eq!(cors, "*");
}

/// Response `Content-Type` is `text/plain`.
#[tokio::test]
async fn test_stellar_toml_content_type() {
    let app = build_app();
    let resp = app
        .oneshot(
            Request::builder()
                .uri("/.well-known/stellar.toml")
                .body(axum::body::Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();

    let ct = resp
        .headers()
        .get("content-type")
        .expect("Content-Type header must be present");
    assert!(
        ct.to_str().unwrap().contains("text/plain"),
        "Content-Type must be text/plain, got: {:?}",
        ct
    );
}

/// Response body contains the required TOML fields.
#[tokio::test]
async fn test_stellar_toml_body_content() {
    let app = build_app();
    let resp = app
        .oneshot(
            Request::builder()
                .uri("/.well-known/stellar.toml")
                .body(axum::body::Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();

    let bytes = to_bytes(resp.into_body(), usize::MAX).await.unwrap();
    let body = std::str::from_utf8(&bytes).unwrap();

    assert!(body.contains("VERSION"), "body must contain VERSION");
    assert!(
        body.contains("NETWORK_PASSPHRASE"),
        "body must contain NETWORK_PASSPHRASE"
    );
    assert!(body.contains("ACCOUNTS"), "body must contain ACCOUNTS");
    assert!(body.contains("CURRENCIES"), "body must contain CURRENCIES");
}

/// Response body contains the USDC currency entry.
#[tokio::test]
async fn test_stellar_toml_contains_usdc() {
    let app = build_app();
    let resp = app
        .oneshot(
            Request::builder()
                .uri("/.well-known/stellar.toml")
                .body(axum::body::Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();

    let bytes = to_bytes(resp.into_body(), usize::MAX).await.unwrap();
    let body = std::str::from_utf8(&bytes).unwrap();

    assert!(body.contains("USDC"), "body must contain USDC currency");
}

/// Response body is non-empty.
#[tokio::test]
async fn test_stellar_toml_non_empty_body() {
    let app = build_app();
    let resp = app
        .oneshot(
            Request::builder()
                .uri("/.well-known/stellar.toml")
                .body(axum::body::Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();

    let bytes = to_bytes(resp.into_body(), usize::MAX).await.unwrap();
    assert!(!bytes.is_empty(), "response body must not be empty");
}

/// Response is identical across multiple calls (handler is pure / stateless).
#[tokio::test]
async fn test_stellar_toml_deterministic() {
    let mut bodies: Vec<Vec<u8>> = Vec::new();

    for _ in 0..5 {
        let app = build_app();
        let resp = app
            .oneshot(
                Request::builder()
                    .uri("/.well-known/stellar.toml")
                    .body(axum::body::Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        let bytes = to_bytes(resp.into_body(), usize::MAX).await.unwrap();
        bodies.push(bytes.to_vec());
    }

    let first = &bodies[0];
    for body in &bodies[1..] {
        assert_eq!(body, first, "all responses must be identical");
    }
}

// ---------------------------------------------------------------------------
// Concurrency tests
// ---------------------------------------------------------------------------

/// 10 concurrent requests all return 200.
#[tokio::test]
async fn test_stellar_toml_10_concurrent() {
    let handles: Vec<_> = (0..10)
        .map(|_| {
            let app = build_app();
            tokio::spawn(async move {
                let resp = app
                    .oneshot(
                        Request::builder()
                            .uri("/.well-known/stellar.toml")
                            .body(axum::body::Body::empty())
                            .unwrap(),
                    )
                    .await
                    .unwrap();
                resp.status()
            })
        })
        .collect();

    for handle in handles {
        assert_eq!(handle.await.unwrap(), StatusCode::OK);
    }
}

/// 50 concurrent requests all return 200.
#[tokio::test]
async fn test_stellar_toml_50_concurrent() {
    let handles: Vec<_> = (0..50)
        .map(|_| {
            let app = build_app();
            tokio::spawn(async move {
                let resp = app
                    .oneshot(
                        Request::builder()
                            .uri("/.well-known/stellar.toml")
                            .body(axum::body::Body::empty())
                            .unwrap(),
                    )
                    .await
                    .unwrap();
                resp.status()
            })
        })
        .collect();

    for handle in handles {
        assert_eq!(handle.await.unwrap(), StatusCode::OK);
    }
}

/// 100 concurrent requests all return 200.
#[tokio::test]
async fn test_stellar_toml_100_concurrent() {
    let handles: Vec<_> = (0..100)
        .map(|_| {
            let app = build_app();
            tokio::spawn(async move {
                let resp = app
                    .oneshot(
                        Request::builder()
                            .uri("/.well-known/stellar.toml")
                            .body(axum::body::Body::empty())
                            .unwrap(),
                    )
                    .await
                    .unwrap();
                resp.status()
            })
        })
        .collect();

    for handle in handles {
        assert_eq!(handle.await.unwrap(), StatusCode::OK);
    }
}

/// Verify that all concurrent responses have identical bodies.
#[tokio::test]
async fn test_stellar_toml_concurrent_identical_bodies() {
    let mut join_set = tokio::task::JoinSet::new();
    for _ in 0..20_usize {
        join_set.spawn(async {
            let app = build_app();
            let resp = app
                .oneshot(
                    Request::builder()
                        .uri("/.well-known/stellar.toml")
                        .body(axum::body::Body::empty())
                        .unwrap(),
                )
                .await
                .unwrap();
            to_bytes(resp.into_body(), usize::MAX)
                .await
                .unwrap()
                .to_vec()
        });
    }

    let mut bodies: Vec<Vec<u8>> = Vec::new();
    while let Some(Ok(body)) = join_set.join_next().await {
        bodies.push(body);
    }

    assert_eq!(bodies.len(), 20);
    let first = &bodies[0];
    for body in &bodies[1..] {
        assert_eq!(body, first, "all concurrent responses must be identical");
    }
}

// ---------------------------------------------------------------------------
// Framework-based load tests with SLO assertions
// ---------------------------------------------------------------------------

/// 10 concurrent tasks × 10 requests each = 100 total.
/// SLO: 0% errors, p99 < 200ms (stateless handler should be very fast).
#[tokio::test]
async fn test_stellar_load_100_requests_slo() {
    let result = run_framework_load(10, 10).await;
    result.print_summary("GET /.well-known/stellar.toml — 100 requests");
    assert_load_result(&result, 0.0, std::time::Duration::from_millis(200));
}

/// 20 concurrent tasks × 10 requests each = 200 total.
/// SLO: 0% errors, p99 < 500ms.
#[tokio::test]
async fn test_stellar_load_200_requests_slo() {
    let result = run_framework_load(20, 10).await;
    result.print_summary("GET /.well-known/stellar.toml — 200 requests");
    assert_load_result(&result, 0.0, std::time::Duration::from_millis(500));
}

/// 50 concurrent tasks × 10 requests each = 500 total.
/// SLO: 0% errors, p99 < 1s.
#[tokio::test]
async fn test_stellar_load_500_requests_slo() {
    let result = run_framework_load(50, 10).await;
    result.print_summary("GET /.well-known/stellar.toml — 500 requests");
    assert_load_result(&result, 0.0, std::time::Duration::from_secs(1));
}

/// Verify that all responses under load have the correct headers.
#[tokio::test]
async fn test_stellar_load_headers_under_load() {
    let mut join_set = tokio::task::JoinSet::new();
    for _ in 0..10_usize {
        join_set.spawn(async {
            let mut results = Vec::new();
            for _ in 0..5_usize {
                let app = build_app();
                let resp = app
                    .oneshot(
                        Request::builder()
                            .uri("/.well-known/stellar.toml")
                            .body(axum::body::Body::empty())
                            .unwrap(),
                    )
                    .await
                    .unwrap();
                let status = resp.status();
                let cors = resp
                    .headers()
                    .get("access-control-allow-origin")
                    .map(|v| v.to_str().unwrap().to_string());
                results.push((status, cors));
            }
            results
        });
    }

    while let Some(Ok(batch)) = join_set.join_next().await {
        for (status, cors) in batch {
            assert_eq!(status, StatusCode::OK);
            assert_eq!(
                cors.as_deref(),
                Some("*"),
                "CORS header must be '*' under load"
            );
        }
    }
}
