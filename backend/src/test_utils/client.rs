//! API TEST CLIENT APPROACH: Option A — builder pattern per request
//! Rationale: A builder pattern offers superior ergonomics, flexibility, and composition for integration tests.
//! It closely maps to familiar HTTP clients like `reqwest`, allowing testers to incrementally build requests
//! (adding headers, auth, query params) without dealing with massive function signatures or rigid method shortcuts.

use axum::{body::Body, Router};
use http::{header, Method, Request, StatusCode};
use http_body_util::BodyExt;
use serde::{de::DeserializeOwned, Serialize};
use std::fmt::Debug;
use tower::ServiceExt;

/// A reusable test client for making in-process HTTP requests against an Axum router.
///
/// This client clones the provided Axum `Router` for each request, allowing it to
/// execute HTTP requests rapidly in memory without binding to a TCP port.
#[derive(Clone, Debug)]
pub struct ApiTestClient {
    app: Router,
    #[allow(dead_code)]
    db_pool: Option<sqlx::PgPool>,
    #[allow(dead_code)]
    redis_client: Option<redis::Client>,
}

impl ApiTestClient {
    /// Creates a new test client wrapping an Axum router.
    ///
    /// # Example
    /// ```rust,ignore
    /// let app = backend::api::app();
    /// let client = ApiTestClient::new(app);
    /// ```
    pub fn new(app: Router) -> Self {
        Self {
            app,
            db_pool: None,
            redis_client: None,
        }
    }

    /// Creates a new test client with an attached database pool for state assertions.
    pub fn with_db(app: Router, pool: sqlx::PgPool) -> Self {
        Self {
            app,
            db_pool: Some(pool),
            redis_client: None,
        }
    }

    /// Creates a full test client with all external dependencies attached.
    pub fn with_redis(app: Router, pool: sqlx::PgPool, redis: redis::Client) -> Self {
        Self {
            app,
            db_pool: Some(pool),
            redis_client: Some(redis),
        }
    }

    /// Starts building a GET request to the specified path.
    pub fn get(&self, path: &str) -> RequestBuilder {
        RequestBuilder::new(self.app.clone(), Method::GET, path)
    }

    /// Starts building a POST request to the specified path.
    pub fn post(&self, path: &str) -> RequestBuilder {
        RequestBuilder::new(self.app.clone(), Method::POST, path)
    }

    /// Starts building a PUT request to the specified path.
    pub fn put(&self, path: &str) -> RequestBuilder {
        RequestBuilder::new(self.app.clone(), Method::PUT, path)
    }

    /// Starts building a PATCH request to the specified path.
    pub fn patch(&self, path: &str) -> RequestBuilder {
        RequestBuilder::new(self.app.clone(), Method::PATCH, path)
    }

    /// Starts building a DELETE request to the specified path.
    pub fn delete(&self, path: &str) -> RequestBuilder {
        RequestBuilder::new(self.app.clone(), Method::DELETE, path)
    }
}

/// A fluent builder for configuring and executing an in-process HTTP request.
pub struct RequestBuilder {
    app: Router,
    method: Method,
    path: String,
    body: Body,
    headers: Vec<(header::HeaderName, String)>,
    query_params: Vec<(String, String)>,
}

impl RequestBuilder {
    fn new(app: Router, method: Method, path: &str) -> Self {
        Self {
            app,
            method,
            path: path.to_string(),
            body: Body::empty(),
            headers: Vec::new(),
            query_params: Vec::new(),
        }
    }

    /// Serializes the provided value as JSON and sets the `Content-Type` header.
    ///
    /// # Example
    /// ```rust,ignore
    /// client.post("/api/users").json(&my_payload).send().await;
    /// ```
    pub fn json<T: Serialize>(mut self, body: &T) -> Self {
        let json_bytes = serde_json::to_vec(body).expect("Failed to serialize body to JSON");
        self.body = Body::from(json_bytes);
        self.headers
            .push((header::CONTENT_TYPE, "application/json".to_string()));
        self
    }

    /// Appends a Bearer token to the `Authorization` header.
    pub fn bearer(mut self, token: &str) -> Self {
        self.headers
            .push((header::AUTHORIZATION, format!("Bearer {}", token)));
        self
    }

    /// Appends a custom header to the request.
    pub fn header(mut self, key: header::HeaderName, value: &str) -> Self {
        self.headers.push((key, value.to_string()));
        self
    }

    /// Appends a query parameter to the URL.
    pub fn query_param(mut self, key: &str, value: &str) -> Self {
        self.query_params.push((key.to_string(), value.to_string()));
        self
    }

    /// Executes the request in-process using `tower::ServiceExt::oneshot` and returns a typed `TestResponse`.
    pub async fn send(self) -> TestResponse {
        let mut uri = self.path;

        if !self.query_params.is_empty() {
            let query_string = self
                .query_params
                .iter()
                .map(|(k, v)| format!("{}={}", k, v))
                .collect::<Vec<_>>()
                .join("&");
            uri = format!("{}?{}", uri, query_string);
        }

        let mut request_builder = Request::builder().method(self.method).uri(uri);

        for (key, value) in self.headers {
            request_builder = request_builder.header(key, value);
        }

        let request = request_builder
            .body(self.body)
            .expect("Failed to build request");

        let response = self
            .app
            .oneshot(request)
            .await
            .expect("Failed to execute oneshot request");

        let status = response.status();
        let body_bytes = response
            .into_body()
            .collect()
            .await
            .expect("Failed to read response body")
            .to_bytes();

        TestResponse {
            status,
            body: body_bytes.to_vec(),
        }
    }
}

/// A wrapper around an HTTP response tailored for integration testing assertions.
#[derive(Clone)]
pub struct TestResponse {
    status: StatusCode,
    body: Vec<u8>,
}

impl TestResponse {
    /// Returns the HTTP status code of the response.
    pub fn status(&self) -> StatusCode {
        self.status
    }

    /// Asserts that the response status matches the expected status code.
    /// Returns `self` for chaining.
    ///
    /// # Panics
    /// Panics with a readable message including the raw body if the status does not match.
    pub fn assert_status(self, expected: StatusCode) -> Self {
        if self.status != expected {
            let body_text = String::from_utf8_lossy(&self.body);
            panic!(
                "Status mismatch. Expected: {}, Actual: {}\nResponse Body: {}",
                expected, self.status, body_text
            );
        }
        self
    }

    /// Deserializes the response body into the target type `T`.
    ///
    /// # Panics
    /// Panics with the raw body shown if deserialization fails.
    pub fn json<T: DeserializeOwned>(self) -> T {
        match serde_json::from_slice::<T>(&self.body) {
            Ok(parsed) => parsed,
            Err(e) => {
                let body_text = String::from_utf8_lossy(&self.body);
                panic!(
                    "Failed to deserialize response as {}: {}\nActual body: {}",
                    std::any::type_name::<T>(),
                    e,
                    body_text
                );
            }
        }
    }

    /// Attempts to deserialize the response body, returning `None` if the body is empty.
    pub fn json_opt<T: DeserializeOwned>(self) -> Option<T> {
        if self.body.is_empty() {
            None
        } else {
            Some(self.json::<T>())
        }
    }

    /// Returns the raw response body as a UTF-8 string.
    pub fn text(self) -> String {
        String::from_utf8(self.body).expect("Response body contained invalid UTF-8")
    }

    /// Asserts that a specific JSON field matches the expected value.
    /// Note: This performs a conversion through `serde_json::Value`.
    pub fn assert_json_field<T: DeserializeOwned + PartialEq + Debug>(
        self,
        field: &str,
        expected: T,
    ) -> Self {
        let json_val: serde_json::Value = serde_json::from_slice(&self.body).unwrap_or_else(|e| {
            panic!(
                "Failed to parse body as JSON for field assertion: {}\nBody: {}",
                e,
                String::from_utf8_lossy(&self.body)
            )
        });

        let actual_val = json_val
            .pointer(&format!("/{}", field.replace('.', "/")))
            .unwrap_or_else(|| {
                panic!(
                    "Field '{}' not found in JSON body:\n{}",
                    field,
                    serde_json::to_string_pretty(&json_val).unwrap()
                );
            });

        let actual: T = serde_json::from_value(actual_val.clone()).unwrap_or_else(|e| {
            panic!(
                "Failed to deserialize field '{}' as {}: {}",
                field,
                std::any::type_name::<T>(),
                e
            );
        });

        assert_eq!(actual, expected, "JSON field '{}' mismatch", field);
        self
    }
}
