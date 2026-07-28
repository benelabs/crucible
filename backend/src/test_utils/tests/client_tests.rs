//! API TEST CLIENT APPROACH: Option A — builder pattern per request
//! Rationale: Testing the test client itself ensures our utilities are durable. Option A's fluent interface allows us to concisely test header insertion and body parsing.

use axum::{
    extract::Query,
    http::{HeaderMap, StatusCode},
    response::Json,
    routing::{get, post},
    Router,
};
use serde::{Deserialize, Serialize};

use crate::test_utils::client::ApiTestClient;

#[derive(Serialize, Deserialize, Debug, PartialEq)]
struct TestPayload {
    name: String,
    age: i32,
}

#[derive(Deserialize)]
struct QueryPayload {
    search: String,
}

fn test_router() -> Router {
    Router::new()
        .route("/health", get(|| async { "OK" }))
        .route(
            "/echo",
            post(|Json(payload): Json<TestPayload>| async move { Json(payload) }),
        )
        .route(
            "/auth-check",
            get(|headers: HeaderMap| async move {
                if headers.contains_key("authorization") {
                    StatusCode::OK
                } else {
                    StatusCode::UNAUTHORIZED
                }
            }),
        )
        .route(
            "/header-check",
            get(|headers: HeaderMap| async move {
                if headers.contains_key("x-custom-header") {
                    StatusCode::OK
                } else {
                    StatusCode::BAD_REQUEST
                }
            }),
        )
        .route(
            "/query-check",
            get(|Query(params): Query<QueryPayload>| async move { params.search }),
        )
}

#[tokio::test]
async fn test_get_status_and_text() {
    let client = ApiTestClient::new(test_router());
    let response = client.get("/health").send().await;

    let response = response.assert_status(StatusCode::OK);
    assert_eq!(response.text(), "OK");
}

#[tokio::test]
async fn test_post_json_serialization_and_deserialization() {
    let client = ApiTestClient::new(test_router());
    let payload = TestPayload {
        name: "Test User".to_string(),
        age: 30,
    };

    let response = client.post("/echo").json(&payload).send().await;

    let returned_payload: TestPayload = response.assert_status(StatusCode::OK).json();
    assert_eq!(payload, returned_payload);
}

#[tokio::test]
async fn test_bearer_token_injection() {
    let client = ApiTestClient::new(test_router());

    client
        .get("/auth-check")
        .send()
        .await
        .assert_status(StatusCode::UNAUTHORIZED);

    client
        .get("/auth-check")
        .bearer("my-secret-token")
        .send()
        .await
        .assert_status(StatusCode::OK);
}

#[tokio::test]
async fn test_custom_header_injection() {
    let client = ApiTestClient::new(test_router());

    client
        .get("/header-check")
        .header(
            axum::http::header::HeaderName::from_static("x-custom-header"),
            "value",
        )
        .send()
        .await
        .assert_status(StatusCode::OK);
}

#[tokio::test]
async fn test_query_params_appended() {
    let client = ApiTestClient::new(test_router());

    let response = client
        .get("/query-check")
        .query_param("search", "hello-world")
        .send()
        .await;

    let response = response.assert_status(StatusCode::OK);
    assert_eq!(response.text(), "hello-world");
}

#[tokio::test]
#[should_panic(expected = "Status mismatch. Expected: 200 OK, Actual: 404 Not Found")]
async fn test_assert_status_panics_readably() {
    let client = ApiTestClient::new(test_router());
    client
        .get("/non-existent")
        .send()
        .await
        .assert_status(StatusCode::OK);
}

#[tokio::test]
#[should_panic(expected = "Failed to deserialize response as")]
async fn test_json_deserialization_panics_with_body() {
    let client = ApiTestClient::new(test_router());
    // "/health" returns plaintext "OK", which will fail JSON parsing
    let _parsed: TestPayload = client.get("/health").send().await.json();
}
