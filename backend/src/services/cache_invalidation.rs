//! Redis Cache Invalidation System
//!
//! This module implements a robust cache invalidation system that works with Redis
//! to ensure data consistency between the database and cache layers.

use std::collections::HashMap;
use serde::{Serialize, Deserialize};
use std::sync::Arc;
use redis::AsyncCommands;
use tokio::sync::mpsc;
use tracing::{debug, error, info};

/// Cache invalidation strategy
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum InvalidationStrategy {
    /// Invalidate specific keys
    Key,
    /// Invalidate all keys matching a pattern
    Pattern,
    /// Invalidate all keys in a namespace
    Namespace,
    /// Invalidate based on dependencies (cache tags)
    Tag,
}

/// Cache invalidation event
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CacheInvalidationEvent {
    /// Strategy to use for invalidation
    pub strategy: InvalidationStrategy,
    /// Keys or patterns to invalidate
    pub targets: Vec<String>,
    /// Optional namespace or tag information
    pub metadata: Option<HashMap<String, String>>,
    /// Timestamp when event was created
    #[serde(skip, default = "std::time::Instant::now")]
    pub timestamp: std::time::Instant,
    /// Cache generation sequence number
    #[serde(default)]
    pub sequence: u64,
}

/// Tracks cache generation sequences in Redis to detect stale local cache entries
/// even when Pub/Sub broadcast messages are dropped during network partitions.
#[derive(Debug, Clone)]
pub struct CacheGenerationTracker {
    redis_client: Arc<redis::Client>,
    local_generations: Arc<tokio::sync::RwLock<HashMap<String, u64>>>,
}

impl CacheGenerationTracker {
    pub fn new(redis_client: Arc<redis::Client>) -> Self {
        Self {
            redis_client,
            local_generations: Arc::new(tokio::sync::RwLock::new(HashMap::new())),
        }
    }

    /// Increment and return the new generation sequence for a cache namespace/key in Redis.
    pub async fn bump_generation(&self, key: &str) -> Result<u64, anyhow::Error> {
        let gen_key = format!("cache:generation:{}", key);
        let mut conn = self.redis_client.get_multiplexed_async_connection().await?;
        let next_gen: u64 = redis::cmd("INCR").arg(&gen_key).query_async(&mut conn).await?;
        self.local_generations.write().await.insert(key.to_string(), next_gen);
        Ok(next_gen)
    }

    /// Check if the local cache generation matches or exceeds the current generation in Redis.
    pub async fn is_valid(&self, key: &str, local_gen: u64) -> Result<bool, anyhow::Error> {
        let gen_key = format!("cache:generation:{}", key);
        let mut conn = self.redis_client.get_multiplexed_async_connection().await?;
        let current_gen: Option<u64> = redis::cmd("GET").arg(&gen_key).query_async(&mut conn).await?;
        match current_gen {
            Some(gen) => Ok(local_gen >= gen),
            None => Ok(true),
        }
    }

    /// Get current generation for a key in Redis.
    pub async fn get_generation(&self, key: &str) -> Result<u64, anyhow::Error> {
        let gen_key = format!("cache:generation:{}", key);
        let mut conn = self.redis_client.get_multiplexed_async_connection().await?;
        let gen: Option<u64> = redis::cmd("GET").arg(&gen_key).query_async(&mut conn).await?;
        Ok(gen.unwrap_or(0))
    }

    /// Get local generation cached in memory.
    pub async fn get_local_generation(&self, key: &str) -> Option<u64> {
        self.local_generations.read().await.get(key).copied()
    }

    /// Set local generation in memory.
    pub async fn set_local_generation(&self, key: &str, gen: u64) {
        self.local_generations.write().await.insert(key.to_string(), gen);
    }
}

/// Cache invalidation manager
#[derive(Debug, Clone)]
pub struct CacheInvalidationManager {
    /// Redis client for cache operations
    redis_client: Arc<redis::Client>,
    /// Channel for receiving invalidation events
    tx: mpsc::Sender<CacheInvalidationEvent>,
    /// Configuration for invalidation behavior
    config: CacheInvalidationConfig,
    /// Generation sequence tracker
    generation_tracker: CacheGenerationTracker,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CacheInvalidationConfig {
    /// Whether to enable background invalidation processing
    pub background_processing: bool,
    /// Maximum number of concurrent invalidation operations
    pub max_concurrent: usize,
    /// Timeout for individual invalidation operations
    pub timeout_ms: u64,
    /// Retry configuration
    pub retry_attempts: u8,
    /// Delay between retries in milliseconds
    pub retry_delay_ms: u64,
    /// Whether to use Redis Streams with consumer group acknowledgments for durable invalidation
    pub use_redis_streams: bool,
    /// Redis Stream name for invalidation events
    pub stream_name: String,
    /// Consumer group name for invalidation processing
    pub consumer_group: String,
    /// Stream retention limit (MAXLEN)
    pub stream_max_len: usize,
}

impl Default for CacheInvalidationConfig {
    fn default() -> Self {
        Self {
            background_processing: true,
            max_concurrent: 10,
            timeout_ms: 5000,
            retry_attempts: 3,
            retry_delay_ms: 100,
            use_redis_streams: true,
            stream_name: "cache:invalidation:stream".to_string(),
            consumer_group: "cache_invalidation_workers".to_string(),
            stream_max_len: 10_000,
        }
    }
}

impl CacheInvalidationManager {
    /// Create a new cache invalidation manager
    pub fn new(redis_client: Arc<redis::Client>, config: CacheInvalidationConfig) -> Self {
        let (tx, rx) = mpsc::channel::<CacheInvalidationEvent>(100);
        let generation_tracker = CacheGenerationTracker::new(redis_client.clone());
        
        // Start background processor if enabled
        if config.background_processing {
            let client = redis_client.clone();
            let config_clone = config.clone();
            
            tokio::spawn(async move {
                Self::background_processor(client, rx, config_clone).await;
            });
        }
        
        Self {
            redis_client,
            tx,
            config,
            generation_tracker,
        }
    }

    /// Access the generation sequence tracker
    pub fn generation_tracker(&self) -> &CacheGenerationTracker {
        &self.generation_tracker
    }

    /// Configuration for invalidation behavior
    pub fn config(&self) -> &CacheInvalidationConfig {
        &self.config
    }

    /// Publish an invalidation event to Redis Streams with consumer group durability
    pub async fn publish_stream_invalidation(
        &self,
        event: &CacheInvalidationEvent,
    ) -> Result<String, anyhow::Error> {
        let mut conn = self.redis_client.get_multiplexed_async_connection().await?;
        let payload = serde_json::to_string(event)?;
        let entry_id: String = redis::cmd("XADD")
            .arg(&self.config.stream_name)
            .arg("MAXLEN")
            .arg("~")
            .arg(self.config.stream_max_len)
            .arg("*")
            .arg("event")
            .arg(payload)
            .query_async(&mut conn)
            .await?;
        Ok(entry_id)
    }

    /// Acknowledge processing of a stream invalidation event
    pub async fn acknowledge_stream_event(&self, stream_id: &str) -> Result<(), anyhow::Error> {
        let mut conn = self.redis_client.get_multiplexed_async_connection().await?;
        let _: () = redis::cmd("XACK")
            .arg(&self.config.stream_name)
            .arg(&self.config.consumer_group)
            .arg(stream_id)
            .query_async(&mut conn)
            .await?;
        Ok(())
    }

    /// Bump generation sequence for a cache namespace or key
    pub async fn bump_generation(&self, key: &str) -> Result<u64, anyhow::Error> {
        self.generation_tracker.bump_generation(key).await
    }

    /// Check if local cache generation is valid against Redis
    pub async fn is_generation_valid(&self, key: &str, local_gen: u64) -> Result<bool, anyhow::Error> {
        self.generation_tracker.is_valid(key, local_gen).await
    }

    /// Enqueue an invalidation event
    pub async fn enqueue_invalidation(&self, event: CacheInvalidationEvent) -> Result<(), anyhow::Error> {
        self.tx.send(event).await.map_err(|e| {
            error!(error = ?e, "Failed to send invalidation event");
            anyhow::anyhow!("Failed to send invalidation event: {}", e)
        })
    }

    /// Invalidate specific cache keys synchronously
    pub async fn invalidate_keys(&self, keys: Vec<String>) -> Result<(), anyhow::Error> {
        let mut conn = self.redis_client.get_multiplexed_async_connection().await?;
        
        // Use pipelining for better performance
        let mut pipe = redis::pipe();
        for key in &keys {
            pipe.del(key);
        }
        
        let _: () = pipe.query_async(&mut conn).await?;
        
        info!(count = keys.len(), "Invalidated cache keys");
        Ok(())
    }

    /// Invalidate keys matching a pattern
    pub async fn invalidate_pattern(&self, pattern: &str) -> Result<(), anyhow::Error> {
        let mut conn = self.redis_client.get_multiplexed_async_connection().await?;
        
        // Get all keys matching the pattern
        let keys: Vec<String> = redis::cmd("KEYS")
            .arg(pattern)
            .query_async(&mut conn)
            .await?;
        
        if !keys.is_empty() {
            self.invalidate_keys(keys.clone()).await?;
            info!(pattern = %pattern, count = keys.len(), "Invalidated keys by pattern");
        }
        
        Ok(())
    }

    /// Invalidate all keys in a namespace
    pub async fn invalidate_namespace(&self, namespace: &str) -> Result<(), anyhow::Error> {
        let pattern = format!("{}:*", namespace);
        self.invalidate_pattern(&pattern).await
    }

    /// Invalidate keys by tag (using Redis sets)
    pub async fn invalidate_by_tag(&self, tag: &str) -> Result<(), anyhow::Error> {
        let mut conn = self.redis_client.get_multiplexed_async_connection().await?;
        
        // Get all keys associated with this tag
        let keys: Vec<String> = redis::cmd("SMEMBERS")
            .arg(format!("cache:tags:{}", tag))
            .query_async(&mut conn)
            .await?;
        
        if !keys.is_empty() {
            self.invalidate_keys(keys.clone()).await?;
            info!(tag = %tag, count = keys.len(), "Invalidated keys by tag");
        }
        
        Ok(())
    }

    /// Background processor for handling invalidation events
    async fn background_processor(
        redis_client: Arc<redis::Client>,
        mut rx: mpsc::Receiver<CacheInvalidationEvent>,
        config: CacheInvalidationConfig,
    ) {
        info!("Starting cache invalidation background processor");
        
        while let Some(event) = rx.recv().await {
            debug!(strategy = ?event.strategy, targets_count = event.targets.len(), "Processing invalidation event");
            
            let result = match event.strategy {
                InvalidationStrategy::Key => {
                    Self::process_key_invalidation(&redis_client, &event.targets, &config).await
                }
                InvalidationStrategy::Pattern => {
                    Self::process_pattern_invalidation(&redis_client, &event.targets, &config).await
                }
                InvalidationStrategy::Namespace => {
                    Self::process_namespace_invalidation(&redis_client, &event.targets, &config).await
                }
                InvalidationStrategy::Tag => {
                    Self::process_tag_invalidation(&redis_client, &event.targets, &config).await
                }
            };
            
            if let Err(e) = result {
                error!(error = ?e, "Failed to process invalidation event");
            }
        }
    }

    async fn process_key_invalidation(
        redis_client: &Arc<redis::Client>,
        keys: &[String],
        config: &CacheInvalidationConfig,
    ) -> Result<(), anyhow::Error> {
        let mut attempts = 0;
        loop {
            match Self::invalidate_keys_impl(redis_client, keys.to_vec()).await {
                Ok(_) => return Ok(()),
                Err(e) => {
                    attempts += 1;
                    if attempts >= config.retry_attempts {
                        return Err(e);
                    }
                    
                    tokio::time::sleep(std::time::Duration::from_millis(config.retry_delay_ms)).await;
                }
            }
        }
    }

    async fn process_pattern_invalidation(
        redis_client: &Arc<redis::Client>,
        patterns: &[String],
        config: &CacheInvalidationConfig,
    ) -> Result<(), anyhow::Error> {
        let mut attempts = 0;
        loop {
            match Self::invalidate_patterns_impl(redis_client, patterns.to_vec()).await {
                Ok(_) => return Ok(()),
                Err(e) => {
                    attempts += 1;
                    if attempts >= config.retry_attempts {
                        return Err(e);
                    }
                    
                    tokio::time::sleep(std::time::Duration::from_millis(config.retry_delay_ms)).await;
                }
            }
        }
    }

    async fn process_namespace_invalidation(
        redis_client: &Arc<redis::Client>,
        namespaces: &[String],
        config: &CacheInvalidationConfig,
    ) -> Result<(), anyhow::Error> {
        let mut attempts = 0;
        loop {
            match Self::invalidate_namespaces_impl(redis_client, namespaces.to_vec()).await {
                Ok(_) => return Ok(()),
                Err(e) => {
                    attempts += 1;
                    if attempts >= config.retry_attempts {
                        return Err(e);
                    }
                    
                    tokio::time::sleep(std::time::Duration::from_millis(config.retry_delay_ms)).await;
                }
            }
        }
    }

    async fn process_tag_invalidation(
        redis_client: &Arc<redis::Client>,
        tags: &[String],
        config: &CacheInvalidationConfig,
    ) -> Result<(), anyhow::Error> {
        let mut attempts = 0;
        loop {
            match Self::invalidate_tags_impl(redis_client, tags.to_vec()).await {
                Ok(_) => return Ok(()),
                Err(e) => {
                    attempts += 1;
                    if attempts >= config.retry_attempts {
                        return Err(e);
                    }
                    
                    tokio::time::sleep(std::time::Duration::from_millis(config.retry_delay_ms)).await;
                }
            }
        }
    }

    async fn invalidate_keys_impl(
        redis_client: &Arc<redis::Client>,
        keys: Vec<String>,
    ) -> Result<(), anyhow::Error> {
        let mut conn = redis_client.get_multiplexed_async_connection().await?;
        
        // Use pipelining for better performance
        let mut pipe = redis::pipe();
        for key in &keys {
            pipe.del(key);
        }
        
        let _: () = pipe.query_async(&mut conn).await?;
        Ok(())
    }

    async fn invalidate_patterns_impl(
        redis_client: &Arc<redis::Client>,
        patterns: Vec<String>,
    ) -> Result<(), anyhow::Error> {
        for pattern in &patterns {
            let mut conn = redis_client.get_multiplexed_async_connection().await?;
            
            // Get all keys matching the pattern
            let keys: Vec<String> = redis::cmd("KEYS")
                .arg(pattern)
                .query_async(&mut conn)
                .await?;
            
            if !keys.is_empty() {
                Self::invalidate_keys_impl(redis_client, keys).await?;
            }
        }
        Ok(())
    }

    async fn invalidate_namespaces_impl(
        redis_client: &Arc<redis::Client>,
        namespaces: Vec<String>,
    ) -> Result<(), anyhow::Error> {
        for namespace in &namespaces {
            let pattern = format!("{}:*", namespace);
            Self::invalidate_patterns_impl(redis_client, vec![pattern]).await?;
        }
        Ok(())
    }

    async fn invalidate_tags_impl(
        redis_client: &Arc<redis::Client>,
        tags: Vec<String>,
    ) -> Result<(), anyhow::Error> {
        for tag in &tags {
            let mut conn = redis_client.get_multiplexed_async_connection().await?;
            
            // Get all keys associated with this tag
            let keys: Vec<String> = redis::cmd("SMEMBERS")
                .arg(format!("cache:tags:{}", tag))
                .query_async(&mut conn)
                .await?;
            if !keys.is_empty() {
                Self::invalidate_keys_impl(redis_client, keys).await?;
            }
        }
        Ok(())
    }
}

// ---------------------------------------------------------------------------
// Transaction Simulation Cache (Moka L1 + Redis L2)
// ---------------------------------------------------------------------------

use std::time::Duration;

/// Key structure for transaction simulation caching.
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct SimulationCacheKey {
    pub contract_id: String,
    pub function: String,
    pub args_hash: String,
    pub ledger_sequence: u64,
}

impl SimulationCacheKey {
    pub fn new(contract_id: impl Into<String>, function: impl Into<String>, args_hash: impl Into<String>, ledger_sequence: u64) -> Self {
        Self {
            contract_id: contract_id.into(),
            function: function.into(),
            args_hash: args_hash.into(),
            ledger_sequence,
        }
    }

    pub fn to_redis_key(&self) -> String {
        format!("sim:{}:{}:{}:{}", self.contract_id, self.function, self.args_hash, self.ledger_sequence)
    }
}

/// Simulation execution result payload.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SimulationResult {
    pub success: bool,
    pub result_xdr: String,
    pub cpu_instructions: u64,
    pub memory_bytes: u64,
    pub footprint: Vec<String>,
}

/// Two-tier simulation cache manager (Moka in-memory + Redis cluster).
#[derive(Clone)]
pub struct SimulationCacheManager {
    moka_cache: moka::future::Cache<String, SimulationResult>,
    redis_client: Arc<redis::Client>,
    ttl: Duration,
}

impl SimulationCacheManager {
    pub fn new(redis_client: Arc<redis::Client>, ttl_secs: u64) -> Self {
        let moka_cache = moka::future::Cache::builder()
            .max_capacity(10_000)
            .time_to_live(Duration::from_secs(ttl_secs))
            .build();

        Self {
            moka_cache,
            redis_client,
            ttl: Duration::from_secs(ttl_secs),
        }
    }

    pub async fn get(&self, key: &SimulationCacheKey) -> Option<SimulationResult> {
        let redis_key = key.to_redis_key();

        // Level 1: Moka in-memory cache
        if let Some(res) = self.moka_cache.get(&redis_key).await {
            debug!(key = %redis_key, "Simulation cache L1 hit (Moka)");
            return Some(res);
        }

        // Level 2: Redis cluster cache
        if let Ok(mut conn) = self.redis_client.get_multiplexed_async_connection().await {
            let res: Result<Option<String>, _> = conn.get(&redis_key).await;
            if let Ok(Some(cached_json)) = res {
                if let Ok(res) = serde_json::from_str::<SimulationResult>(&cached_json) {
                    debug!(key = %redis_key, "Simulation cache L2 hit (Redis)");
                    self.moka_cache.insert(redis_key, res.clone()).await;
                    return Some(res);
                }
            }
        }

        None
    }

    pub async fn insert(&self, key: &SimulationCacheKey, result: SimulationResult) {
        let redis_key = key.to_redis_key();
        self.moka_cache.insert(redis_key.clone(), result.clone()).await;

        if let Ok(mut conn) = self.redis_client.get_multiplexed_async_connection().await {
            if let Ok(json) = serde_json::to_string(&result) {
                let _: Result<(), _> = conn.set_ex(redis_key, json, self.ttl.as_secs()).await;
            }
        }
    }

    /// Invalidate simulation cache entries when a ledger modifies the target contract footprint.
    pub async fn invalidate_contract_footprint(&self, contract_id: &str) -> Result<(), anyhow::Error> {
        info!(contract_id = %contract_id, "Invalidating simulation cache for updated contract footprint");
        
        let pattern = format!("sim:{}*", contract_id);
        let mut conn = self.redis_client.get_multiplexed_async_connection().await?;
        let keys: Vec<String> = redis::cmd("KEYS").arg(&pattern).query_async(&mut conn).await?;

        for key in &keys {
            self.moka_cache.invalidate(key).await;
        }

        if !keys.is_empty() {
            redis::cmd("DEL").arg(&keys).query_async::<()>(&mut conn).await?;
        }

        Ok(())
    }
}

/// Helper trait for cache-aware services
pub trait CacheAware {
    /// Invalidate cache for this entity
    fn invalidate_cache(&self) -> Result<(), anyhow::Error>;
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::Instant;

    #[tokio::test]
    async fn test_cache_invalidation_manager_creation() {
        // Test that manager can be created
        let client = redis::Client::open("redis://127.0.0.1/").unwrap();
        let manager = CacheInvalidationManager::new(Arc::new(client), CacheInvalidationConfig::default());
        
        assert!(manager.config.background_processing);
        assert_eq!(manager.config.max_concurrent, 10);
        assert!(manager.config.use_redis_streams);
        assert_eq!(manager.config.stream_name, "cache:invalidation:stream");
    }
    
    #[tokio::test]
    async fn test_invalidation_strategy_enum() {
        assert_eq!(InvalidationStrategy::Key as i32, 0);
        assert_eq!(InvalidationStrategy::Pattern as i32, 1);
        assert_eq!(InvalidationStrategy::Namespace as i32, 2);
        assert_eq!(InvalidationStrategy::Tag as i32, 3);
    }

    #[test]
    fn test_cache_invalidation_event_serde_roundtrip() {
        let mut metadata = HashMap::new();
        metadata.insert("reason".to_string(), "update".to_string());

        let event = CacheInvalidationEvent {
            strategy: InvalidationStrategy::Key,
            targets: vec!["users:123".to_string(), "users:456".to_string()],
            metadata: Some(metadata),
            timestamp: Instant::now(),
            sequence: 42,
        };

        let json = serde_json::to_string(&event).unwrap();
        let deserialized: CacheInvalidationEvent = serde_json::from_str(&json).unwrap();

        assert_eq!(deserialized.strategy, InvalidationStrategy::Key);
        assert_eq!(deserialized.targets, vec!["users:123", "users:456"]);
        assert_eq!(deserialized.sequence, 42);
        assert_eq!(deserialized.metadata.unwrap().get("reason").unwrap(), "update");
    }

    #[tokio::test]
    async fn test_generation_tracker_local_fallback() {
        let client = Arc::new(redis::Client::open("redis://127.0.0.1/").unwrap());
        let tracker = CacheGenerationTracker::new(client);

        assert_eq!(tracker.get_local_generation("test_key").await, None);
        tracker.set_local_generation("test_key", 5).await;
        assert_eq!(tracker.get_local_generation("test_key").await, Some(5));
    }

    #[tokio::test]
    async fn test_simulation_cache_hit_vs_miss_latency() {
        let client = Arc::new(redis::Client::open("redis://127.0.0.1/").unwrap());
        let sim_cache = SimulationCacheManager::new(client, 300);

        let key = SimulationCacheKey::new("C123456789", "transfer", "hash_abc_123", 100500);
        let sim_res = SimulationResult {
            success: true,
            result_xdr: "AAAAAAA==".to_string(),
            cpu_instructions: 150_000,
            memory_bytes: 4096,
            footprint: vec!["contract_data".to_string()],
        };

        // Cache miss timing
        let t_start_miss = Instant::now();
        let miss_res = sim_cache.get(&key).await;
        let miss_duration = t_start_miss.elapsed();

        assert!(miss_res.is_none());

        // Insert into cache
        sim_cache.insert(&key, sim_res).await;

        // Cache hit timing
        let t_start_hit = Instant::now();
        let hit_res = sim_cache.get(&key).await;
        let hit_duration = t_start_hit.elapsed();

        assert!(hit_res.is_some());
        assert_eq!(hit_res.unwrap().cpu_instructions, 150_000);
        assert!(hit_duration < miss_duration + Duration::from_millis(50));
    }
}