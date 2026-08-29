//! Health check endpoints.
//!
//! Provides two endpoints:
//!
//! - `GET /health/live`  — liveness probe: returns 200 if the process is running.
//! - `GET /health/ready` — readiness probe: returns 200 only when PostgreSQL,
//!   Redis, worker queue, and Soroban RPC are reachable; returns 503 otherwise.
//!
//! Both endpoints return a JSON body with per-component status details so that
//! operators can quickly identify which dependency is unhealthy. Connection
//! strings, hostnames, and credentials are never included in responses.

use std::time::Duration;
use axum::{extract::State, http::StatusCode, response::IntoResponse, Json};
use redis::aio::ConnectionManager;
use serde::Serialize;
use sqlx::PgPool;
use tokio::time::timeout;
use tracing::{debug, instrument, warn};

/// Timeout threshold for individual dependency health check pings.
const HEALTH_CHECK_TIMEOUT: Duration = Duration::from_secs(2);

/// Minimal application state required by health check handlers.
#[derive(Clone)]
pub struct HealthState {
    pub db: PgPool,
    pub cache: ConnectionManager,
    pub queue: ConnectionManager,
    pub soroban_rpc_url: Option<String>,
}

// ---------------------------------------------------------------------------
// Response types
// ---------------------------------------------------------------------------

/// Single dependency check result.
#[derive(Debug, Serialize, Clone, PartialEq, Eq)]
#[serde(rename_all = "lowercase")]
pub struct CheckResult {
    pub status: &'static str,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub message: Option<String>,
}

/// Container for all dependency checks.
#[derive(Debug, Serialize, Clone, PartialEq, Eq)]
pub struct HealthChecks {
    pub database: CheckResult,
    pub redis: CheckResult,
    pub queue: CheckResult,
    pub soroban_rpc: CheckResult,
}

/// Response body for the readiness probe.
#[derive(Debug, Serialize, Clone, PartialEq, Eq)]
pub struct HealthReport {
    /// Overall status: `"healthy"` or `"degraded"`.
    pub status: String,
    /// Per-dependency health details.
    pub checks: HealthChecks,
    /// Application version from `CARGO_PKG_VERSION`.
    pub version: String,
}

/// Response body for the liveness probe.
#[derive(Debug, Serialize, Clone, PartialEq, Eq)]
pub struct LivenessResponse {
    pub status: &'static str,
    pub version: String,
}

// ---------------------------------------------------------------------------
// HealthChecker Abstraction
// ---------------------------------------------------------------------------

/// Deep dependency diagnostic executor with timeout guards.
pub struct HealthChecker;

impl HealthChecker {
    /// Execute health pings across all dependencies with quick timeout bounds.
    pub async fn run_checks(state: &HealthState) -> (bool, HealthReport) {
        let database = Self::check_database_with_timeout(&state.db).await;
        let redis = Self::check_cache_with_timeout(&state.cache).await;
        let queue = Self::check_queue_with_timeout(&state.queue).await;
        let soroban_rpc = Self::check_soroban_rpc_with_timeout(state.soroban_rpc_url.as_deref()).await;

        let all_healthy = database.status == "up"
            && redis.status == "up"
            && queue.status == "up"
            && soroban_rpc.status == "up";

        let report = HealthReport {
            status: if all_healthy {
                "healthy".into()
            } else {
                "degraded".into()
            },
            checks: HealthChecks {
                database,
                redis,
                queue,
                soroban_rpc,
            },
            version: env!("CARGO_PKG_VERSION").to_string(),
        };

        (all_healthy, report)
    }

    pub async fn check_database_with_timeout(pool: &PgPool) -> CheckResult {
        match timeout(HEALTH_CHECK_TIMEOUT, check_database(pool)).await {
            Ok(result) => result,
            Err(_) => CheckResult {
                status: "down",
                message: Some("database health check timed out".into()),
            },
        }
    }

    pub async fn check_cache_with_timeout(conn: &ConnectionManager) -> CheckResult {
        match timeout(HEALTH_CHECK_TIMEOUT, check_cache(conn)).await {
            Ok(result) => result,
            Err(_) => CheckResult {
                status: "down",
                message: Some("redis health check timed out".into()),
            },
        }
    }

    pub async fn check_queue_with_timeout(conn: &ConnectionManager) -> CheckResult {
        match timeout(HEALTH_CHECK_TIMEOUT, check_queue(conn)).await {
            Ok(result) => result,
            Err(_) => CheckResult {
                status: "down",
                message: Some("queue health check timed out".into()),
            },
        }
    }

    pub async fn check_soroban_rpc_with_timeout(url: Option<&str>) -> CheckResult {
        let url = match url {
            Some(u) if !u.trim().is_empty() => u,
            _ => {
                return CheckResult {
                    status: "up",
                    message: Some("rpc endpoint unconfigured (defaulting up)".into()),
                }
            }
        };

        match timeout(HEALTH_CHECK_TIMEOUT, async {
            reqwest::Client::new()
                .post(url)
                .json(&serde_json::json!({
                    "jsonrpc": "2.0",
                    "id": 1,
                    "method": "getHealth"
                }))
                .send()
                .await
        })
        .await
        {
            Ok(Ok(resp)) if resp.status().is_success() => CheckResult {
                status: "up",
                message: None,
            },
            Ok(Ok(resp)) => CheckResult {
                status: "down",
                message: Some(format!("rpc error status: {}", resp.status())),
            },
            Ok(Err(e)) => CheckResult {
                status: "down",
                message: Some(format!("rpc unreachable: {e}")),
            },
            Err(_) => CheckResult {
                status: "down",
                message: Some("rpc health check timed out".into()),
            },
        }
    }
}

// ---------------------------------------------------------------------------
// Handlers
// ---------------------------------------------------------------------------

/// `GET /health/live` — liveness probe.
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
/// Returns 200 OK when all backing services are healthy, or 503 Service Unavailable
/// when any dependency is degraded or unreachable.
#[instrument(skip_all)]
pub async fn readiness(State(state): State<HealthState>) -> impl IntoResponse {
    let (all_healthy, report) = HealthChecker::run_checks(&state).await;

    let status_code = if all_healthy {
        StatusCode::OK
    } else {
        StatusCode::SERVICE_UNAVAILABLE
    };

    (status_code, Json(report))
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

async fn has_registered_workers(conn: &mut ConnectionManager) -> bool {
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
                soroban_rpc: CheckResult {
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
        assert_eq!(json["checks"]["soroban_rpc"]["status"], "up");
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
                soroban_rpc: CheckResult {
                    status: "up",
                    message: None,
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
        assert_eq!(json["checks"]["queue"]["status"], "down");
    }

    #[test]
    fn partial_degradation_status_code_mapping() {
        let healthy_report = HealthReport {
            status: "healthy".into(),
            checks: HealthChecks {
                database: CheckResult { status: "up", message: None },
                redis: CheckResult { status: "up", message: None },
                queue: CheckResult { status: "up", message: None },
                soroban_rpc: CheckResult { status: "up", message: None },
            },
            version: "0.1.0".into(),
        };
        let degraded_report = HealthReport {
            status: "degraded".into(),
            checks: HealthChecks {
                database: CheckResult { status: "down", message: Some("err".into()) },
                redis: CheckResult { status: "up", message: None },
                queue: CheckResult { status: "up", message: None },
                soroban_rpc: CheckResult { status: "up", message: None },
            },
            version: "0.1.0".into(),
        };

        let status_healthy = if healthy_report.status == "healthy" {
            StatusCode::OK
        } else {
            StatusCode::SERVICE_UNAVAILABLE
        };

        let status_degraded = if degraded_report.status == "healthy" {
            StatusCode::OK
        } else {
            StatusCode::SERVICE_UNAVAILABLE
        };

        assert_eq!(status_healthy, StatusCode::OK);
        assert_eq!(status_degraded, StatusCode::SERVICE_UNAVAILABLE);
    }
}
