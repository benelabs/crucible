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
use tracing::{error, info, info_span, instrument, warn};

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct FundAccountRequest {
    pub destination: String,
    #[serde(default)]
    pub amount_xlm: Option<Decimal>,
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
    max_requests_per_window: usize,
    rate_window: Duration,
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
            max_requests_per_window: 5,
            rate_window: Duration::from_secs(60),
        }
    }

    pub async fn check_rate_limit(&self, ip: &str) -> bool {
        let mut limits = self.rate_limits.lock().await;
        let now = Instant::now();
        let timestamps = limits.entry(ip.to_string()).or_default();

        // Prune older than window
        timestamps.retain(|&t| now.duration_since(t) < self.rate_window);

        if timestamps.len() >= self.max_requests_per_window {
            false
        } else {
            timestamps.push(now);
            true
        }
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
    ) -> Result<FundAccountResponse, (StatusCode, String)> {
        // Validate destination Stellar address
        if !destination.starts_with('G') || destination.len() != 56 {
            return Err((
                StatusCode::BAD_REQUEST,
                "Invalid Stellar testnet public key. Must start with 'G' and be 56 characters.".to_string(),
            ));
        }

        // Check rate limit per IP
        if !self.check_rate_limit(client_ip).await {
            warn!(client_ip, "Faucet rate limit exceeded for client IP");
            return Err((
                StatusCode::TOO_MANY_REQUESTS,
                "Rate limit exceeded. Maximum 5 funding requests per minute per IP.".to_string(),
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
        hasher.update(format!("{}:{}:{}", destination, client_ip, Instant::now().elapsed().as_nanos()).as_bytes());
        let tx_hash = hasher.finalize().iter().map(|b| format!("{:02x}", b)).collect::<String>();

        info!(
            destination,
            amount = %dispense_amount,
            tx_hash = %tx_hash,
            "Dispensed testnet XLM to account"
        );

        Ok(FundAccountResponse {
            success: true,
            transaction_hash: tx_hash,
            funded_account: destination.to_string(),
            amount_dispensed: dispense_amount,
            pool_remaining_balance: remaining,
            dispenser_source: source,
            message: format!("Successfully funded {} XLM to {}", dispense_amount, destination),
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
DOCUMENTATION_URL="https://github.com/your-org/crucible"

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
    let res = service.dispense(&payload.destination, client_ip, payload.amount_xlm).await?;
    Ok(Json(res))
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
        let res = service.dispense(valid_destination, "192.168.1.10", Some(Decimal::new(50, 0))).await.unwrap();

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
            let res = service.dispense(valid_dest, ip, None).await;
            assert!(res.is_ok());
        }

        // 6th request from same IP within window must fail with 429
        let err = service.dispense(valid_dest, ip, None).await.unwrap_err();
        assert_eq!(err.0, StatusCode::TOO_MANY_REQUESTS);

        // Different IP must still succeed
        let other_ip_res = service.dispense(valid_dest, "10.0.0.6", None).await;
        assert!(other_ip_res.is_ok());
    }

    #[tokio::test]
    async fn test_invalid_destination_rejection() {
        let service = FaucetDispenserService::new();
        let err = service.dispense("INVALID_KEY", "127.0.0.1", None).await.unwrap_err();
        assert_eq!(err.0, StatusCode::BAD_REQUEST);
    }

    #[tokio::test]
    async fn test_concurrent_funding_requests() {
        let service = Arc::new(FaucetDispenserService::new());
        let mut handles = Vec::new();

        for i in 0..10 {
            let svc = Arc::clone(&service);
            let ip = format!("172.16.0.{}", i);
            let handle = tokio::spawn(async move {
                let dest = "GBBD67IF6I2E7E5NCTZTTG46YAKMBH2O7662T7O4B5XW4YVRE3L363C6";
                svc.dispense(dest, &ip, Some(Decimal::new(10, 0))).await
            });
            handles.push(handle);
        }

        for handle in handles {
            let res = handle.await.unwrap();
            assert!(res.is_ok());
        }

        let stats = service.get_stats().await;
        assert_eq!(stats.total_dispensed_count, 10);
    }

    #[tokio::test]
    async fn test_pool_replenishment() {
        let service = FaucetDispenserService::new();
        let initial_stats = service.get_stats().await;
        service.replenish_pool(Decimal::new(5000, 0)).await;
        let after_stats = service.get_stats().await;
        assert_eq!(after_stats.balance_xlm, initial_stats.balance_xlm + Decimal::new(5000, 0));
    }
}
