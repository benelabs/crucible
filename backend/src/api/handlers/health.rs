//! Health check endpoints.
//!
//! Provides two endpoints:
//!
//! - `GET /health/live`  — liveness probe: returns 200 if the process is running.
//! - `GET /health/ready` — readiness probe: returns 200 only when PostgreSQL,
//!   Redis, and the worker queue are reachable; returns 503 otherwise.
//!
//! Both endpoints return a JSON body with per-component status details so that
//! operators can quickly identify which dependency is unhealthy. Connection
//! strings, hostnames, and credentials are never included in responses.

use axum::{extract::State, http::StatusCode, response::IntoResponse, Json};
use redis::aio::ConnectionManager;
use serde::Serialize;
use sqlx::PgPool;
use tracing::{debug, instrument, warn};

/// Minimal application state required by health check handlers.
#[derive(Clone)]
pub struct HealthState {
    pub db: PgPool,
    pub cache: ConnectionManager,
    pub queue: ConnectionManager,
}

// ---------------------------------------------------------------------------
// Response types
// ---------------------------------------------------------------------------

/// Single dependency check result.
#[derive(Debug, Serialize)]
#[serde(rename_all = "lowercase")]
pub struct CheckResult {
    pub status: &'static str,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub message: Option<String>,
}

/// Container for all dependency checks.
#[derive(Debug, Serialize)]
pub struct HealthChecks {
    pub database: CheckResult,
    pub redis: CheckResult,
    pub queue: CheckResult,
}

/// Response body for the readiness probe.
#[derive(Debug, Serialize)]
pub struct HealthReport {
    /// Overall status: `"healthy"` or `"degraded"`.
    pub status: String,
    /// Per-dependency health details.
    pub checks: HealthChecks,
    /// Application version from `CARGO_PKG_VERSION`.
    pub version: String,
}

/// Response body for the liveness probe.
#[derive(Debug, Serialize)]
pub struct LivenessResponse {
    pub status: &'static str,
    pub version: String,
}

// ---------------------------------------------------------------------------
// Handlers
// ---------------------------------------------------------------------------

/// `GET /health/live` — liveness probe.
///
/// Always returns `200 OK` as long as the process is running. Kubernetes uses
/// this to decide whether to restart the container. Never fails due to
/// PostgreSQL, Redis, or worker queue unavailability.
#[instrument(skip_all)]
pub async fn liveness() -> impl IntoResponse {
    debug!("Liveness probe");
    (
        StatusCode::OK,
        Json(LivenessResponse {
            status: "ok",
            version: env!("CARGO_PKG_VERSION").to_string(),
        }),
    )
}

/// `GET /health/ready` — readiness probe.
///
/// Checks PostgreSQL, Redis, and worker queue connectivity. Returns `200 OK`
/// when all dependencies are healthy, or `503 Service Unavailable` when any
/// are not. Kubernetes uses this to decide whether to route traffic to the pod.
#[instrument(skip_all)]
pub async fn readiness(State(state): State<HealthState>) -> impl IntoResponse {
    let database = check_database(&state.db).await;
    let redis = check_cache(&state.cache).await;
    let queue = check_queue(&state.queue).await;

    let all_healthy =
        database.status == "up" && redis.status == "up" && queue.status == "up";

    let status_code = if all_healthy {
        StatusCode::OK
    } else {
        StatusCode::SERVICE_UNAVAILABLE
    };

    (
        status_code,
        Json(HealthReport {
            status: if all_healthy {
                "healthy".into()
            } else {
                "degraded".into()
            },
            checks: HealthChecks {
                database,
                redis,
                queue,
            },
            version: env!("CARGO_PKG_VERSION").to_string(),
        }),
    )
}

// ---------------------------------------------------------------------------
// Dependency checks
// ---------------------------------------------------------------------------

async fn check_database(pool: &PgPool) -> CheckResult {
    match sqlx::query_scalar::<_, i32>("SELECT 1")
        .fetch_one(pool)
        .await
    {
        Ok(_) => {
            debug!("Database health check passed");
            CheckResult {
                status: "up",
                message: None,
            }
        }
        Err(e) => {
            warn!("Database health check failed: {e}");
            CheckResult {
                status: "down",
                message: Some("connection unavailable".into()),
            }
        }
    }
}

async fn check_cache(conn: &ConnectionManager) -> CheckResult {
    let mut conn = conn.clone();
    match redis::cmd("PING").query_async::<String>(&mut conn).await {
        Ok(_) => {
            debug!("Redis health check passed");
            CheckResult {
                status: "up",
                message: None,
            }
        }
        Err(e) => {
            warn!("Redis health check failed: {e}");
            CheckResult {
                status: "down",
                message: Some("connection unavailable".into()),
            }
        }
    }
}

async fn check_queue(conn: &ConnectionManager) -> CheckResult {
    let mut conn = conn.clone();

    // Step 1: Verify queue backend (Redis) connectivity
    let ping = redis::cmd("PING")
        .query_async::<String>(&mut conn)
        .await;
    match ping {
        Ok(_) => debug!("Queue backend connection is healthy"),
        Err(e) => {
            warn!("Queue backend connection failed: {e}");
            return CheckResult {
                status: "down",
                message: Some("connection unavailable".into()),
            };
        }
    }

    // Step 2: Check whether workers are actually registered.
    // Apalis workers register in a `{namespace}:consumers` Redis SET via their
    // keep-alive heartbeat.  Also checks the project's `worker:*:health` pattern
    // used by WorkerHealthMonitor.
    if has_registered_workers(&mut conn).await {
        debug!("Queue health check passed — active workers found");
        CheckResult {
            status: "up",
            message: None,
        }
    } else {
        warn!("Queue health check failed — no active workers");
        CheckResult {
            status: "down",
            message: Some("workers unavailable".into()),
        }
    }
}

/// Returns `true` when at least one worker consumer or health heartbeat is
/// present in Redis.
async fn has_registered_workers(conn: &mut ConnectionManager) -> bool {
    // Check for Apalis consumer sets (workers registered via keep_alive)
    match redis::cmd("KEYS")
        .arg("*:consumers")
        .query_async::<Vec<String>>(conn)
        .await
    {
        Ok(keys) if !keys.is_empty() => {
            for key in &keys {
                if let Ok(count) = redis::cmd("SCARD")
                    .arg(key)
                    .query_async::<i32>(conn)
                    .await
                {
                    if count > 0 {
                        return true;
                    }
                }
            }
        }
        _ => {}
    }

    // Also check for project-specific worker heartbeat keys
    match redis::cmd("KEYS")
        .arg("worker:*:health")
        .query_async::<Vec<String>>(conn)
        .await
    {
        Ok(keys) if !keys.is_empty() => true,
        _ => false,
    }
}

// ---------------------------------------------------------------------------
// Router helper
// ---------------------------------------------------------------------------

/// Returns an Axum router with the health check routes mounted.
///
/// Mount this under `/health` in the main application router:
///
/// ```rust,no_run
/// use axum::Router;
/// use backend::api::handlers::health;
///
/// let app: Router = Router::new()
///     .nest("/health", health::router());
/// ```
pub fn router() -> axum::Router<HealthState> {
    use axum::routing::get;
    axum::Router::new()
        .route("/live", get(liveness))
        .route("/ready", get(readiness))
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use axum::{body::Body, http::Request};
    use tower::ServiceExt;

    /// Build a minimal router with only the liveness endpoint (no AppState needed).
    fn liveness_app() -> axum::Router {
        use axum::routing::get;
        axum::Router::new().route("/live", get(liveness))
    }

    #[tokio::test]
    async fn liveness_returns_200() {
        let app = liveness_app();
        let response = app
            .oneshot(Request::builder().uri("/live").body(Body::empty()).unwrap())
            .await
            .unwrap();
        assert_eq!(response.status(), StatusCode::OK);
    }

    #[tokio::test]
    async fn liveness_body_contains_ok() {
        let app = liveness_app();
        let response = app
            .oneshot(Request::builder().uri("/live").body(Body::empty()).unwrap())
            .await
            .unwrap();
        let body = axum::body::to_bytes(response.into_body(), usize::MAX)
            .await
            .unwrap();
        let json: serde_json::Value = serde_json::from_slice(&body).unwrap();
        assert_eq!(json["status"], "ok");
        assert!(json["version"].is_string());
    }

    #[test]
    fn health_report_serializes_healthy() {
        let report = HealthReport {
            status: "healthy".into(),
            checks: HealthChecks {
                database: CheckResult {
                    status: "up",
                    message: None,
                },
                redis: CheckResult {
                    status: "up",
                    message: None,
                },
                queue: CheckResult {
                    status: "up",
                    message: None,
                },
            },
            version: "0.1.0".into(),
        };
        let json = serde_json::to_value(&report).unwrap();
        assert_eq!(json["status"], "healthy");
        assert_eq!(json["checks"]["database"]["status"], "up");
        assert_eq!(json["checks"]["redis"]["status"], "up");
        assert_eq!(json["checks"]["queue"]["status"], "up");
        assert!(json["checks"]["database"].get("message").is_none());
        assert_eq!(json["version"], "0.1.0");
    }

    #[test]
    fn health_report_serializes_degraded() {
        let report = HealthReport {
            status: "degraded".into(),
            checks: HealthChecks {
                database: CheckResult {
                    status: "down",
                    message: Some("connection unavailable".into()),
                },
                redis: CheckResult {
                    status: "up",
                    message: None,
                },
                queue: CheckResult {
                    status: "down",
                    message: Some("workers unavailable".into()),
                },
            },
            version: "0.1.0".into(),
        };
        let json = serde_json::to_value(&report).unwrap();
        assert_eq!(json["status"], "degraded");
        assert_eq!(json["checks"]["database"]["status"], "down");
        assert_eq!(
            json["checks"]["database"]["message"],
            "connection unavailable"
        );
        assert_eq!(json["checks"]["redis"]["status"], "up");
        assert_eq!(json["checks"]["redis"].get("message"), None);
        assert_eq!(json["checks"]["queue"]["status"], "down");
        assert_eq!(
            json["checks"]["queue"]["message"],
            "workers unavailable"
        );
    }
}
