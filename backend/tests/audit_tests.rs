use super::audit::*;
use axum::Json;
use axum::{body::Body, http::Request, http::StatusCode};
use redis::AsyncCommands;
use serde_json::json;
use sqlx::{Executor, PgPool};
use std::sync::Arc;
use tokio::sync::OnceCell;

// Mock or test helpers for DB and Redis
static DB_POOL: OnceCell<PgPool> = OnceCell::const_new();
static REDIS_CLIENT: OnceCell<Arc<redis::Client>> = OnceCell::const_new();
use tower::ServiceExt;

async fn setup() -> (AuditService, PgPool, Arc<redis::Client>) {
    let db_url = std::env::var("DATABASE_URL").expect("DATABASE_URL must be set for tests");
    let redis_url = std::env::var("REDIS_URL").expect("REDIS_URL must be set for tests");
    let db = PgPool::connect(&db_url).await.unwrap();
    let redis = Arc::new(redis::Client::open(redis_url).unwrap());
    cleanup_audit_logs(&db).await;
    (AuditService::new(db.clone(), redis.clone()), db, redis)
}

async fn cleanup_audit_logs(db: &PgPool) {
    sqlx::query!("DELETE FROM audit_logs")
        .execute(db)
        .await
        .ok();
}

#[tokio::test]
async fn test_log_event_success() {
    let (service, db, redis) = setup().await;
    let event = AuditEvent {
        event_type: "login_attempt".to_string(),
        user_id: Some("user123".to_string()),
        details: json!({"ip": "127.0.0.1", "success": true}),
        timestamp: chrono::Utc::now(),
    };
    let result = service.log_event(event.clone()).await;
    assert!(result.is_ok());
    // Check DB
    let row = sqlx::query!(
        "SELECT * FROM audit_logs WHERE event_type = $1 ORDER BY timestamp DESC LIMIT 1",
        event.event_type
    )
    .fetch_one(&db)
    .await
    .unwrap();

    let row = sqlx::query!(
        "SELECT * FROM audit_logs WHERE event_type = $1 ORDER BY timestamp DESC LIMIT 1",
        event.event_type
    )
    .fetch_one(&db)
    .await
    .unwrap();
    assert_eq!(row.user_id, Some("user123".to_string()));

    let mut conn = redis.get_async_connection().await.unwrap();
    let val: String = conn.lpop("audit_queue", None).await.unwrap();
    let parsed: AuditEvent = serde_json::from_str(&val).unwrap();
    assert_eq!(parsed.event_type, "login_attempt");
}

#[tokio::test]
async fn test_log_audit_event_handler() {
    let (service, _, _) = setup().await;
    let app = axum::Router::new().merge(routes(Arc::new(service)));

    let payload = AuditEventRequest {
        event_type: "password_reset".to_string(),
        user_id: Some("user456".to_string()),
        details: json!({"ip": "10.0.0.1", "success": false}),
    };
    let body = serde_json::to_vec(&payload).unwrap();
    let response = Request::builder()
        .method("POST")
        .uri("/log")
        .header("content-type", "application/json")
        .body(Body::from(body))
        .unwrap();
    let resp = axum::Server::bind(&"127.0.0.1:0".parse().unwrap())
        .serve(app.into_make_service())
        .with_graceful_shutdown(async {
            tokio::time::sleep(std::time::Duration::from_millis(100)).await
        })
        .await;
    assert!(resp.is_ok());

    let resp = app.oneshot(response).await.unwrap();
    assert_eq!(resp.status(), StatusCode::CREATED);
}

#[tokio::test]
async fn test_list_audit_reports() {
    let (service, db, _) = setup().await;
    let service = Arc::new(service);
    let app = axum::Router::new().merge(routes(service.clone()));

    service
        .log_event(AuditEvent {
            event_type: "login_attempt".to_string(),
            user_id: Some("user123".to_string()),
            details: json!({"ip": "127.0.0.1"}),
            timestamp: chrono::Utc::now(),
        })
        .await
        .unwrap();

    let response = app
        .oneshot(
            Request::builder()
                .uri("/reports")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();

    assert_eq!(response.status(), StatusCode::OK);
    let body = hyper::body::to_bytes(response.into_body()).await.unwrap();
    let events: serde_json::Value = serde_json::from_slice(&body).unwrap();
    assert!(events.is_array());
    assert!(events.as_array().unwrap().len() >= 1);
}

async fn test_get_audit_report() {
    let row = sqlx::query!("SELECT id FROM audit_logs ORDER BY timestamp DESC LIMIT 1")
        .fetch_one(&db)
        .oneshot(
            Request::builder()
                .uri(format!("/reports/{}", row.id))
                .body(Body::empty())
                .unwrap(),
        );

    let event: serde_json::Value = serde_json::from_slice(&body).unwrap();
    assert_eq!(event["id"], row.id);
    assert_eq!(event["event_type"], "login_attempt");
}
