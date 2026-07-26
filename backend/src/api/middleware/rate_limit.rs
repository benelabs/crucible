use axum::{
    extract::Request,
    http::{HeaderMap, HeaderValue, StatusCode},
    middleware::Next,
    response::{IntoResponse, Response},
};
use redis::AsyncCommands;
use std::{
    collections::HashMap,
    sync::{Arc, Mutex},
    time::{Duration, Instant, SystemTime, UNIX_EPOCH},
};

/// Configuration for Token Bucket Rate Limiter
#[derive(Debug, Clone)]
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

impl TokenBucketRateLimiter {
    pub fn new(redis_client: Option<redis::Client>, config: RateLimitConfig) -> Self {
        Self {
            redis_client,
            config,
            local_buckets: Arc::new(Mutex::new(HashMap::new())),
        }
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

                if let Ok((allowed_int, remaining, capacity)): Result<(i32, u64, u64), _> = script
                    .key(&key_name)
                    .arg(self.config.capacity)
                    .arg(self.config.refill_rate_per_sec)
                    .arg(now)
                    .invoke_async(&mut conn)
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
#[derive(Debug, Clone)]
pub struct RateLimitResult {
    pub allowed: bool,
    pub limit: u64,
    pub remaining: u64,
    pub reset_secs: u64,
}

/// Axum Middleware function for Token Bucket Rate Limiting
pub async fn rate_limit_middleware(
    request: Request,
    next: Next,
) -> Response {
    // Extract key (API key header or IP)
    let key = request
        .headers()
        .get("x-api-key")
        .and_then(|h| h.to_str().ok())
        .or_else(|| {
            request
                .headers()
                .get("x-forwarded-for")
                .and_then(|h| h.to_str().ok())
        })
        .unwrap_or("anonymous_client")
        .to_string();

    let limiter = TokenBucketRateLimiter::new(None, RateLimitConfig::default());

    match limiter.check_and_consume(&key).await {
        Ok(result) => {
            if !result.allowed {
                let mut headers = HeaderMap::new();
                headers.insert("X-RateLimit-Limit", HeaderValue::from(result.limit));
                headers.insert("X-RateLimit-Remaining", HeaderValue::from(result.remaining));
                headers.insert("X-RateLimit-Reset", HeaderValue::from(result.reset_secs));

                return (
                    StatusCode::TOO_MANY_REQUESTS,
                    headers,
                    "429 Too Many Requests: Rate limit exceeded",
                )
                    .into_response();
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
