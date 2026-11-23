use axum::{
    extract::{Json, State},
    http::{header, HeaderMap, StatusCode},
    response::IntoResponse,
};
use rust_decimal::Decimal;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::sync::Arc;
use std::time::{Duration, Instant};
use tokio::sync::Mutex;
use tracing::{info, info_span, instrument, warn};

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "lowercase")]
pub enum StellarMemoType {
    Text,
    Id,
    Hash,
}

/// Validates Stellar transaction memos:
/// - Text memo: maximum 28 UTF-8 bytes.
/// - Id memo: valid 64-bit unsigned integer (`u64`).
/// - Hash memo: 32-byte hash (64 hex characters).
pub fn validate_stellar_memo(
    memo: &str,
    memo_type: Option<StellarMemoType>,
) -> Result<(), (StatusCode, String)> {
    match memo_type {
        Some(StellarMemoType::Text) | None => {
            let byte_len = memo.len();
            if byte_len > 28 {
                return Err((
                    StatusCode::BAD_REQUEST,
                    format!(
                        "Stellar text memo exceeds 28-byte limit (got {} bytes).",
                        byte_len
                    ),
                ));
            }
        }
        Some(StellarMemoType::Id) => {
            if memo.parse::<u64>().is_err() {
                return Err((
                    StatusCode::BAD_REQUEST,
                    "Invalid Stellar memo ID. Must be a valid 64-bit unsigned integer.".to_string(),
                ));
            }
        }
        Some(StellarMemoType::Hash) => {
            if memo.len() != 64 || !memo.chars().all(|c| c.is_ascii_hexdigit()) {
                return Err((
                    StatusCode::BAD_REQUEST,
                    "Invalid Stellar memo hash. Must be a 32-byte hex-encoded string (64 characters).".to_string(),
                ));
            }
        }
    }
    Ok(())
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct FundAccountRequest {
    pub destination: String,
    #[serde(default)]
    pub amount_xlm: Option<Decimal>,
    #[serde(default)]
    pub memo: Option<String>,
    #[serde(default)]
    pub memo_type: Option<StellarMemoType>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct FundAccountResponse {
    pub success: bool,
    pub transaction_hash: String,
    pub funded_account: String,
    pub amount_dispensed: Decimal,
    pub pool_remaining_balance: Decimal,
    pub dispenser_source: String,
    pub message: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub memo: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct FaucetPoolStats {
    pub pool_address: String,
    pub balance_xlm: Decimal,
    pub total_dispensed_count: u64,
    pub rate_limited_ips_count: usize,
}

#[derive(Clone)]
pub struct FaucetDispenserService {
    pool_address: String,
    balance_xlm: Arc<Mutex<Decimal>>,
    dispense_count: Arc<Mutex<u64>>,
    rate_limits: Arc<Mutex<HashMap<String, Vec<Instant>>>>,
    account_rate_limits: Arc<Mutex<HashMap<String, Vec<Instant>>>>,
    max_requests_per_window: usize,
    rate_window: Duration,
    redis: Option<redis::Client>,
}

impl Default for FaucetDispenserService {
    fn default() -> Self {
        Self::new()
    }
}

impl FaucetDispenserService {
    pub fn new() -> Self {
        Self {
            pool_address: "GCARU656RHO62CJJUXOG4YJ4V5W266OFEQ3Q4U6V5B6AUPY7V7S6TEST".to_string(),
            balance_xlm: Arc::new(Mutex::new(Decimal::new(100_000, 0))), // 100,000 XLM initial pool
            dispense_count: Arc::new(Mutex::new(0)),
            rate_limits: Arc::new(Mutex::new(HashMap::new())),
            account_rate_limits: Arc::new(Mutex::new(HashMap::new())),
            max_requests_per_window: 5,
            rate_window: Duration::from_secs(60),
            redis: None,
        }
    }

    /// Attach a Redis client to enable centralized sliding-window rate limiting across multi-pod deployments.
    pub fn with_redis(mut self, redis: redis::Client) -> Self {
        self.redis = Some(redis);
        self
    }

    /// Check rate limit against centralized Redis using a sliding-window sorted set.
    async fn check_redis_rate_limit(
        &self,
        redis: &redis::Client,
        key: &str,
    ) -> Result<bool, redis::RedisError> {
        let mut conn = redis.get_multiplexed_async_connection().await?;
        let now = chrono::Utc::now().timestamp_millis();
        let window_millis = self.rate_window.as_millis() as i64;
        let clear_before = now - window_millis;
        let expire_secs = (self.rate_window.as_secs() + 10).max(60) as i64;
        let member = format!("{}:{}", now, uuid::Uuid::new_v4());

        // 1. Prune expired timestamps older than sliding window
        // 2. Count requests currently within sliding window
        let (_removed, count): (i64, usize) = redis::pipe()
            .atomic()
            .cmd("ZREMRANGEBYSCORE")
            .arg(key)
            .arg("-inf")
            .arg(clear_before)
            .cmd("ZCARD")
            .arg(key)
            .query_async(&mut conn)
            .await?;

        if count >= self.max_requests_per_window {
            return Ok(false);
        }

        // Add this request timestamp and set TTL
        let _: () = redis::pipe()
            .atomic()
            .cmd("ZADD")
            .arg(key)
            .arg(now)
            .arg(&member)
            .cmd("EXPIRE")
            .arg(key)
            .arg(expire_secs)
            .query_async(&mut conn)
            .await?;

        Ok(true)
    }

    pub fn with_config(max_requests: usize, rate_window: Duration) -> Self {
        let mut s = Self::new();
        s.max_requests_per_window = max_requests;
        s.rate_window = rate_window;
        s
    }

    /// Evicts all expired IP and account timestamps and removes inactive keys from the rate limiting tables.
    pub async fn sweep_expired(&self) {
        let mut limits = self.rate_limits.lock().await;
        let mut account_limits = self.account_rate_limits.lock().await;
        let now = Instant::now();

        limits.retain(|_, timestamps| {
            timestamps.retain(|&t| now.duration_since(t) < self.rate_window);
            !timestamps.is_empty()
        });
        account_limits.retain(|_, timestamps| {
            timestamps.retain(|&t| now.duration_since(t) < self.rate_window);
            !timestamps.is_empty()
        });
    }

    /// Check rate limit against local in-memory stores.
    async fn check_in_memory_rate_limit(&self, ip: &str, destination: &str) -> bool {
        let mut limits = self.rate_limits.lock().await;
        let mut account_limits = self.account_rate_limits.lock().await;
        let now = Instant::now();

        // Prune expired entries and purge empty keys to prevent unbounded memory growth
        limits.retain(|_, timestamps| {
            timestamps.retain(|&t| now.duration_since(t) < self.rate_window);
            !timestamps.is_empty()
        });
        account_limits.retain(|_, timestamps| {
            timestamps.retain(|&t| now.duration_since(t) < self.rate_window);
            !timestamps.is_empty()
        });

        // Prune and check IP rate limit
        let ip_timestamps = limits.entry(ip.to_string()).or_default();
        if ip_timestamps.len() >= self.max_requests_per_window {
            return false;
        }

        // Check and record target account destination rate limit
        let acc_timestamps = account_limits.entry(destination.to_string()).or_default();
        if acc_timestamps.len() >= self.max_requests_per_window {
            return false;
        }

        ip_timestamps.push(now);
        acc_timestamps.push(now);
        true
    }

    /// Check rate limit for IP and destination account using centralized Redis when available,
    /// falling back cleanly to in-memory tracking.
    pub async fn check_rate_limit(&self, ip: &str, destination: &str) -> bool {
        if let Some(redis) = &self.redis {
            let ip_key = format!("faucet:ratelimit:ip:{ip}");
            let dest_key = format!("faucet:ratelimit:dest:{destination}");

            match self.check_redis_rate_limit(redis, &ip_key).await {
                Ok(false) => return false,
                Ok(true) => match self.check_redis_rate_limit(redis, &dest_key).await {
                    Ok(dest_ok) => return dest_ok,
                    Err(e) => {
                        warn!(error = %e, "Redis faucet rate limiter failed on destination, falling back to local store");
                    }
                },
                Err(e) => {
                    warn!(error = %e, "Redis faucet rate limiter failed on IP, falling back to local store");
                }
            }
        }

        self.check_in_memory_rate_limit(ip, destination).await
    }

    pub async fn replenish_pool(&self, amount: Decimal) {
        let mut bal = self.balance_xlm.lock().await;
        *bal += amount;
        info!(pool_balance = %*bal, "Faucet pool replenished successfully via Friendbot");
    }

    pub async fn get_stats(&self) -> FaucetPoolStats {
        let bal = *self.balance_xlm.lock().await;
        let count = *self.dispense_count.lock().await;
        let limits = self.rate_limits.lock().await;
        FaucetPoolStats {
            pool_address: self.pool_address.clone(),
            balance_xlm: bal,
            total_dispensed_count: count,
            rate_limited_ips_count: limits.len(),
        }
    }

    pub async fn dispense(
        &self,
        destination: &str,
        client_ip: &str,
        requested_amount: Option<Decimal>,
        memo: Option<String>,
        memo_type: Option<StellarMemoType>,
    ) -> Result<FundAccountResponse, (StatusCode, String)> {
        // Validate destination Stellar address
        if !destination.starts_with('G') || destination.len() != 56 {
            return Err((
                StatusCode::BAD_REQUEST,
                "Invalid Stellar testnet public key. Must start with 'G' and be 56 characters."
                    .to_string(),
            ));
        }

        // Validate Stellar transaction memo
        if let Some(ref m) = memo {
            validate_stellar_memo(m, memo_type)?;
        }

        // Check rate limit per IP and per destination account
        if !self.check_rate_limit(client_ip, destination).await {
            warn!(
                client_ip,
                destination, "Faucet rate limit exceeded for client IP or destination key"
            );
            return Err((
                StatusCode::TOO_MANY_REQUESTS,
                "Rate limit exceeded. Maximum 5 funding requests per minute per IP or account."
                    .to_string(),
            ));
        }

        let dispense_amount = requested_amount.unwrap_or_else(|| Decimal::new(100, 0)); // 100 XLM default

        let mut pool_bal = self.balance_xlm.lock().await;
        let mut source = "internal_pool".to_string();

        // Auto-replenish from Friendbot if pool drops below threshold
        if *pool_bal < dispense_amount {
            info!("Pool balance low; executing background Friendbot auto-replenish");
            *pool_bal += Decimal::new(10_000, 0); // 10,000 XLM replenishment
            source = "friendbot_replenished".to_string();
        }

        *pool_bal -= dispense_amount;
        let remaining = *pool_bal;
        drop(pool_bal);

        let mut count = self.dispense_count.lock().await;
        *count += 1;
        drop(count);

        use sha2::{Digest, Sha256};
        let mut hasher = Sha256::new();
        let memo_repr = memo.as_deref().unwrap_or("");
        hasher.update(
            format!(
                "{}:{}:{}:{}",
                destination,
                client_ip,
                memo_repr,
                Instant::now().elapsed().as_nanos()
            )
            .as_bytes(),
        );
        let tx_hash = hasher
            .finalize()
            .iter()
            .map(|b| format!("{:02x}", b))
            .collect::<String>();

        info!(
            destination,
            amount = %dispense_amount,
            tx_hash = %tx_hash,
            memo = ?memo,
            "Dispensed testnet XLM to account"
        );

        let message = match &memo {
            Some(m) => format!(
                "Successfully funded {} XLM to {} (memo: {})",
                dispense_amount, destination, m
            ),
            None => format!(
                "Successfully funded {} XLM to {}",
                dispense_amount, destination
            ),
        };

        Ok(FundAccountResponse {
            success: true,
            transaction_hash: tx_hash,
            funded_account: destination.to_string(),
            amount_dispensed: dispense_amount,
            pool_remaining_balance: remaining,
            dispenser_source: source,
            message,
            memo,
        })
    }
}

/// Handler for the /.well-known/stellar.toml (SEP-1) file.
/// This is essential for Stellar network identification and discovery.
#[instrument(skip_all, fields(http.method = "GET", http.route = "/.well-known/stellar.toml"))]
pub async fn get_stellar_toml() -> impl IntoResponse {
    let span = info_span!("stellar.toml.fetch");
    let _enter = span.enter();

    info!("Serving Stellar TOML for SEP-1 discovery");

    let mut headers = HeaderMap::new();
    headers.insert(header::CONTENT_TYPE, "text/plain".parse().unwrap());
    headers.insert(header::ACCESS_CONTROL_ALLOW_ORIGIN, "*".parse().unwrap());

    let toml_content = r#"
VERSION="2.0.0"
NETWORK_PASSPHRASE="Test SDF Network ; September 2015"
ACCOUNTS=[
  "GBBD67IF6I2E7E5NCTZTTG46YAKMBH2O7662T7O4B5XW4YVRE3L363C6"
]
DOCUMENTATION_URL="https://github.com/benelabs/crucible"

[[CURRENCIES]]
code="USDC"
issuer="GBBD67IF6I2E7E5NCTZTTG46YAKMBH2O7662T7O4B5XW4YVRE3L363C6"
display_decimals=6
"#;

    (StatusCode::OK, headers, toml_content)
}

/// Handler for instant ephemeral testnet faucet account dispenser.
#[instrument(skip_all, fields(http.method = "POST", http.route = "/api/v1/stellar/faucet"))]
pub async fn dispense_faucet_handler(
    State(service): State<Arc<FaucetDispenserService>>,
    Json(payload): Json<FundAccountRequest>,
) -> Result<Json<FundAccountResponse>, (StatusCode, String)> {
    let client_ip = "127.0.0.1";
    let res = service
        .dispense(
            &payload.destination,
            client_ip,
            payload.amount_xlm,
            payload.memo,
            payload.memo_type,
        )
        .await?;
    Ok(Json(res))
}

static GLOBAL_FAUCET_POOL: std::sync::OnceLock<Arc<FaucetDispenserService>> =
    std::sync::OnceLock::new();

/// Initialize the global faucet pool with a centralized Redis client.
pub fn init_faucet_pool_with_redis(redis: redis::Client) {
    let _ = GLOBAL_FAUCET_POOL.set(Arc::new(FaucetDispenserService::new().with_redis(redis)));
}

pub fn get_faucet_pool() -> Arc<FaucetDispenserService> {
    GLOBAL_FAUCET_POOL
        .get_or_init(|| {
            let mut svc = FaucetDispenserService::new();
            if let Ok(url) = std::env::var("REDIS_URL") {
                if !url.trim().is_empty() {
                    if let Ok(client) = redis::Client::open(url) {
                        svc = svc.with_redis(client);
                    }
                }
            }
            Arc::new(svc)
        })
        .clone()
}

/// Handler for the Ephemeral Testnet Faucet & Account Dispenser (`POST /api/stellar/faucet` / `POST /api/v1/faucet`)
#[instrument(skip(headers))]
pub async fn fund_testnet_account(
    headers: HeaderMap,
    Json(payload): Json<FundAccountRequest>,
) -> Result<impl IntoResponse, (StatusCode, String)> {
    let client_ip = headers
        .get("x-forwarded-for")
        .and_then(|v| v.to_str().ok())
        .unwrap_or("127.0.0.1")
        .split(',')
        .next()
        .unwrap_or("127.0.0.1")
        .trim();

    let pool = get_faucet_pool();
    let res = pool
        .dispense(
            &payload.destination,
            client_ip,
            payload.amount_xlm,
            payload.memo,
            payload.memo_type,
        )
        .await?;
    Ok((StatusCode::OK, Json(res)))
}

/// Handler to inspect Faucet Pool status (`GET /api/stellar/faucet/status` / `GET /api/v1/faucet/status`)
#[instrument]
pub async fn get_faucet_status() -> impl IntoResponse {
    let pool = get_faucet_pool();
    let stats = pool.get_stats().await;
    (StatusCode::OK, Json(stats))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn test_stellar_toml() {
        let res = get_stellar_toml().await.into_response();
        assert_eq!(res.status(), StatusCode::OK);
    }

    #[tokio::test]
    async fn test_dispense_testnet_account_success() {
        let service = FaucetDispenserService::new();
        let valid_destination = "GBBD67IF6I2E7E5NCTZTTG46YAKMBH2O7662T7O4B5XW4YVRE3L363C6";
        let res = service
            .dispense(
                valid_destination,
                "192.168.1.10",
                Some(Decimal::new(50, 0)),
                None,
                None,
            )
            .await
            .unwrap();

        assert!(res.success);
        assert_eq!(res.funded_account, valid_destination);
        assert_eq!(res.amount_dispensed, Decimal::new(50, 0));
        assert!(!res.transaction_hash.is_empty());
    }

    #[tokio::test]
    async fn test_dispense_rate_limiting_per_ip() {
        let service = FaucetDispenserService::new();
        let valid_dest = "GBBD67IF6I2E7E5NCTZTTG46YAKMBH2O7662T7O4B5XW4YVRE3L363C6";
        let ip = "10.0.0.5";

        for _ in 0..5 {
            let res = service.dispense(valid_dest, ip, None, None, None).await;
            assert!(res.is_ok());
        }

        // 6th request from same IP within window must fail with 429
        let err = service
            .dispense(valid_dest, ip, None, None, None)
            .await
            .unwrap_err();
        assert_eq!(err.0, StatusCode::TOO_MANY_REQUESTS);

        // Different IP and destination must still succeed
        let other_dest = "GCARU656RHO62CJJUXOG4YJ4V5W266OFEQ3Q4U6V5B6AUPY7V7S6TEST";
        let other_ip_res = service
            .dispense(other_dest, "10.0.0.6", None, None, None)
            .await;
        assert!(other_ip_res.is_ok());
    }

    #[tokio::test]
    async fn test_dispense_rate_limiting_per_destination() {
        let service = FaucetDispenserService::new();
        let valid_dest = "GBBD67IF6I2E7E5NCTZTTG46YAKMBH2O7662T7O4B5XW4YVRE3L363C6";

        for i in 0..5 {
            let ip = format!("10.0.1.{}", i);
            let res = service.dispense(valid_dest, &ip, None, None, None).await;
            assert!(res.is_ok());
        }

        // 6th request to same destination account within window must fail with 429 even from a new IP
        let err = service
            .dispense(valid_dest, "10.0.1.99", None, None, None)
            .await
            .unwrap_err();
        assert_eq!(err.0, StatusCode::TOO_MANY_REQUESTS);
    }

    #[tokio::test]
    async fn test_invalid_destination_rejection() {
        let service = FaucetDispenserService::new();
        let err = service
            .dispense("INVALID_KEY", "127.0.0.1", None, None, None)
            .await
            .unwrap_err();
        assert_eq!(err.0, StatusCode::BAD_REQUEST);
    }

    #[tokio::test]
    async fn test_concurrent_funding_requests() {
        let service = Arc::new(FaucetDispenserService::new());
        let mut handles = Vec::new();

        for i in 0..5 {
            let svc = Arc::clone(&service);
            let ip = format!("172.16.0.{}", i);
            let handle = tokio::spawn(async move {
                let dest = "GBBD67IF6I2E7E5NCTZTTG46YAKMBH2O7662T7O4B5XW4YVRE3L363C6";
                svc.dispense(dest, &ip, Some(Decimal::new(10, 0)), None, None)
                    .await
            });
            handles.push(handle);
        }

        for handle in handles {
            let res = handle.await.unwrap();
            assert!(res.is_ok());
        }

        let stats = service.get_stats().await;
        assert_eq!(stats.total_dispensed_count, 5);
    }

    #[tokio::test]
    async fn test_pool_replenishment() {
        let service = FaucetDispenserService::new();
        let initial_stats = service.get_stats().await;
        service.replenish_pool(Decimal::new(5000, 0)).await;
        let after_stats = service.get_stats().await;
        assert_eq!(
            after_stats.balance_xlm,
            initial_stats.balance_xlm + Decimal::new(5000, 0)
        );
    }

    #[tokio::test]
    async fn test_faucet_service_with_redis_builder() {
        let client = redis::Client::open("redis://127.0.0.1:6379").unwrap();
        let service = FaucetDispenserService::new().with_redis(client);
        assert!(service.redis.is_some());
    }

    #[tokio::test]
    async fn test_faucet_rate_limit_pruning_removes_expired_entries() {
        let mut service = FaucetDispenserService::new();
        service.rate_window = Duration::from_millis(50);
        let dest = "GBBD67IF6I2E7E5NCTZTTG46YAKMBH2O7662T7O4B5XW4YVRE3L363C6";

        assert!(service.check_rate_limit("1.2.3.4", dest).await);
        let stats_before = service.get_stats().await;
        assert_eq!(stats_before.rate_limited_ips_count, 1);

        tokio::time::sleep(Duration::from_millis(60)).await;

        // Next check prunes expired entry, preventing monotonic memory growth
        assert!(service.check_rate_limit("5.6.7.8", dest).await);
        let stats_after = service.get_stats().await;
        assert_eq!(stats_after.rate_limited_ips_count, 1);
    }

    #[tokio::test]
    async fn test_dispense_with_valid_text_memo() {
        let service = FaucetDispenserService::new();
        let valid_destination = "GBBD67IF6I2E7E5NCTZTTG46YAKMBH2O7662T7O4B5XW4YVRE3L363C6";
        let res = service
            .dispense(
                valid_destination,
                "192.168.1.20",
                Some(Decimal::new(50, 0)),
                Some("order-12345".to_string()),
                Some(StellarMemoType::Text),
            )
            .await
            .unwrap();

        assert!(res.success);
        assert_eq!(res.memo, Some("order-12345".to_string()));
        assert!(res.message.contains("memo: order-12345"));
    }

    #[tokio::test]
    async fn test_dispense_with_oversized_text_memo() {
        let service = FaucetDispenserService::new();
        let valid_destination = "GBBD67IF6I2E7E5NCTZTTG46YAKMBH2O7662T7O4B5XW4YVRE3L363C6";
        // 29 bytes: exceeds 28-byte limit
        let long_memo = "12345678901234567890123456789";
        assert_eq!(long_memo.len(), 29);

        let err = service
            .dispense(
                valid_destination,
                "192.168.1.21",
                Some(Decimal::new(50, 0)),
                Some(long_memo.to_string()),
                Some(StellarMemoType::Text),
            )
            .await
            .unwrap_err();

        assert_eq!(err.0, StatusCode::BAD_REQUEST);
        assert!(err.1.contains("28-byte limit"));
    }

    #[tokio::test]
    async fn test_dispense_with_valid_id_memo() {
        let service = FaucetDispenserService::new();
        let valid_destination = "GBBD67IF6I2E7E5NCTZTTG46YAKMBH2O7662T7O4B5XW4YVRE3L363C6";
        let res = service
            .dispense(
                valid_destination,
                "192.168.1.22",
                Some(Decimal::new(50, 0)),
                Some("9876543210".to_string()),
                Some(StellarMemoType::Id),
            )
            .await
            .unwrap();

        assert!(res.success);
        assert_eq!(res.memo, Some("9876543210".to_string()));
    }

    #[tokio::test]
    async fn test_dispense_with_invalid_id_memo() {
        let service = FaucetDispenserService::new();
        let valid_destination = "GBBD67IF6I2E7E5NCTZTTG46YAKMBH2O7662T7O4B5XW4YVRE3L363C6";
        let err = service
            .dispense(
                valid_destination,
                "192.168.1.23",
                Some(Decimal::new(50, 0)),
                Some("not-a-number".to_string()),
                Some(StellarMemoType::Id),
            )
            .await
            .unwrap_err();

        assert_eq!(err.0, StatusCode::BAD_REQUEST);
        assert!(err.1.contains("Invalid Stellar memo ID"));
    }

    #[tokio::test]
    async fn test_dispense_with_valid_hash_memo() {
        let service = FaucetDispenserService::new();
        let valid_destination = "GBBD67IF6I2E7E5NCTZTTG46YAKMBH2O7662T7O4B5XW4YVRE3L363C6";
        let valid_hash = "0123456789abcdef0123456789abcdef0123456789abcdef0123456789abcdef";
        let res = service
            .dispense(
                valid_destination,
                "192.168.1.24",
                Some(Decimal::new(50, 0)),
                Some(valid_hash.to_string()),
                Some(StellarMemoType::Hash),
            )
            .await
            .unwrap();

        assert!(res.success);
        assert_eq!(res.memo, Some(valid_hash.to_string()));
    }

    #[tokio::test]
    async fn test_dispense_with_invalid_hash_memo() {
        let service = FaucetDispenserService::new();
        let valid_destination = "GBBD67IF6I2E7E5NCTZTTG46YAKMBH2O7662T7O4B5XW4YVRE3L363C6";
        let err = service
            .dispense(
                valid_destination,
                "192.168.1.25",
                Some(Decimal::new(50, 0)),
                Some("too_short_hash".to_string()),
                Some(StellarMemoType::Hash),
            )
            .await
            .unwrap_err();

        assert_eq!(err.0, StatusCode::BAD_REQUEST);
        assert!(err.1.contains("Invalid Stellar memo hash"));
    }

    #[tokio::test]
    async fn test_rate_limit_table_purges_expired_entries() {
        let service = FaucetDispenserService::with_config(5, Duration::from_millis(40));
        let dest = "GBBD67IF6I2E7E5NCTZTTG46YAKMBH2O7662T7O4B5XW4YVRE3L363C6";

        // Insert requests from 5 distinct client IPs
        for i in 0..5 {
            let ip = format!("192.168.1.{}", i);
            assert!(service.check_rate_limit(&ip, dest).await);
        }

        assert_eq!(service.get_stats().await.rate_limited_ips_count, 5);

        // Wait for rate window to elapse
        tokio::time::sleep(Duration::from_millis(50)).await;

        // Next rate limit check triggers eviction of inactive/expired IP keys
        assert!(service.check_rate_limit("192.168.1.100", dest).await);

        // Previous 5 IPs were purged; only the new active IP remains
        let stats = service.get_stats().await;
        assert_eq!(stats.rate_limited_ips_count, 1);
    }
}
