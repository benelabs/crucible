use axum::{body::Body, http::Request, http::StatusCode};
use backend::services::audit::{routes, AuditDomainEvent, AuditEvent, AuditEventRequest, AuditService};
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
    let event = AuditEventRequest {
        aggregate_id: None,
        event_type: "login_attempt".to_string(),
        user_id: Some("user123".to_string()),
        details: json!({"ip": "127.0.0.1", "success": true}),
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
    let val: String = conn.lpop("audit_event_stream", None).await.unwrap();
    let parsed: AuditDomainEvent = serde_json::from_str(&val).unwrap();
    assert_eq!(parsed.event_type, "login_attempt");
}

#[tokio::test]
async fn test_log_audit_event_handler() {
    let (service, _, _) = setup().await;
    let app = axum::Router::new().merge(routes(Arc::new(service)));

    let payload = AuditEventRequest {
        aggregate_id: None,
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
        .log_event(AuditEventRequest {
            aggregate_id: None,
            event_type: "login_attempt".to_string(),
            user_id: Some("user123".to_string()),
            details: json!({"ip": "127.0.0.1"}),
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
        .log_event(AuditEventRequest {
            aggregate_id: None,
            event_type: "login_attempt".to_string(),
            user_id: Some("user123".to_string()),
            details: json!({"ip": "127.0.0.1"}),
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

fn url_encode(s: &str) -> String {
    s.replace(":", "%3A").replace("+", "%2B")
}

#[tokio::test]
async fn test_audit_query_by_date_range() {
    let (service, db, _) = setup().await;
    let service = Arc::new(service);
    let app = axum::Router::new().merge(routes(service.clone()));

    let base_time = chrono::Utc::now();

    insert_mock_audit_log(&db, "test_event_1", None, base_time - chrono::Duration::days(5)).await;
    insert_mock_audit_log(&db, "test_event_2", None, base_time - chrono::Duration::days(2)).await;
    insert_mock_audit_log(&db, "test_event_3", None, base_time).await;

    let start_str = (base_time - chrono::Duration::days(4)).to_rfc3339();
    let end_str = (base_time - chrono::Duration::days(1)).to_rfc3339();
    let uri = format!(
        "/reports?start_date={}&end_date={}",
        url_encode(&start_str),
        url_encode(&end_str)
    );

    let response = app
        .clone()
        .oneshot(Request::builder().uri(uri).body(Body::empty()).unwrap())
        .await
        .unwrap();

    assert_eq!(response.status(), StatusCode::OK);
    let body = axum::body::to_bytes(response.into_body(), usize::MAX).await.unwrap();
    let events: serde_json::Value = serde_json::from_slice(&body).unwrap();
    assert_eq!(events.as_array().unwrap().len(), 1);
    assert_eq!(events[0]["event_type"], "test_event_2");

    let uri_start_only = format!("/reports?start_date={}", url_encode(&start_str));
    let response = app
        .clone()
        .oneshot(Request::builder().uri(uri_start_only).body(Body::empty()).unwrap())
        .await
        .unwrap();

    assert_eq!(response.status(), StatusCode::OK);
    let body = axum::body::to_bytes(response.into_body(), usize::MAX).await.unwrap();
    let events: serde_json::Value = serde_json::from_slice(&body).unwrap();
    assert_eq!(events.as_array().unwrap().len(), 2);
}

#[tokio::test]
async fn test_audit_multi_tenant_isolation() {
    let (service, db, _) = setup().await;
    let service = Arc::new(service);
    let app = axum::Router::new().merge(routes(service.clone()));

    let base_time = chrono::Utc::now();
    insert_mock_audit_log(&db, "tenant_a_event", Some("tenant-A"), base_time).await;
    insert_mock_audit_log(
        &db,
        "tenant_b_event",
        Some("tenant-B"),
        base_time - chrono::Duration::seconds(10),
    )
    .await;
    insert_mock_audit_log(
        &db,
        "no_tenant_event",
        None,
        base_time - chrono::Duration::seconds(20),
    )
    .await;

    let response = app
        .clone()
        .oneshot(
            Request::builder()
                .uri("/reports?tenant_id=tenant-A")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::OK);
    let body = axum::body::to_bytes(response.into_body(), usize::MAX).await.unwrap();
    let events: serde_json::Value = serde_json::from_slice(&body).unwrap();
    assert_eq!(events.as_array().unwrap().len(), 1);
    assert_eq!(events[0]["event_type"], "tenant_a_event");

    let response = app
        .clone()
        .oneshot(
            Request::builder()
                .uri("/reports?tenant_id=tenant-B")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::OK);
    let body = axum::body::to_bytes(response.into_body(), usize::MAX).await.unwrap();
    let events: serde_json::Value = serde_json::from_slice(&body).unwrap();
    assert_eq!(events.as_array().unwrap().len(), 1);
    assert_eq!(events[0]["event_type"], "tenant_b_event");
}

#[tokio::test]
async fn test_audit_pagination() {
    let (service, db, _) = setup().await;
    let service = Arc::new(service);
    let app = axum::Router::new().merge(routes(service.clone()));

    let base_time = chrono::Utc::now();
    for i in 1..=5 {
        insert_mock_audit_log(
            &db,
            &format!("event_{}", i),
            None,
            base_time - chrono::Duration::minutes(i),
        )
        .await;
    }

    let response = app
        .clone()
        .oneshot(Request::builder().uri("/reports?limit=2").body(Body::empty()).unwrap())
        .await
        .unwrap();
    let body = axum::body::to_bytes(response.into_body(), usize::MAX).await.unwrap();
    let events: serde_json::Value = serde_json::from_slice(&body).unwrap();
    assert_eq!(events.as_array().unwrap().len(), 2);
    assert_eq!(events[0]["event_type"], "event_1");
    assert_eq!(events[1]["event_type"], "event_2");

    let response = app
        .clone()
        .oneshot(
            Request::builder()
                .uri("/reports?limit=2&offset=2")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    let body = axum::body::to_bytes(response.into_body(), usize::MAX).await.unwrap();
    let events: serde_json::Value = serde_json::from_slice(&body).unwrap();
    assert_eq!(events.as_array().unwrap().len(), 2);
    assert_eq!(events[0]["event_type"], "event_3");
    assert_eq!(events[1]["event_type"], "event_4");
}

async fn insert_mock_audit_log(
    db: &sqlx::PgPool,
    event_type: &str,
    tenant_id: Option<&str>,
    timestamp: chrono::DateTime<chrono::Utc>,
) {
    let details = match tenant_id {
        Some(tid) => serde_json::json!({ "tenant_id": tid }),
        None => serde_json::json!({}),
    };
    sqlx::query(
        "INSERT INTO audit_logs (event_type, user_id, details, timestamp, hash, previous_hash) VALUES ($1, $2, $3, $4, $5, $6)"
    )
    .bind(event_type)
    .bind("test-user")
    .bind(details)
    .bind(timestamp)
    .bind("dummy-hash")
    .bind("dummy-prev-hash")
    .execute(db)
    .await
    .unwrap();
}
