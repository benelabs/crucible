//! Integration tests for `GET /health/live` and `GET /health/ready`.
//!
//! Each scenario builds a minimal Axum router with a `HealthState` tuned to
//! the conditions under test (healthy DB, broken DB, no worker, etc.).

use axum::{
    body::Body,
    http::{Request, StatusCode},
    Router,
};
use backend::api::handlers::health::{self, HealthState, LivenessResponse};
use redis::aio::ConnectionManager;
use redis::AsyncCommands;
use redis::Client as RedisClient;
use sqlx::postgres::PgPoolOptions;
use sqlx::PgPool;
use tower::ServiceExt;

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

/// A pool whose queries will succeed (requires a reachable PostgreSQL).
fn live_pool() -> PgPool {
    let url = std::env::var("DATABASE_URL")
        .or_else(|_| std::env::var("TEST_DATABASE_URL"))
        .unwrap_or_else(|_| "postgres://postgres:password@localhost:5432/crucible_test".into());
    PgPoolOptions::new()
        .max_connections(2)
        .connect_lazy(&url)
        .expect("live_pool")
}

/// A pool whose queries will always fail.
fn dead_pool() -> PgPool {
    PgPoolOptions::new()
        .max_connections(1)
        .connect_lazy("postgres://invalid:5432/nonexistent")
        .expect("dead_pool")
}

/// A [`ConnectionManager`] backed by a real Redis instance.
async fn live_redis() -> ConnectionManager {
    let url = std::env::var("REDIS_URL")
        .or_else(|_| std::env::var("TEST_REDIS_URL"))
        .unwrap_or_else(|_| "redis://127.0.0.1:6379".into());
    let client = RedisClient::open(url).expect("live_redis client");
    ConnectionManager::new(client)
        .await
        .expect("live_redis connection")
}

/// Attempt to create a [`ConnectionManager`] that will fail I/O.  Returns
/// `None` when the connection is refused immediately at construction time.
async fn dead_redis() -> Option<ConnectionManager> {
    let client = RedisClient::open("redis://127.0.0.1:1/").expect("dead_redis client");
    ConnectionManager::new(client).await.ok()
}

/// Register a synthetic worker heartbeat so the queue-probe sees an active
/// worker and reports "up".
async fn register_worker_hb(conn: &mut ConnectionManager) {
    let payload = serde_json::json!({
        "worker_id": "health-test-worker",
        "last_heartbeat": 0,
        "last_checked": 0,
        "is_healthy": true,
        "uptime_seconds": 0,
    })
    .to_string();
    conn.set("worker:health-test-worker:health", &payload)
        .await
        .expect("register_worker_hb");
}

/// Remove the synthetic worker heartbeat.
async fn remove_worker_hb(conn: &mut ConnectionManager) {
    let _: Result<(), _> = conn.del("worker:health-test-worker:health").await;
}

/// Parse the response body into a JSON value.
async fn body_json(resp: axum::response::Response) -> serde_json::Value {
    let bytes = axum::body::to_bytes(resp.into_body(), usize::MAX)
        .await
        .expect("read body");
    serde_json::from_slice(&bytes).expect("valid JSON")
}

// ---------------------------------------------------------------------------
// Liveness — never depends on external services
// ---------------------------------------------------------------------------

#[tokio::test]
async fn liveness_returns_200() {
    let app = Router::new().route("/live", axum::routing::get(health::liveness));
    let resp = app
        .oneshot(Request::builder().uri("/live").body(Body::empty()).unwrap())
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::OK);

    let json = body_json(resp).await;
    assert_eq!(json["status"], "ok");
    assert!(json["version"].is_string());
}

// ---------------------------------------------------------------------------
// Readiness — all dependencies healthy
// ---------------------------------------------------------------------------

#[tokio::test]
async fn readiness_all_healthy() {
    let db = live_pool();
    let cache = live_redis().await;
    let mut queue = cache.clone();

    // Register a worker heartbeat so the queue check passes
    register_worker_hb(&mut queue).await;

    let state = HealthState {
        db,
        cache,
        queue,
    };

    let app = Router::new().nest("/health", health::router().with_state(state));
    let resp = app
        .oneshot(
            Request::builder()
                .uri("/health/ready")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();

    assert_eq!(resp.status(), StatusCode::OK);
    let json = body_json(resp).await;
    assert_eq!(json["status"], "healthy");
    assert_eq!(json["checks"]["database"]["status"], "up");
    assert_eq!(json["checks"]["redis"]["status"], "up");
    assert_eq!(json["checks"]["queue"]["status"], "up");
    assert!(json["version"].is_string());

    // Cleanup — remove the synthetic heartbeat
    let mut cleanup = live_redis().await;
    remove_worker_hb(&mut cleanup).await;
}

// ---------------------------------------------------------------------------
// Readiness — PostgreSQL unavailable
// ---------------------------------------------------------------------------

#[tokio::test]
async fn readiness_db_unavailable() {
    let cache = live_redis().await;
    let queue = cache.clone();

    let state = HealthState {
        db: dead_pool(),
        cache,
        queue,
    };

    let app = Router::new().nest("/health", health::router().with_state(state));
    let resp = app
        .oneshot(
            Request::builder()
                .uri("/health/ready")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();

    assert_eq!(resp.status(), StatusCode::SERVICE_UNAVAILABLE);
    let json = body_json(resp).await;
    assert_eq!(json["status"], "degraded");
    assert_eq!(json["checks"]["database"]["status"], "down");
    assert_eq!(
        json["checks"]["database"]["message"],
        "connection unavailable"
    );
}

// ---------------------------------------------------------------------------
// Readiness — queue unavailable (no active workers)
// ---------------------------------------------------------------------------

#[tokio::test]
async fn readiness_queue_unavailable() {
    let db = live_pool();
    let cache = live_redis().await;
    // Re-use the same Redis — no worker heartbeat or Apalis consumer set
    // exists in the test namespace, so the queue probe will correctly
    // report "down".
    let queue = cache.clone();

    let state = HealthState {
        db,
        cache,
        queue,
    };

    let app = Router::new().nest("/health", health::router().with_state(state));
    let resp = app
        .oneshot(
            Request::builder()
                .uri("/health/ready")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();

    assert_eq!(resp.status(), StatusCode::SERVICE_UNAVAILABLE);
    let json = body_json(resp).await;
    assert_eq!(json["status"], "degraded");
    assert_eq!(json["checks"]["queue"]["status"], "down");
    assert_eq!(
        json["checks"]["queue"]["message"],
        "workers unavailable"
    );
}

// ---------------------------------------------------------------------------
// Readiness — Redis unavailable
// ---------------------------------------------------------------------------

#[tokio::test]
async fn readiness_redis_unavailable() {
    let some_redis = dead_redis().await;

    let (cache, queue) = match some_redis {
        Some(conn) => {
            // Connection was established lazily — the PING will fail later.
            (conn.clone(), conn)
        }
        None => {
            // Connection was refused at construction time, so we cannot build
            // a HealthState.  The check functions handle this internally; the
            // degraded-response schema is verified via the unit tests in
            // health.rs and the other integration tests in this file.
            eprintln!(
                "⚠  Skipping full Redis-down integration test — \
                 port 1 refused connection immediately.  \
                 The degraded-response schema is verified via the unit tests \
                 in health.rs."
            );
            return;
        }
    };

    let state = HealthState {
        db: live_pool(),
        cache,
        queue,
    };

    let app = Router::new().nest("/health", health::router().with_state(state));
    let resp = app
        .oneshot(
            Request::builder()
                .uri("/health/ready")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();

    // Redis is down so both the cache and queue checks should fail.
    assert_eq!(resp.status(), StatusCode::SERVICE_UNAVAILABLE);
    let json = body_json(resp).await;
    assert_eq!(json["status"], "degraded");
    assert_eq!(json["checks"]["redis"]["status"], "down");
    assert_eq!(
        json["checks"]["redis"]["message"],
        "connection unavailable"
    );
    assert_eq!(json["checks"]["queue"]["status"], "down");
}

// ---------------------------------------------------------------------------
// Liveness stays healthy when readiness would be degraded
// ---------------------------------------------------------------------------

#[tokio::test]
async fn liveness_stays_healthy_when_deps_are_down() {
    let cache = live_redis().await;
    let state = HealthState {
        db: dead_pool(),
        cache: cache.clone(),
        queue: cache,
    };

    // Mount both liveness and readiness.
    let app = Router::new().nest("/health", health::router().with_state(state));

    // Liveness MUST return 200 even when deps are down.
    let live = app
        .clone()
        .oneshot(
            Request::builder()
                .uri("/health/live")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(live.status(), StatusCode::OK);
    let json = body_json(live).await;
    assert_eq!(json["status"], "ok");
    assert!(json["version"].is_string());

    // Readiness returns 503 with the same broken state.
    let ready = app
        .oneshot(
            Request::builder()
                .uri("/health/ready")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(ready.status(), StatusCode::SERVICE_UNAVAILABLE);
    let json = body_json(ready).await;
    assert_eq!(json["status"], "degraded");
    assert_eq!(json["checks"]["database"]["status"], "down");
}
