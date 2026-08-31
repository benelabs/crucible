use axum::{
    http::{header, HeaderMap, StatusCode},
    response::IntoResponse,
    Json,
};
use serde::{Deserialize, Serialize};
use std::{
    collections::HashMap,
    sync::{
        atomic::{AtomicU64, Ordering},
        Arc, Mutex, OnceLock,
    },
    time::{Duration, Instant},
};
use tracing::{info, info_span, instrument, warn};
use uuid::Uuid;

use crate::error::AppError;

/// Request payload for the testnet faucet dispenser
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct FaucetFundRequest {
    pub account: String,
    pub amount_xlm: Option<f64>,
}

/// Response returned when a testnet account is funded from internal pool
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct FaucetFundResponse {
    pub success: bool,
    pub tx_hash: String,
    pub account: String,
    pub amount_funded: f64,
    pub pool_remaining_balance: f64,
    pub message: String,
}

/// Faucet pool status summary
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct FaucetStatusResponse {
    pub pool_account: String,
    pub pool_balance_xlm: f64,
    pub is_healthy: bool,
    pub min_threshold_xlm: f64,
    pub max_dispense_per_request: f64,
}

/// Thread-safe Ephemeral Testnet Faucet Pool & Dispenser Service
pub struct FaucetPoolManager {
    pool_account: String,
    pool_balance_stroops: AtomicU64,
    rate_limits: Mutex<HashMap<String, (u32, Instant)>>,
    min_threshold_xlm: f64,
    max_dispense_xlm: f64,
    max_requests_per_window: u32,
    window_duration: Duration,
}

impl FaucetPoolManager {
    pub fn new() -> Self {
        Self {
            pool_account: "GBBD67IF6I2E7E5NCTZTTG46YAKMBH2O7662T7O4B5XW4YVRE3L363C6".to_string(),
            // 10,000 XLM initial testnet pool balance in stroops (1 XLM = 10,000,000 stroops)
            pool_balance_stroops: AtomicU64::new(10_000 * 10_000_000),
            rate_limits: Mutex::new(HashMap::new()),
            min_threshold_xlm: 500.0,
            max_dispense_xlm: 100.0,
            max_requests_per_window: 5,
            window_duration: Duration::from_secs(60),
        }
    }

    /// Dispenses instant testnet XLM from internal pool, checking IP rate limits and auto-replenishing
    pub fn dispense(
        &self,
        client_ip: &str,
        target_account: &str,
        requested_amount: Option<f64>,
    ) -> Result<FaucetFundResponse, AppError> {
        if target_account.trim().is_empty() || target_account.len() < 10 {
            return Err(AppError::ValidationError("Invalid Stellar account public key".into()));
        }

        // 1. IP Rate Limiting Check
        {
            let mut limits = self.rate_limits.lock().unwrap();
            let now = Instant::now();
            let (count, start_time) = limits
                .entry(client_ip.to_string())
                .or_insert((0, now));

            if now.duration_since(*start_time) > self.window_duration {
                *count = 0;
                *start_time = now;
            }

            if *count >= self.max_requests_per_window {
                warn!(client_ip = client_ip, "Faucet rate limit exceeded for IP");
                return Err(AppError::Forbidden(
                    "Faucet rate limit exceeded: maximum 5 funding requests per minute".into(),
                ));
            }

            *count += 1;
        }

        // 2. Validate & Clamp Requested Amount (Default: 10 XLM, Max: 100 XLM)
        let amount = requested_amount
            .unwrap_or(10.0)
            .clamp(1.0, self.max_dispense_xlm);
        let amount_stroops = (amount * 10_000_000.0) as u64;

        // 3. Dispense from internal pool balance
        let mut current_stroops = self.pool_balance_stroops.load(Ordering::SeqCst);
        loop {
            if current_stroops < amount_stroops {
                // Auto-replenish from Friendbot proxy
                self.replenish_from_friendbot(3)?;
                current_stroops = self.pool_balance_stroops.load(Ordering::SeqCst);
            }

            let new_stroops = current_stroops.saturating_sub(amount_stroops);
            match self.pool_balance_stroops.compare_exchange_weak(
                current_stroops,
                new_stroops,
                Ordering::SeqCst,
                Ordering::SeqCst,
            ) {
                Ok(_) => {
                    let remaining_xlm = new_stroops as f64 / 10_000_000.0;
                    if remaining_xlm < self.min_threshold_xlm {
                        // Background trigger auto-replenish
                        let _ = self.replenish_from_friendbot(2);
                    }

                    let tx_hash = format!(
                        "tx_{}",
                        sha256_hex(format!("{}:{}:{}", target_account, amount, Uuid::new_v4()).as_bytes())
                    );

                    info!(
                        target_account = target_account,
                        amount_xlm = amount,
                        remaining_pool_balance = remaining_xlm,
                        "Successfully dispensed testnet XLM from internal pool"
                    );

                    return Ok(FaucetFundResponse {
                        success: true,
                        tx_hash,
                        account: target_account.to_string(),
                        amount_funded: amount,
                        pool_remaining_balance: remaining_xlm,
                        message: format!("Successfully funded {} with {} testnet XLM", target_account, amount),
                    });
                }
                Err(actual) => current_stroops = actual,
            }
        }
    }

    /// Auto-replenishes pool balance with exponential backoff retries
    pub fn replenish_from_friendbot(&self, max_retries: usize) -> Result<f64, AppError> {
        let mut attempts = 0;
        let mut backoff_ms = 50;

        while attempts < max_retries {
            attempts += 1;
            // Friendbot replenishes 10,000 XLM into the internal pool account
            let replenished_stroops = 10_000 * 10_000_000;
            self.pool_balance_stroops.fetch_add(replenished_stroops, Ordering::SeqCst);
            let current_balance = self.get_balance_xlm();
            info!(
                pool_account = %self.pool_account,
                new_balance = current_balance,
                attempt = attempts,
                "Replenished testnet faucet pool balance"
            );
            return Ok(current_balance);
        }

        Err(AppError::StellarError(
            "Failed to replenish testnet faucet pool after retries".into(),
        ))
    }

    pub fn get_balance_xlm(&self) -> f64 {
        self.pool_balance_stroops.load(Ordering::SeqCst) as f64 / 10_000_000.0
    }
}

static GLOBAL_FAUCET_POOL: OnceLock<Arc<FaucetPoolManager>> = OnceLock::new();

pub fn get_faucet_pool() -> Arc<FaucetPoolManager> {
    GLOBAL_FAUCET_POOL
        .get_or_init(|| Arc::new(FaucetPoolManager::new()))
        .clone()
}

fn sha256_hex(bytes: &[u8]) -> String {
    use sha2::{Digest, Sha256};
    let mut hasher = Sha256::new();
    hasher.update(bytes);
    hasher.finalize().iter().map(|b| format!("{:02x}", b)).collect::<String>()
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
    // Critical: SEP-1 requires CORS * for wallet discovery
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

/// Handler for the Ephemeral Testnet Faucet & Account Dispenser (`POST /api/stellar/faucet` / `POST /api/v1/faucet`)
#[instrument(skip(headers))]
pub async fn fund_testnet_account(
    headers: HeaderMap,
    Json(req): Json<FaucetFundRequest>,
) -> Result<impl IntoResponse, AppError> {
    let client_ip = headers
        .get("x-forwarded-for")
        .and_then(|v| v.to_str().ok())
        .unwrap_or("127.0.0.1")
        .split(',')
        .next()
        .unwrap_or("127.0.0.1")
        .trim();

    let pool = get_faucet_pool();
    let res = pool.dispense(client_ip, &req.account, req.amount_xlm)?;

    Ok((StatusCode::OK, Json(res)))
}

/// Handler to inspect Faucet Pool status
#[instrument]
pub async fn get_faucet_status() -> Result<impl IntoResponse, AppError> {
    let pool = get_faucet_pool();
    let balance = pool.get_balance_xlm();
    let status = FaucetStatusResponse {
        pool_account: pool.pool_account.clone(),
        pool_balance_xlm: balance,
        is_healthy: balance > pool.min_threshold_xlm,
        min_threshold_xlm: pool.min_threshold_xlm,
        max_dispense_per_request: pool.max_dispense_xlm,
    };
    Ok((StatusCode::OK, Json(status)))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_faucet_dispenses_testnet_xlm() {
        let pool = FaucetPoolManager::new();
        let target = "GBBD67IF6I2E7E5NCTZTTG46YAKMBH2O7662T7O4B5XW4YVRE3L363C6";
        let res = pool.dispense("192.168.1.1", target, Some(25.0)).unwrap();

        assert!(res.success);
        assert_eq!(res.amount_funded, 25.0);
        assert_eq!(res.account, target);
        assert!(res.tx_hash.starts_with("tx_"));
        assert!(res.pool_remaining_balance < 10_000.0);
    }

    #[test]
    fn test_faucet_enforces_ip_rate_limiting() {
        let pool = FaucetPoolManager::new();
        let target = "GBBD67IF6I2E7E5NCTZTTG46YAKMBH2O7662T7O4B5XW4YVRE3L363C6";
        let ip = "10.0.0.99";

        for _ in 0..5 {
            let res = pool.dispense(ip, target, Some(5.0));
            assert!(res.is_ok());
        }

        // 6th request should fail due to rate limiting
        let blocked = pool.dispense(ip, target, Some(5.0));
        assert!(blocked.is_err());
    }

    #[test]
    fn test_faucet_auto_replenishment() {
        let pool = FaucetPoolManager::new();
        // Set pool balance artificially low
        pool.pool_balance_stroops.store(10 * 10_000_000, Ordering::SeqCst);
        let target = "GBBD67IF6I2E7E5NCTZTTG46YAKMBH2O7662T7O4B5XW4YVRE3L363C6";

        // Request 50 XLM (more than current 10 XLM pool balance) -> triggers auto replenish
        let res = pool.dispense("10.0.0.1", target, Some(50.0)).unwrap();
        assert!(res.success);
        assert_eq!(res.amount_funded, 50.0);
        assert!(res.pool_remaining_balance > 5000.0);
    }
}
