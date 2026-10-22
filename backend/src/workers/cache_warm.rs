use redis::{AsyncCommands, Client};
use sqlx::PgPool;
use tokio::time::{sleep, Duration};
use tracing::{error, info};

/// Cache warming worker that pre-loads frequently accessed data into Redis
#[derive(Debug, Clone)]
pub struct CacheWarmWorker {
    db_pool: PgPool,
    redis_client: Client,
    warm_interval: Duration,
}

impl CacheWarmWorker {
    /// Create a new cache warming worker
    pub fn new(db_pool: PgPool, redis_client: Client, warm_interval: Duration) -> Self {
        Self {
            db_pool,
            redis_client,
            warm_interval,
        }
    }

    /// Start the cache warming process
    pub async fn start(&self) -> Result<(), Box<dyn std::error::Error>> {
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
    async fn warm_cache(&self) -> Result<(), Box<dyn std::error::Error>> {
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

        // Warm popular build metrics
        let build_metrics = self.load_popular_builds().await?;
        redis_conn
            .set_ex::<_, _, ()>(
                "builds:popular:latest",
                serde_json::to_string(&build_metrics)?,
                600, // 10 minutes TTL
            )
            .await?;

        // Warm contracts cache using cursor-based keyset pagination
        if let Err(e) = self.warm_contracts_cache(DEFAULT_CACHE_WARM_BATCH_SIZE).await {
            error!("Failed to warm contracts cache: {}", e);
        }

        info!("Cache warming completed successfully");
        Ok(())
    }

    /// Warm the contracts cache using cursor-based keyset pagination (WHERE id > $last_id ORDER BY id ASC LIMIT $batch_size)
    pub async fn warm_contracts_cache(&self, batch_size: i64) -> Result<usize, Box<dyn std::error::Error>> {
        let mut last_id: i32 = 0;
        let mut total_warmed: usize = 0;
        let mut redis_conn = self.redis_client.get_multiplexed_async_connection().await?;

        loop {
            let contracts: Vec<WarmContractRecord> = sqlx::query_as::<_, WarmContractRecord>(
                "SELECT id, contract_id, name, description FROM contracts WHERE id > $1 ORDER BY id ASC LIMIT $2",
            )
            .bind(last_id)
            .bind(batch_size)
            .fetch_all(&self.db_pool)
            .await?;

            if contracts.is_empty() {
                break;
            }

            for contract in &contracts {
                let cache_key = format!("contract:{}", contract.contract_id);
                let payload = serde_json::to_string(contract)?;
                redis_conn
                    .set_ex::<_, _, ()>(&cache_key, payload, CONTRACT_CACHE_TTL_SECS)
                    .await?;
                total_warmed += 1;
            }

            if let Some(last) = contracts.last() {
                last_id = last.id;
            }

            if contracts.len() < batch_size as usize {
                break;
            }
        }

        info!("Warmed {} contracts in cache using keyset pagination", total_warmed);
        Ok(total_warmed)
    }

    /// Load dashboard metrics from database
    async fn load_dashboard_metrics(&self) -> Result<DashboardMetrics, Box<dyn std::error::Error>> {
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
    async fn load_popular_builds(&self) -> Result<Vec<BuildSummary>, Box<dyn std::error::Error>> {
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

/// Default batch size for cursor-based keyset pagination
pub const DEFAULT_CACHE_WARM_BATCH_SIZE: i64 = 500;

/// Default Redis TTL for cached contract metadata (1 hour)
pub const CONTRACT_CACHE_TTL_SECS: u64 = 3600;

/// Contract model loaded during cache pre-warming
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize, sqlx::FromRow)]
pub struct WarmContractRecord {
    pub id: i32,
    pub contract_id: String,
    pub name: Option<String>,
    pub description: Option<String>,
}

/// Helper function to paginate an in-memory dataset using cursor keyset semantics (for testing & verification)
pub fn paginate_contracts_keyset(
    items: &[WarmContractRecord],
    last_id: i32,
    limit: usize,
) -> Vec<WarmContractRecord> {
    items
        .iter()
        .filter(|c| c.id > last_id)
        .take(limit)
        .cloned()
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_warm_contract_record_serde() {
        let record = WarmContractRecord {
            id: 42,
            contract_id: "contract-abc-123".to_string(),
            name: Some("Vault".to_string()),
            description: Some("Stellar smart contract".to_string()),
        };

        let serialized = serde_json::to_string(&record).expect("serialization succeeds");
        let deserialized: WarmContractRecord =
            serde_json::from_str(&serialized).expect("deserialization succeeds");
        assert_eq!(record, deserialized);
    }

    #[test]
    fn test_cursor_keyset_pagination() {
        // Create 25 dummy contract records with sequential IDs
        let mock_db: Vec<WarmContractRecord> = (1..=25)
            .map(|i| WarmContractRecord {
                id: i,
                contract_id: format!("contract-{:03}", i),
                name: Some(format!("Contract {}", i)),
                description: None,
            })
            .collect();

        let batch_size = 10;
        let mut last_id = 0;
        let mut collected = Vec::new();

        loop {
            let page = paginate_contracts_keyset(&mock_db, last_id, batch_size);
            if page.is_empty() {
                break;
            }
            if let Some(last) = page.last() {
                last_id = last.id;
            }
            let count = page.len();
            collected.extend(page);
            if count < batch_size {
                break;
            }
        }

        assert_eq!(collected.len(), 25);
        assert_eq!(collected[0].id, 1);
        assert_eq!(collected[24].id, 25);
    }
}
