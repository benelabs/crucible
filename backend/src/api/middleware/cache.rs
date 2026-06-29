use axum::{extract::Request, middleware::Next};
use redis::Client;
use std::time::Duration;

/// Cache key generator for HTTP requests
#[derive(Debug, Clone)]
pub struct CacheKeyGenerator;

impl CacheKeyGenerator {
    /// Generate a cache key from request
    pub fn generate_key(request: &Request) -> String {
        let method = request.method().as_str();
        let uri = request.uri().to_string();
        let query = request.uri().query().unwrap_or("");

        // Create hashable key
        format!("{}:{}:{}", method, uri, query)
    }
}

/// Axum middleware function
pub async fn cache_middleware(request: Request, next: Next) -> axum::response::Response {
    next.run(request).await
}

/// Cache middleware layer
#[derive(Debug, Clone)]
pub struct CacheLayer;

impl CacheLayer {
    pub fn new(_redis_client: Client, _default_ttl: Duration) -> Self {
        Self
    }
}

impl<S> tower::Layer<S> for CacheLayer {
    type Service = CacheService<S>;

    fn layer(&self, inner: S) -> Self::Service {
        CacheService { inner }
    }
}

/// Cache service that wraps the inner service
#[derive(Debug, Clone)]
pub struct CacheService<S> {
    inner: S,
}

impl<S, Req> tower::Service<Req> for CacheService<S>
where
    S: tower::Service<Req> + Clone + Send + 'static,
    S::Future: Send + 'static,
    Req: Send + 'static,
{
    type Response = S::Response;
    type Error = S::Error;
    type Future = std::pin::Pin<
        Box<dyn std::future::Future<Output = Result<Self::Response, Self::Error>> + Send + 'static>,
    >;

    fn poll_ready(
        &mut self,
        cx: &mut std::task::Context<'_>,
    ) -> std::task::Poll<Result<(), Self::Error>> {
        self.inner.poll_ready(cx)
    }

    fn call(&mut self, req: Req) -> Self::Future {
        let mut inner = self.inner.clone();
        Box::pin(async move { inner.call(req).await })
    }
}
