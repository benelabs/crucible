use redis::{AsyncCommands, Client};
use serde::{Deserialize, Serialize};
use sqlx::PgPool;
use tokio::time::{sleep, Duration};
use tracing::{error, info};

/// Default batch size for cursor-based keyset pagination (100 contracts).
pub const DEFAULT_CONTRACT_CACHE_BATCH_SIZE: i64 = 100;
/// Default TTL in seconds for cached contract metadata (1 hour).
pub const DEFAULT_CONTRACT_CACHE_TTL_SECS: u64 = 3600;

/// Cached representation of a smart contract record loaded via keyset pagination
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct CachedContract {
    pub id: i32,
    pub contract_id: String,
    pub name: Option<String>,
    pub description: Option<String>,
}

/// Cache warming worker that pre-loads frequently accessed data into Redis
#[derive(Debug, Clone)]
pub struct CacheWarmWorker {
    pub db_pool: PgPool,
    pub redis_client: Client,
    pub warm_interval: Duration,
    pub contract_batch_size: i64,
}

impl CacheWarmWorker {
    /// Create a new cache warming worker
    pub fn new(db_pool: PgPool, redis_client: Client, warm_interval: Duration) -> Self {
        Self {
            db_pool,
            redis_client,
            warm_interval,
            contract_batch_size: DEFAULT_CONTRACT_CACHE_BATCH_SIZE,
        }
    }

    /// Set the batch size for keyset paginated contract pre-warming
    pub fn with_contract_batch_size(mut self, batch_size: i64) -> Self {
        self.contract_batch_size = if batch_size <= 0 {
            DEFAULT_CONTRACT_CACHE_BATCH_SIZE
        } else {
            batch_size
        };
        self
    }

    /// Return the configured contract pre-warming batch size
    pub fn contract_batch_size(&self) -> i64 {
        self.contract_batch_size
    }

    /// Start the cache warming process
    pub async fn start(&self) -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
        info!("Starting cache warming worker...");

        loop {
            if let Err(e) = self.warm_cache().await {
                error!("Failed to warm cache: {}", e);
            }

            // Wait for the configured interval before next warm cycle
            sleep(self.warm_interval).await;
        }
    }

    /// Warm the cache with frequently accessed data
    pub async fn warm_cache(&self) -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
        info!("Warming cache...");

        // Get Redis connection
        let mut redis_conn = self.redis_client.get_multiplexed_async_connection().await?;

        // Example: Warm dashboard metrics cache
        // In a real implementation, this would query database and populate Redis
        let dashboard_metrics = self.load_dashboard_metrics().await?;

        // Store in Redis with TTL
        redis_conn
            .set_ex::<_, _, ()>(
                "dashboard:metrics:latest",
                serde_json::to_string(&dashboard_metrics)?,
                300, // 5 minutes TTL
            )
            .await?;

        // Example: Warm popular build metrics
        let build_metrics = self.load_popular_builds().await?;
        redis_conn
            .set_ex::<_, _, ()>(
                "builds:popular:latest",
                serde_json::to_string(&build_metrics)?,
                600, // 10 minutes TTL
            )
            .await?;

        // Warm contracts cache using keyset pagination to avoid query timeouts and memory spikes
        if let Err(e) = self.warm_contracts_cache(self.contract_batch_size).await {
            error!("Failed to warm contracts cache: {}", e);
        }

        info!("Cache warming completed successfully");
        Ok(())
    }

    /// Pre-warms contract metadata in Redis using cursor-based keyset pagination.
    /// Iterates across all pages until no further contracts remain.
    /// Returns the total number of warmed contracts.
    pub async fn warm_contracts_cache(
        &self,
        batch_size: i64,
    ) -> Result<usize, Box<dyn std::error::Error + Send + Sync>> {
        let mut redis_conn = self.redis_client.get_multiplexed_async_connection().await?;
        let batch_size = if batch_size <= 0 {
            DEFAULT_CONTRACT_CACHE_BATCH_SIZE
        } else {
            batch_size
        };
        let mut last_id: i32 = 0;
        let mut total_warmed: usize = 0;

        loop {
            let contracts = self.load_contracts_keyset(last_id, batch_size).await?;
            if contracts.is_empty() {
                break;
            }

            for contract in &contracts {
                let cache_key = format!("contract:metadata:{}", contract.contract_id);
                let json = serde_json::to_string(contract)?;
                redis_conn
                    .set_ex::<_, _, ()>(cache_key, json, DEFAULT_CONTRACT_CACHE_TTL_SECS)
                    .await?;
            }

            total_warmed += contracts.len();
            if let Some(last) = contracts.last() {
                last_id = last.id;
            }

            if (contracts.len() as i64) < batch_size {
                break;
            }

            tokio::task::yield_now().await;
        }

        info!(
            "Pre-warmed {} contracts using keyset pagination",
            total_warmed
        );
        Ok(total_warmed)
    }

    /// Fetches a page of contracts using cursor-based keyset pagination (id > cursor).
    pub async fn load_contracts_keyset(
        &self,
        cursor_id: i32,
        limit: i64,
    ) -> Result<Vec<CachedContract>, sqlx::Error> {
        let rows = sqlx::query(
            r#"
            SELECT id, contract_id, name, description
            FROM contracts
            WHERE id > $1
            ORDER BY id ASC
            LIMIT $2
            "#,
        )
        .bind(cursor_id)
        .bind(limit)
        .fetch_all(&self.db_pool)
        .await?;

        let mut contracts = Vec::with_capacity(rows.len());
        for row in rows {
            contracts.push(CachedContract {
                id: sqlx::Row::try_get(&row, "id")?,
                contract_id: sqlx::Row::try_get(&row, "contract_id")?,
                name: sqlx::Row::try_get(&row, "name")?,
                description: sqlx::Row::try_get(&row, "description")?,
            });
        }
        Ok(contracts)
    }

    /// Load dashboard metrics from database
    async fn load_dashboard_metrics(
        &self,
    ) -> Result<DashboardMetrics, Box<dyn std::error::Error + Send + Sync>> {
        // This would be implemented with actual SQL queries
        // For now, returning a placeholder
        Ok(DashboardMetrics {
            total_builds: 12345,
            successful_builds: 11234,
            failed_builds: 1111,
            avg_build_time_ms: 4567,
        })
    }

    /// Load popular builds from database
    async fn load_popular_builds(
        &self,
    ) -> Result<Vec<BuildSummary>, Box<dyn std::error::Error + Send + Sync>> {
        // This would be implemented with actual SQL queries
        // For now, returning a placeholder
        Ok(vec![
            BuildSummary {
                id: "build-123".to_string(),
                name: "main-branch".to_string(),
                status: "success".to_string(),
                duration_ms: 3456,
            },
            BuildSummary {
                id: "build-456".to_string(),
                name: "develop-branch".to_string(),
                status: "success".to_string(),
                duration_ms: 2891,
            },
        ])
    }
}

/// Dashboard metrics structure
#[derive(serde::Serialize, serde::Deserialize, Debug)]
pub struct DashboardMetrics {
    pub total_builds: u64,
    pub successful_builds: u64,
    pub failed_builds: u64,
    pub avg_build_time_ms: u64,
}

/// Build summary structure
#[derive(serde::Serialize, serde::Deserialize, Debug)]
pub struct BuildSummary {
    pub id: String,
    pub name: String,
    pub status: String,
    pub duration_ms: u64,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_batch_constants() {
        assert_eq!(DEFAULT_CONTRACT_CACHE_BATCH_SIZE, 100);
        assert_eq!(DEFAULT_CONTRACT_CACHE_TTL_SECS, 3600);
    }

    #[test]
    fn test_cached_contract_serialization() {
        let contract = CachedContract {
            id: 42,
            contract_id: "CA_TEST_12345".to_string(),
            name: Some("VaultToken".to_string()),
            description: Some("Staking token contract".to_string()),
        };

        let json = serde_json::to_string(&contract).unwrap();
        let deserialized: CachedContract = serde_json::from_str(&json).unwrap();
        assert_eq!(contract, deserialized);
    }

    #[tokio::test]
    async fn test_worker_batch_size_configuration() {
        let pool = sqlx::PgPool::connect_lazy("postgres://localhost/test").unwrap();
        let redis = redis::Client::open("redis://127.0.0.1/").unwrap();
        let worker = CacheWarmWorker::new(pool, redis, Duration::from_secs(60));
        assert_eq!(worker.contract_batch_size(), 100);

        let customized = worker.with_contract_batch_size(250);
        assert_eq!(customized.contract_batch_size(), 250);

        let fallback = customized.with_contract_batch_size(-5);
        assert_eq!(fallback.contract_batch_size(), 100);
    }
}
