//! Error Analytics Dashboard API handler implementation
//
// Implements endpoints for build error analytics dashboard.
// Uses Axum for HTTP, SQLx for DB, Redis for caching, and tracing for observability.

use crate::error::AppError;
use axum::{extract::State, response::IntoResponse, routing::get, Json, Router};
use redis::AsyncCommands;
use serde::{Deserialize, Serialize};
use sqlx::PgPool;
use tracing::{info, instrument};

#[derive(Debug, Serialize, Deserialize)]
pub struct BuildErrorAnalytics {
    pub total_errors: i64,
    pub error_types: Vec<(String, i64)>,
    pub recent_errors: Vec<BuildErrorDetail>,
}

#[derive(Debug, Serialize, Deserialize, sqlx::FromRow)]
pub struct BuildErrorDetail {
    pub id: i64,
    pub error_type: String,
    pub message: String,
    pub occurred_at: chrono::NaiveDateTime,
}

#[derive(Clone)]
pub struct ErrorAnalyticsState {
    pub pool: PgPool,
    pub redis: redis::Client,
}

#[instrument(skip(state))]
pub async fn get_build_error_analytics(
    State(state): State<ErrorAnalyticsState>,
) -> Result<impl IntoResponse, AppError> {
    let pool = state.pool;
    let redis = state.redis;
    // Try cache first
    let mut redis_conn = redis.get_multiplexed_async_connection().await.map_err(AppError::from)?;
    if let Ok(cached) = redis_conn.get::<_, String>("build_error_analytics").await {
        if let Ok(data) = serde_json::from_str::<BuildErrorAnalytics>(&cached) {
            info!(cache_hit = true, "Returning cached build error analytics");
            return Ok(Json(data));
        }
    }

    // Query DB for analytics
    let total_errors: (i64,) = sqlx::query_as("SELECT COUNT(*) FROM build_errors")
        .fetch_one(&pool)
        .await?;
    let error_types = sqlx::query_as::<_, (String, i64)>(
        "SELECT error_type, COUNT(*) FROM build_errors GROUP BY error_type ORDER BY COUNT(*) DESC",
    )
    .fetch_all(&pool)
    .await?;
    let recent_errors = sqlx::query_as::<_, BuildErrorDetail>(
        "SELECT id, error_type, message, occurred_at FROM build_errors ORDER BY occurred_at DESC LIMIT 10",
    )
    .fetch_all(&pool)
    .await?;

    let analytics = BuildErrorAnalytics {
        total_errors: total_errors.0,
        error_types,
        recent_errors,
    };

    // Cache result
    let _: () = redis_conn
        .set_ex::<_, _, ()>(
            "build_error_analytics",
            serde_json::to_string(&analytics).unwrap(),
            60,
        )
        .await
        .map_err(AppError::from)?;

    Ok(Json(analytics))
}

pub fn error_analytics_routes(pool: PgPool, redis: redis::Client) -> Router {
    let state = ErrorAnalyticsState { pool, redis };
    Router::new()
        .route("/dashboard/build-errors", get(get_build_error_analytics))
        .with_state(state)
}
