use std::sync::Arc;

use axum::{
    body::{to_bytes, Body},
    extract::State,
    http::{header::CONTENT_TYPE, HeaderValue, Request, StatusCode},
    middleware::Next,
    response::Response,
};
use redis::AsyncCommands;
use serde_json::json;

use crate::api::handlers::profiling::AppState;
use crate::error::AppError;

const IDEMPOTENCY_TTL_SECONDS: u64 = 86_400;
const IDEMPOTENCY_ROUTE: &str = "/api/alerts/ingest";

/// Cache successful `POST /api/alerts/ingest` responses by idempotency key.
///
/// When `Idempotency-Key` is present:
/// - cache hit: replay the stored body immediately.
/// - cache miss: execute handler, then persist successful `200 OK` response.
pub async fn idempotency_middleware(
    State(state): State<Arc<AppState>>,
    request: Request<Body>,
    next: Next,
) -> Result<Response, AppError> {
    let key = request
        .headers()
        .get("Idempotency-Key")
        .and_then(|value| value.to_str().ok())
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(|value| value.to_string());

    let Some(key) = key else {
        return Ok(next.run(request).await);
    };

    let cache_key = format!("idempotency:{IDEMPOTENCY_ROUTE}:{key}");
    let mut conn = state.redis.get_multiplexed_async_connection().await?;

    if let Some(cached_response) = conn.get::<_, Option<String>>(&cache_key).await? {
        let mut response = Response::new(Body::from(cached_response));
        *response.status_mut() = StatusCode::OK;
        response
            .headers_mut()
            .insert(CONTENT_TYPE, HeaderValue::from_static("application/json"));
        response.headers_mut().insert(
            "x-idempotency-replayed",
            HeaderValue::from_static("true"),
        );
        return Ok(response);
    }

    let response = next.run(request).await;
    if response.status() != StatusCode::OK {
        return Ok(response);
    }

    let (parts, body) = response.into_parts();
    let body_bytes = to_bytes(body, usize::MAX)
        .await
        .map_err(|error| AppError::InternalError(format!("failed to read response body: {error}")))?;

    let body_text = String::from_utf8(body_bytes.to_vec())
        .map_err(|error| AppError::InternalError(format!("response body was not UTF-8: {error}")))?;

    conn.set_ex::<_, _, ()>(cache_key, body_text.clone(), IDEMPOTENCY_TTL_SECONDS)
        .await?;

    let mut rebuilt = Response::from_parts(parts, Body::from(body_text));
    rebuilt.headers_mut().insert(
        "x-idempotency-replayed",
        HeaderValue::from_static("false"),
    );
    Ok(rebuilt)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{
        api::handlers::profiling::AppState,
        config::{reload::ConfigManager, AppConfig},
        services::{
            contract_benchmark::ContractBenchmarkService, error_recovery::ErrorManager,
            log_aggregator::LogAggregator, sys_metrics::MetricsExporter,
        },
    };
    use axum::{middleware, routing::post, Json, Router};
    use std::sync::{
        atomic::{AtomicUsize, Ordering},
        Arc,
    };
    use tower::ServiceExt;

    #[tokio::test]
    #[ignore = "requires local Redis server"]
    async fn duplicate_request_replays_original_response() {
        let counter = Arc::new(AtomicUsize::new(0));
        let counter_for_handler = counter.clone();

        let redis = redis::Client::open("redis://127.0.0.1:6379").unwrap();
        let app_state = Arc::new(AppState {
            db: None,
            metrics_exporter: Arc::new(MetricsExporter::new()),
            error_manager: Arc::new(ErrorManager::new()),
            config_manager: Arc::new(ConfigManager::new(AppConfig::default())),
            log_aggregator: Arc::new(LogAggregator::new().0),
            contract_benchmark_service: Arc::new(ContractBenchmarkService::new()),
            redis: redis.clone(),
        });

        let app = Router::new()
            .route(
                IDEMPOTENCY_ROUTE,
                post(move || {
                    let counter = counter_for_handler.clone();
                    async move {
                        let value = counter.fetch_add(1, Ordering::SeqCst) + 1;
                        Json(json!({ "request_number": value }))
                    }
                })
                .route_layer(middleware::from_fn_with_state(
                    app_state.clone(),
                    idempotency_middleware,
                )),
            )
            .with_state(app_state);

        let request = || {
            Request::builder()
                .method("POST")
                .uri(IDEMPOTENCY_ROUTE)
                .header("content-type", "application/json")
                .header("Idempotency-Key", "duplicate-check-1")
                .body(Body::from(r#"{"message":"hello"}"#))
                .unwrap()
        };

        let first = app.clone().oneshot(request()).await.unwrap();
        let second = app.oneshot(request()).await.unwrap();

        let first_body = to_bytes(first.into_body(), usize::MAX).await.unwrap();
        let second_body = to_bytes(second.into_body(), usize::MAX).await.unwrap();

        assert_eq!(first_body, second_body);
        assert_eq!(counter.load(Ordering::SeqCst), 1);
    }
}
