use axum::{body::Body, http::Request, http::StatusCode};
use backend::services::audit::{routes, AuditEvent, AuditEventRequest, AuditService};
use redis::AsyncCommands;
use serde_json::json;
use sqlx::{Executor, PgPool, Row};
use std::sync::Arc;
use tokio::sync::OnceCell;
use tower::ServiceExt;

// Mock or test helpers for DB and Redis
static DB_POOL: OnceCell<PgPool> = OnceCell::const_new();
static REDIS_CLIENT: OnceCell<Arc<redis::Client>> = OnceCell::const_new();

async fn setup() -> (AuditService, PgPool, Arc<redis::Client>) {
    let db_url = std::env::var("DATABASE_URL").expect("DATABASE_URL must be set for tests");
    let redis_url = std::env::var("REDIS_URL").expect("REDIS_URL must be set for tests");
    let db = PgPool::connect(&db_url).await.unwrap();
    let redis = Arc::new(redis::Client::open(redis_url).unwrap());
    cleanup_audit_logs(&db).await;
    (AuditService::new(db.clone(), redis.clone()), db, redis)
}

async fn cleanup_audit_logs(db: &PgPool) {
    let _ = sqlx::query("DELETE FROM audit_logs").execute(db).await;
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
    let row = sqlx::query("SELECT event_type, user_id FROM audit_logs WHERE event_type = $1 ORDER BY timestamp DESC LIMIT 1")
        .bind(&event.event_type)
        .fetch_optional(&db)
        .await
        .unwrap();

    let row = sqlx::query("SELECT event_type, user_id FROM audit_logs WHERE event_type = $1 ORDER BY timestamp DESC LIMIT 1")
        .bind(&event.event_type)
        .fetch_optional(&db)
        .await
        .unwrap();
    let row = row.and_then(|row| {
        let event_type: String = row.get("event_type");
        let user_id: Option<String> = row.get("user_id");
        Some((event_type, user_id))
    });
    assert_eq!(
        row.as_ref().map(|(_, user_id)| user_id.clone()),
        Some(Some("user123".to_string()))
    );

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
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let _ = tokio::time::timeout(
        std::time::Duration::from_millis(100),
        axum::serve(listener, app.clone()),
    )
    .await;

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
    let body = axum::body::to_bytes(response.into_body(), usize::MAX)
        .await
        .unwrap();
    let events: serde_json::Value = serde_json::from_slice(&body).unwrap();
    assert!(events.is_array());
    assert!(events.as_array().unwrap().len() >= 1);
}

#[tokio::test]
async fn test_get_audit_report() {
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

    let row = sqlx::query("SELECT id, event_type FROM audit_logs ORDER BY timestamp DESC LIMIT 1")
        .fetch_optional(&db)
        .await
        .unwrap();

    let row = row.unwrap();
    let id: i64 = row.get("id");
    let event_type: String = row.get("event_type");

    let response = app
        .oneshot(
            Request::builder()
                .uri(format!("/reports/{}", id))
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();

    let body = axum::body::to_bytes(response.into_body(), usize::MAX)
        .await
        .unwrap();
    let event: serde_json::Value = serde_json::from_slice(&body).unwrap();
    assert_eq!(event["id"], id);
    assert_eq!(event["event_type"], event_type);
}
