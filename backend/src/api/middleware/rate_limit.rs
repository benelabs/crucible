use axum::{
    extract::Request,
    http::{HeaderMap, HeaderValue, StatusCode},
    middleware::Next,
    response::{IntoResponse, Response},
};
use serde::{Deserialize, Serialize};
use std::{
    collections::HashMap,
    sync::{Arc, Mutex, OnceLock},
    time::{Duration, Instant, SystemTime, UNIX_EPOCH},
};

/// Configuration for Token Bucket Rate Limiter
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct RateLimitConfig {
    pub capacity: u64,
    pub refill_rate_per_sec: u64,
    pub ttl: Duration,
}

impl Default for RateLimitConfig {
    fn default() -> Self {
        Self {
            capacity: 100,
            refill_rate_per_sec: 10,
            ttl: Duration::from_secs(3600),
        }
    }
}

/// In-memory token bucket state used for fallback or direct state tracking
#[derive(Debug, Clone)]
pub struct LocalTokenBucket {
    pub tokens: u64,
    pub last_refill: Instant,
}

/// Redis-backed Token Bucket Rate Limiter with Local Memory Fallback
#[derive(Debug, Clone)]
pub struct TokenBucketRateLimiter {
    redis_client: Option<redis::Client>,
    config: RateLimitConfig,
    local_buckets: Arc<Mutex<HashMap<String, LocalTokenBucket>>>,
}

static SHARED_LIMITER: OnceLock<Arc<TokenBucketRateLimiter>> = OnceLock::new();

impl TokenBucketRateLimiter {
    pub fn new(redis_client: Option<redis::Client>, config: RateLimitConfig) -> Self {
        Self {
            redis_client,
            config,
            local_buckets: Arc::new(Mutex::new(HashMap::new())),
        }
    }

    /// Get global shared rate limiter instance
    pub fn global() -> Arc<Self> {
        SHARED_LIMITER
            .get_or_init(|| Arc::new(Self::new(None, RateLimitConfig::default())))
            .clone()
    }

    pub fn config(&self) -> &RateLimitConfig {
        &self.config
    }

    /// Check and consume a token for a given key (IP address or API key)
    pub async fn check_and_consume(&self, key: &str) -> Result<RateLimitResult, String> {
        let key_name = format!("rate_limit:{}", key);

        if let Some(client) = &self.redis_client {
            if let Ok(mut conn) = client.get_multiplexed_async_connection().await {
                let now = SystemTime::now()
                    .duration_since(UNIX_EPOCH)
                    .unwrap_or_default()
                    .as_secs();

                // Token Bucket Script in Redis Lua
                let script = redis::Script::new(
                    r#"
                    local key = KEYS[1]
                    local capacity = tonumber(ARGV[1])
                    local refill_rate = tonumber(ARGV[2])
                    local now = tonumber(ARGV[3])
                    local requested = 1

                    local data = redis.call("HMGET", key, "tokens", "last_refill")
                    local tokens = tonumber(data[1])
                    local last_refill = tonumber(data[2])

                    if not tokens or not last_refill then
                        tokens = capacity
                        last_refill = now
                    else
                        local delta = math.max(0, now - last_refill)
                        tokens = math.min(capacity, tokens + (delta * refill_rate))
                        last_refill = now
                    end

                    local allowed = false
                    local remaining = tokens
                    if tokens >= requested then
                        allowed = true
                        tokens = tokens - requested
                        remaining = tokens
                        redis.call("HMSET", key, "tokens", tokens, "last_refill", last_refill)
                        redis.call("EXPIRE", key, 3600)
                    end

                    return { allowed and 1 or 0, remaining, capacity }
                    "#,
                );

                if let Ok((allowed_int, remaining, capacity)) = script
                    .key(&key_name)
                    .arg(self.config.capacity)
                    .arg(self.config.refill_rate_per_sec)
                    .arg(now)
                    .invoke_async::<(i32, u64, u64)>(&mut conn)
                    .await
                {
                    return Ok(RateLimitResult {
                        allowed: allowed_int == 1,
                        limit: capacity,
                        remaining,
                        reset_secs: 1,
                    });
                }
            }
        }

        // Local Memory Fallback Rate Limiting
        let mut buckets = self
            .local_buckets
            .lock()
            .map_err(|e| format!("Lock error: {}", e))?;

        let now = Instant::now();
        let bucket = buckets.entry(key.to_string()).or_insert(LocalTokenBucket {
            tokens: self.config.capacity,
            last_refill: now,
        });

        let elapsed = now.duration_since(bucket.last_refill).as_secs();
        if elapsed > 0 {
            bucket.tokens = (bucket.tokens + elapsed * self.config.refill_rate_per_sec)
                .min(self.config.capacity);
            bucket.last_refill = now;
        }

        let allowed = if bucket.tokens >= 1 {
            bucket.tokens -= 1;
            true
        } else {
            false
        };

        Ok(RateLimitResult {
            allowed,
            limit: self.config.capacity,
            remaining: bucket.tokens,
            reset_secs: 1,
        })
    }
}

/// Result of Rate Limit evaluation
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct RateLimitResult {
    pub allowed: bool,
    pub limit: u64,
    pub remaining: u64,
    pub reset_secs: u64,
}

/// Extract rate limiting key (API Key token or IP address) from request headers
pub fn extract_rate_limit_key(headers: &HeaderMap) -> String {
    // 1. Check API Key header
    if let Some(key) = headers.get("x-api-key").and_then(|h| h.to_str().ok()) {
        if !key.trim().is_empty() {
            return format!("token:{}", key.trim());
        }
    }

    // 2. Check Authorization header (Bearer token)
    if let Some(auth) = headers.get("authorization").and_then(|h| h.to_str().ok()) {
        if auth.to_lowercase().starts_with("bearer ") {
            let token = auth[7..].trim();
            if !token.is_empty() {
                return format!("token:{}", token);
            }
        }
    }

    // 3. Check X-Forwarded-For header (first client IP)
    if let Some(forwarded) = headers.get("x-forwarded-for").and_then(|h| h.to_str().ok()) {
        if let Some(first_ip) = forwarded.split(',').next() {
            let ip = first_ip.trim();
            if !ip.is_empty() {
                return format!("ip:{}", ip);
            }
        }
    }

    // 4. Check X-Real-IP header
    if let Some(real_ip) = headers.get("x-real-ip").and_then(|h| h.to_str().ok()) {
        let ip = real_ip.trim();
        if !ip.is_empty() {
            return format!("ip:{}", ip);
        }
    }

    "ip:anonymous".to_string()
}

/// Axum Middleware function for Token Bucket Rate Limiting
pub async fn rate_limit_middleware(
    request: Request,
    next: Next,
) -> Response {
    let key = extract_rate_limit_key(request.headers());

    let limiter = request
        .extensions()
        .get::<Arc<TokenBucketRateLimiter>>()
        .cloned()
        .unwrap_or_else(TokenBucketRateLimiter::global);

    match limiter.check_and_consume(&key).await {
        Ok(result) => {
            if !result.allowed {
                let mut response = crate::api::errors::make_error_response(
                    StatusCode::TOO_MANY_REQUESTS,
                    "too_many_requests",
                    "429 Too Many Requests: Rate limit exceeded",
                );
                let response_headers = response.headers_mut();
                response_headers.insert("X-RateLimit-Limit", HeaderValue::from(result.limit));
                response_headers.insert("X-RateLimit-Remaining", HeaderValue::from(result.remaining));
                response_headers.insert("X-RateLimit-Reset", HeaderValue::from(result.reset_secs));
                response_headers.insert("Retry-After", HeaderValue::from(result.reset_secs));
                return response;
            }

            let mut response = next.run(request).await;
            let headers = response.headers_mut();
            headers.insert("X-RateLimit-Limit", HeaderValue::from(result.limit));
            headers.insert("X-RateLimit-Remaining", HeaderValue::from(result.remaining));
            headers.insert("X-RateLimit-Reset", HeaderValue::from(result.reset_secs));
            response
        }
        Err(_) => next.run(request).await,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use axum::{body::Body, routing::get, Router};
    use tower::ServiceExt;

    #[tokio::test]
    async fn test_extract_rate_limit_key_priority() {
        let mut headers = HeaderMap::new();
        assert_eq!(extract_rate_limit_key(&headers), "ip:anonymous");

        headers.insert("x-real-ip", HeaderValue::from_static("192.168.1.1"));
        assert_eq!(extract_rate_limit_key(&headers), "ip:192.168.1.1");

        headers.insert("x-forwarded-for", HeaderValue::from_static("10.0.0.1, 10.0.0.2"));
        assert_eq!(extract_rate_limit_key(&headers), "ip:10.0.0.1");

        headers.insert("authorization", HeaderValue::from_static("Bearer secret-jwt"));
        assert_eq!(extract_rate_limit_key(&headers), "token:secret-jwt");

        headers.insert("x-api-key", HeaderValue::from_static("api-key-123"));
        assert_eq!(extract_rate_limit_key(&headers), "token:api-key-123");
    }

    #[tokio::test]
    async fn test_rate_limit_middleware_429_too_many_requests() {
        let config = RateLimitConfig {
            capacity: 2,
            refill_rate_per_sec: 0,
            ttl: Duration::from_secs(60),
        };
        let limiter = Arc::new(TokenBucketRateLimiter::new(None, config));

        let app = Router::new()
            .route("/test", get(|| async { "OK" }))
            .layer(axum::middleware::from_fn(rate_limit_middleware))
            .layer(axum::Extension(limiter));

        // Request 1: 200 OK
        let req1 = Request::builder()
            .uri("/test")
            .header("x-api-key", "client-1")
            .body(Body::empty())
            .unwrap();
        let res1 = app.clone().oneshot(req1).await.unwrap();
        assert_eq!(res1.status(), StatusCode::OK);
        assert_eq!(res1.headers().get("X-RateLimit-Remaining").unwrap(), "1");

        // Request 2: 200 OK
        let req2 = Request::builder()
            .uri("/test")
            .header("x-api-key", "client-1")
            .body(Body::empty())
            .unwrap();
        let res2 = app.clone().oneshot(req2).await.unwrap();
        assert_eq!(res2.status(), StatusCode::OK);
        assert_eq!(res2.headers().get("X-RateLimit-Remaining").unwrap(), "0");

        // Request 3: 429 Too Many Requests
        let req3 = Request::builder()
            .uri("/test")
            .header("x-api-key", "client-1")
            .body(Body::empty())
            .unwrap();
        let res3 = app.oneshot(req3).await.unwrap();
        assert_eq!(res3.status(), StatusCode::TOO_MANY_REQUESTS);
        assert!(res3.headers().contains_key("X-RateLimit-Limit"));
        assert!(res3.headers().contains_key("Retry-After"));
    }
}
