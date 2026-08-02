use wiremock::{matchers::{method, path}, Mock, MockServer, ResponseTemplate};

/// Reusable HTTP Mock Server Framework for Crucible integration testing.
///
/// This provides a wrapper around `wiremock::MockServer` to simplify mocking
/// external APIs during integration test runs.
pub struct MockHttpServer {
    server: MockServer,
}

impl MockHttpServer {
    /// Start a new mock HTTP server on a random local port.
    pub async fn start() -> Self {
        let server = MockServer::start().await;
        Self { server }
    }

    /// Get the mock server's base URL (e.g. "http://127.0.0.1:3485").
    pub fn uri(&self) -> String {
        self.server.uri()
    }

    /// Register a mock responder for GET requests on a specific path.
    pub async fn mock_get<T>(&self, path_str: &str, status: u16, response_body: &T)
    where
        T: serde::Serialize,
    {
        let body_json = serde_json::to_value(response_body).unwrap_or(serde_json::Value::Null);
        Mock::given(method("GET"))
            .and(path(path_str))
            .respond_with(ResponseTemplate::new(status).set_body_json(body_json))
            .mount(&self.server)
            .await;
    }

    /// Register a mock responder for POST requests on a specific path.
    pub async fn mock_post<T>(&self, path_str: &str, status: u16, response_body: &T)
    where
        T: serde::Serialize,
    {
        let body_json = serde_json::to_value(response_body).unwrap_or(serde_json::Value::Null);
        Mock::given(method("POST"))
            .and(path(path_str))
            .respond_with(ResponseTemplate::new(status).set_body_json(body_json))
            .mount(&self.server)
            .await;
    }

    /// Exposes the underlying `wiremock::MockServer` for advanced mock configurations.
    pub fn server(&self) -> &MockServer {
        &self.server
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use reqwest::StatusCode;

    #[tokio::test]
    async fn test_mock_http_server_get_and_post() {
        let mock_server = MockHttpServer::start().await;
        let test_response = serde_json::json!({ "status": "ok", "data": 42 });

        mock_server.mock_get("/v1/test", 200, &test_response).await;
        mock_server.mock_post("/v1/test", 201, &test_response).await;

        let client = reqwest::Client::new();

        let get_url = format!("{}/v1/test", mock_server.uri());
        let res = client.get(&get_url).send().await.unwrap();
        assert_eq!(res.status(), StatusCode::OK);
        let res_body: serde_json::Value = res.json().await.unwrap();
        assert_eq!(res_body["data"], 42);

        let post_url = format!("{}/v1/test", mock_server.uri());
        let res = client.post(&post_url).send().await.unwrap();
        assert_eq!(res.status(), StatusCode::CREATED);
        let res_body: serde_json::Value = res.json().await.unwrap();
        assert_eq!(res_body["status"], "ok");
    }
}
