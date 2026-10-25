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
    net::IpAddr,
};

/// Tiered rate limit quota configuration based on caller identity/tier
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub enum RateLimitTier {
    Anonymous,
    FreeTier,
    Standard,
    Enterprise,
}

impl RateLimitTier {
    pub fn config(&self) -> RateLimitConfig {
        match self {
            RateLimitTier::Anonymous => RateLimitConfig {
                capacity: 30,
                refill_rate_per_sec: 2,
                ttl: Duration::from_secs(3600),
            },
            RateLimitTier::FreeTier => RateLimitConfig {
                capacity: 100,
                refill_rate_per_sec: 10,
                ttl: Duration::from_secs(3600),
            },
            RateLimitTier::Standard => RateLimitConfig {
                capacity: 500,
                refill_rate_per_sec: 50,
                ttl: Duration::from_secs(3600),
            },
            RateLimitTier::Enterprise => RateLimitConfig {
                capacity: 2000,
                refill_rate_per_sec: 200,
                ttl: Duration::from_secs(3600),
            },
        }
    }
}

/// Configuration for Token Bucket Rate Limiter
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct RateLimitConfig {
    pub capacity: u64,
    pub refill_rate_per_sec: u64,
    pub ttl: Duration,
}

impl Default for RateLimitConfig {
    fn default() -> Self {
        RateLimitTier::FreeTier.config()
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

    /// Check and consume a token for a given key (IP address or API key) with custom or default config
    pub async fn check_and_consume(&self, key: &str) -> Result<RateLimitResult, String> {
        self.check_and_consume_with_config(key, &self.config).await
    }

    /// Check and consume a token with specific tiered RateLimitConfig
    pub async fn check_and_consume_with_config(&self, key: &str, config: &RateLimitConfig) -> Result<RateLimitResult, String> {
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
                    .arg(config.capacity)
                    .arg(config.refill_rate_per_sec)
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
            tokens: config.capacity,
            last_refill: now,
        });

        let elapsed = now.duration_since(bucket.last_refill).as_secs();
        if elapsed > 0 {
            bucket.tokens = (bucket.tokens + elapsed * config.refill_rate_per_sec)
                .min(config.capacity);
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
            limit: config.capacity,
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

/// Trusted proxy CIDR blocks (must be configured)
static TRUSTED_PROXIES: OnceLock<Vec<(u128, u128)>> = OnceLock::new();

/// Configure trusted proxy CIDR ranges. Call this during application initialization.
pub fn configure_trusted_proxies(cidrs: Vec<&str>) {
    let mut ranges = Vec::new();
    for cidr in cidrs {
        if let Ok((mask, prefix)) = parse_cidr(cidr) {
            ranges.push((mask, prefix));
        }
    }
    let _ = TRUSTED_PROXIES.set(ranges);
}

/// Parse CIDR notation into `(network, mask)` in a family-normalised `u128` space.
///
/// IPv4 addresses occupy the low 32 bits, so their mask must be built with a 32-bit
/// width; using the 128-bit width for IPv4 would zero the network half of every entry
/// and make every IPv4 address look trusted.
fn parse_cidr(cidr: &str) -> Result<(u128, u128), String> {
    let parts: Vec<&str> = cidr.split('/').collect();
    if parts.len() != 2 {
        return Err("Invalid CIDR format".to_string());
    }

    let ip_addr = parts[0]
        .parse::<std::net::IpAddr>()
        .map_err(|e| e.to_string())?;
    let prefix_len: u32 = parts[1]
        .parse()
        .map_err(|e: std::num::ParseIntError| e.to_string())?;

    let (width, ip_num) = match ip_addr {
        std::net::IpAddr::V4(v4) => (32u32, u128::from(u32::from(v4))),
        std::net::IpAddr::V6(v6) => (128u32, u128::from(v6)),
    };
    if prefix_len > width {
        return Err("CIDR prefix length out of range".to_string());
    }

    let full: u128 = if width == 128 {
        u128::MAX
    } else {
        (1u128 << width) - 1
    };
    let host_bits = width - prefix_len;
    let mask = if host_bits == 0 {
        full
    } else {
        full & !((1u128 << host_bits) - 1)
    };

    Ok((ip_num & mask, mask))
}

/// Returns true when `ip_str` falls inside one of the configured trusted proxy ranges.
fn is_trusted_proxy(ip_str: &str) -> bool {
    let Ok(ip_addr) = ip_str.parse::<std::net::IpAddr>() else {
        return false;
    };

    let ip_num = match ip_addr {
        std::net::IpAddr::V4(v4) => u128::from(u32::from(v4)),
        std::net::IpAddr::V6(v6) => u128::from(v6),
    };

    let trusted = TRUSTED_PROXIES.get_or_init(|| {
        // Default to common RFC 1918 private ranges if not configured
        [
            "10.0.0.0/8",
            "172.16.0.0/12",
            "192.168.0.0/16",
            "127.0.0.0/8",
            "::1/128",
        ]
        .iter()
        .filter_map(|cidr| parse_cidr(cidr).ok())
        .collect()
    });

    let is_v4 = ip_addr.is_ipv4();
    trusted.iter().any(|(network, mask)| {
        // An IPv4 entry can never describe an IPv6 range and vice versa.
        let entry_is_v4 = *mask <= u128::from(u32::MAX);
        entry_is_v4 == is_v4 && (ip_num & mask) == *network
    })
}

/// Extract the client IP from X-Forwarded-For, validating proxy trust
fn extract_client_ip_from_forwarded(forwarded_str: &str) -> Option<String> {
    let ips: Vec<&str> = forwarded_str
        .split(',')
        .map(|s| s.trim())
        .filter(|s| !s.is_empty())
        .collect();

    if ips.is_empty() {
        return None;
    }

    // Scan right to left, skipping hops we trust, and take the first untrusted one.
    for hop in ips.iter().rev() {
        if !is_trusted_proxy(hop) {
            return Some((*hop).to_string());
        }
    }

    // Every hop is trusted: keep the rightmost entry, which is the connection peer.
    ips.last().map(|ip| ip.to_string())
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

    // 3. Check X-Forwarded-For header with proxy trust validation (security fix)
    if let Some(forwarded) = headers.get("x-forwarded-for").and_then(|h| h.to_str().ok()) {
        if let Some(client_ip) = extract_client_ip_from_forwarded(forwarded) {
            if !client_ip.is_empty() {
                return format!("ip:{}", client_ip);
            }
        }
    }

    // 4. Check X-Real-IP header
    if let Some(real_ip) = headers.get("x-real-ip").and_then(|h| h.to_str().ok()) {
        let ip = real_ip.trim();
        if !ip.is_empty() && is_trusted_proxy(ip) {
            return format!("ip:{}", ip);
        }
    }

    "ip:anonymous".to_string()
}

/// Resolves the tier from request headers (checking subscription tier, API keys, or fallback to IP/anonymous)
pub fn resolve_rate_limit_tier(headers: &HeaderMap) -> RateLimitTier {
    if let Some(tier_str) = headers.get("x-subscription-tier").and_then(|h| h.to_str().ok()) {
        match tier_str.to_lowercase().as_str() {
            "enterprise" => return RateLimitTier::Enterprise,
            "standard" | "pro" => return RateLimitTier::Standard,
            "free" => return RateLimitTier::FreeTier,
            _ => {}
        }
    }

    if headers.contains_key("x-api-key") || headers.contains_key("authorization") {
        RateLimitTier::Standard
    } else if headers.contains_key("x-forwarded-for") || headers.contains_key("x-real-ip") {
        RateLimitTier::FreeTier
    } else {
        RateLimitTier::Anonymous
    }
}

/// Axum Middleware function for Token Bucket Rate Limiting with IP & API Key Tiering
pub async fn rate_limit_middleware(
    request: Request,
    next: Next,
) -> Response {
    let key = extract_rate_limit_key(request.headers());
    let tier = resolve_rate_limit_tier(request.headers());

    // A limiter supplied with the request owns its bucket configuration; the shared
    // default falls back to the caller's tier quota.
    let (limiter, tier_config) = match request
        .extensions()
        .get::<Arc<TokenBucketRateLimiter>>()
        .cloned()
    {
        Some(limiter) => {
            let config = limiter.config().clone();
            (limiter, config)
        }
        None => (
            TokenBucketRateLimiter::global(),
            tier.config(),
        ),
    };

    match limiter.check_and_consume_with_config(&key, &tier_config).await {
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

    #[test]
    fn test_parse_cidr_ipv4() {
        let (mask, prefix) = parse_cidr("192.168.1.0/24").unwrap();
        assert!(mask > 0);
        assert!(prefix > 0);
    }

    #[test]
    fn test_parse_cidr_ipv6() {
        let (mask, prefix) = parse_cidr("2001:db8::/32").unwrap();
        assert_eq!(prefix, u128::MAX << (128 - 32));
    }

    #[test]
    fn test_is_trusted_proxy_private_ranges() {
        assert!(is_trusted_proxy("127.0.0.1"));
        assert!(is_trusted_proxy("192.168.1.1"));
        assert!(is_trusted_proxy("10.0.0.1"));
        assert!(is_trusted_proxy("172.16.0.1"));
        assert!(!is_trusted_proxy("8.8.8.8"));
        assert!(!is_trusted_proxy("1.1.1.1"));
    }

    #[test]
    fn test_extract_client_ip_single_trusted_proxy() {
        let result = extract_client_ip_from_forwarded("10.0.0.1");
        assert_eq!(result, Some("10.0.0.1".to_string()));
    }

    #[test]
    fn test_extract_client_ip_untrusted_client_trusted_proxy() {
        let result = extract_client_ip_from_forwarded("203.0.113.1, 192.168.1.1");
        assert_eq!(result, Some("203.0.113.1".to_string()));
    }

    #[test]
    fn test_extract_client_ip_trusted_proxy_chain() {
        let result = extract_client_ip_from_forwarded("192.168.1.1, 10.0.0.1, 127.0.0.1");
        assert_eq!(result, Some("127.0.0.1".to_string()));
    }

    #[test]
    fn test_extract_client_ip_complex_chain() {
        let result = extract_client_ip_from_forwarded("203.0.113.5, 198.51.100.1, 10.0.0.1");
        assert!(result.is_some());
    }

    #[test]
    fn test_extract_rate_limit_key_priority() {
        let mut headers = HeaderMap::new();
        assert_eq!(extract_rate_limit_key(&headers), "ip:anonymous");

        headers.insert("x-forwarded-for", HeaderValue::from_static("203.0.113.5, 10.0.0.1"));
        assert!(extract_rate_limit_key(&headers).contains("203.0.113.5"));

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

        let req1 = Request::builder()
            .uri("/test")
            .header("x-api-key", "client-1")
            .body(Body::empty())
            .unwrap();
        let res1 = app.clone().oneshot(req1).await.unwrap();
        assert_eq!(res1.status(), StatusCode::OK);

        let req2 = Request::builder()
            .uri("/test")
            .header("x-api-key", "client-1")
            .body(Body::empty())
            .unwrap();
        let res2 = app.clone().oneshot(req2).await.unwrap();
        assert_eq!(res2.status(), StatusCode::OK);

        let req3 = Request::builder()
            .uri("/test")
            .header("x-api-key", "client-1")
            .body(Body::empty())
            .unwrap();
        let res3 = app.oneshot(req3).await.unwrap();
        assert_eq!(res3.status(), StatusCode::TOO_MANY_REQUESTS);
    }

    #[tokio::test]
    async fn test_rate_limit_tiering_quota_enforcement() {
        let mut headers_anon = HeaderMap::new();
        assert_eq!(resolve_rate_limit_tier(&headers_anon), RateLimitTier::Anonymous);

        let mut headers_api = HeaderMap::new();
        headers_api.insert("x-api-key", HeaderValue::from_static("key-123"));
        assert_eq!(resolve_rate_limit_tier(&headers_api), RateLimitTier::Standard);

        let mut headers_ent = HeaderMap::new();
        headers_ent.insert("x-subscription-tier", HeaderValue::from_static("enterprise"));
        assert_eq!(resolve_rate_limit_tier(&headers_ent), RateLimitTier::Enterprise);
    }
}
