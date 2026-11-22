//! Redis Cache Invalidation System
//!
//! This module implements a robust cache invalidation system that works with Redis
//! to ensure data consistency between the database and cache layers.

use redis::AsyncCommands;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Duration, SystemTime, UNIX_EPOCH};
use tokio::sync::mpsc;
use tracing::{debug, error, info, warn};

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
#[derive(Debug, Clone)]
pub struct CacheInvalidationEvent {
    /// Strategy to use for invalidation
    pub strategy: InvalidationStrategy,
    /// Keys or patterns to invalidate
    pub targets: Vec<String>,
    /// Optional namespace or tag information
    pub metadata: Option<HashMap<String, String>>,
    /// Timestamp when event was created
    pub timestamp: std::time::Instant,
}

/// Resilient cache invalidation message with sequence tracking and metadata for reliable delivery.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct ResilientInvalidationMessage {
    /// Monotonically increasing sequence number per cache domain/partition.
    pub sequence: u64,
    /// Invalidation strategy to apply.
    pub strategy: InvalidationStrategy,
    /// Keys or patterns to invalidate.
    pub targets: Vec<String>,
    /// Cache domain or namespace (e.g. "contracts", "simulation", "global").
    pub domain: String,
    /// Epoch timestamp in milliseconds.
    pub timestamp_ms: u64,
}

/// Status returned when validating an incoming message sequence against the local tracker.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SequenceStatus {
    /// Expected in-order sequence received.
    Next,
    /// Sequence gap detected — indicates messages were lost during partition or disconnection!
    Gap {
        missed_count: u64,
        last_seen: u64,
        current: u64,
    },
    /// Duplicate or out-of-order old sequence.
    DuplicateOrOld { last_seen: u64, received: u64 },
}

/// In-memory tracker of cache generation sequences across domains.
#[derive(Debug, Default)]
pub struct GenerationSequenceTracker {
    last_seen: Mutex<HashMap<String, u64>>,
}

impl GenerationSequenceTracker {
    pub fn new() -> Self {
        Self::default()
    }

    /// Evaluates an incoming sequence number for a domain and updates the watermark.
    pub fn check_and_update(&self, domain: &str, sequence: u64) -> SequenceStatus {
        let mut map = self.last_seen.lock().expect("lock tracker");
        let last = map.entry(domain.to_string()).or_insert(0);
        if *last == 0 || sequence == *last + 1 {
            *last = sequence;
            SequenceStatus::Next
        } else if sequence > *last + 1 {
            let missed_count = sequence - *last - 1;
            let status = SequenceStatus::Gap {
                missed_count,
                last_seen: *last,
                current: sequence,
            };
            *last = sequence;
            status
        } else {
            SequenceStatus::DuplicateOrOld {
                last_seen: *last,
                received: sequence,
            }
        }
    }

    pub fn last_sequence(&self, domain: &str) -> u64 {
        let map = self.last_seen.lock().expect("lock tracker");
        map.get(domain).copied().unwrap_or(0)
    }

    pub fn reset_domain(&self, domain: &str) {
        let mut map = self.last_seen.lock().expect("lock tracker");
        map.remove(domain);
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
    /// In-memory generation sequence tracker to detect partition losses
    sequence_tracker: Arc<GenerationSequenceTracker>,
    /// Atomic sequence counter for outbound invalidation messages
    sequence_counter: Arc<AtomicU64>,
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
    /// Redis Stream name for durable invalidations
    pub stream_name: String,
    /// Consumer group name for durable invalidation stream
    pub consumer_group: String,
    /// Whether Redis Streams fallback is enabled for partition resilience
    pub enable_streams_fallback: bool,
}

impl Default for CacheInvalidationConfig {
    fn default() -> Self {
        Self {
            background_processing: true,
            max_concurrent: 10,
            timeout_ms: 5000,
            retry_attempts: 3,
            retry_delay_ms: 100,
            stream_name: "cache:invalidation:stream".to_string(),
            consumer_group: "cache:invalidation:group".to_string(),
            enable_streams_fallback: true,
        }
    }
}

impl CacheInvalidationManager {
    /// Create a new cache invalidation manager
    pub fn new(redis_client: Arc<redis::Client>, config: CacheInvalidationConfig) -> Self {
        let (tx, rx) = mpsc::channel::<CacheInvalidationEvent>(100);

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
            sequence_tracker: Arc::new(GenerationSequenceTracker::new()),
            sequence_counter: Arc::new(AtomicU64::new(1)),
        }
    }

    /// Access the generation sequence tracker.
    pub fn sequence_tracker(&self) -> Arc<GenerationSequenceTracker> {
        self.sequence_tracker.clone()
    }

    /// Next monotonic sequence number.
    pub fn next_sequence(&self) -> u64 {
        self.sequence_counter.fetch_add(1, Ordering::SeqCst)
    }

    /// Publishes a resilient invalidation event:
    /// - Emits to Redis Pub/Sub for immediate cross-pod notification.
    /// - If streams fallback is enabled, also writes to Redis Streams (`XADD`) with generation sequence
    ///   so disconnected pods can replay missed invalidations upon reconnect.
    pub async fn publish_resilient_invalidation(
        &self,
        domain: &str,
        strategy: InvalidationStrategy,
        targets: Vec<String>,
    ) -> Result<ResilientInvalidationMessage, anyhow::Error> {
        let seq = self.next_sequence();
        let now_ms = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap_or_default()
            .as_millis() as u64;

        let msg = ResilientInvalidationMessage {
            sequence: seq,
            strategy,
            targets,
            domain: domain.to_string(),
            timestamp_ms: now_ms,
        };

        if let Ok(mut conn) = self.redis_client.get_multiplexed_async_connection().await {
            let json = serde_json::to_string(&msg)?;
            let channel = format!("cache:invalidation:{}", domain);
            let _: Result<(), _> = conn.publish(&channel, &json).await;

            if self.config.enable_streams_fallback {
                let targets_json = serde_json::to_string(&msg.targets)?;
                let strategy_str = serde_json::to_string(&msg.strategy)?;
                let _: Result<String, _> = redis::cmd("XADD")
                    .arg(&self.config.stream_name)
                    .arg("*")
                    .arg("sequence")
                    .arg(seq.to_string())
                    .arg("domain")
                    .arg(domain)
                    .arg("strategy")
                    .arg(strategy_str)
                    .arg("targets")
                    .arg(targets_json)
                    .arg("timestamp_ms")
                    .arg(now_ms.to_string())
                    .query_async(&mut conn)
                    .await;
            }
        }

        Ok(msg)
    }

    /// Acknowledges a processed message in a Redis Stream consumer group (`XACK`).
    pub async fn ack_stream_message(
        &self,
        stream: &str,
        group: &str,
        message_id: &str,
    ) -> Result<(), anyhow::Error> {
        let mut conn = self.redis_client.get_multiplexed_async_connection().await?;
        let _: () = redis::cmd("XACK")
            .arg(stream)
            .arg(group)
            .arg(message_id)
            .query_async(&mut conn)
            .await?;
        Ok(())
    }

    /// Ensures a Redis Stream consumer group exists (`XGROUP CREATE ... MKSTREAM`).
    pub async fn ensure_consumer_group(
        &self,
        stream: &str,
        group: &str,
    ) -> Result<(), anyhow::Error> {
        let mut conn = self.redis_client.get_multiplexed_async_connection().await?;
        let _: Result<(), _> = redis::cmd("XGROUP")
            .arg("CREATE")
            .arg(stream)
            .arg(group)
            .arg("$")
            .arg("MKSTREAM")
            .query_async(&mut conn)
            .await;
        Ok(())
    }

    /// Handles an incoming resilient message. If a sequence gap is detected (indicating messages were
    /// lost during a network partition), triggers a domain namespace invalidation to prevent stale cache entries.
    pub async fn handle_incoming_resilient_message(
        &self,
        msg: &ResilientInvalidationMessage,
    ) -> Result<SequenceStatus, anyhow::Error> {
        let status = self
            .sequence_tracker
            .check_and_update(&msg.domain, msg.sequence);
        match &status {
            SequenceStatus::Next => {
                debug!(domain = %msg.domain, seq = msg.sequence, "In-sequence invalidation received");
                self.apply_resilient_targets(msg.strategy, &msg.targets, &msg.domain)
                    .await?;
            }
            SequenceStatus::Gap {
                missed_count,
                last_seen,
                current,
            } => {
                warn!(
                    domain = %msg.domain,
                    missed = missed_count,
                    last_seen = last_seen,
                    current = current,
                    "Cache invalidation sequence gap detected (possible Pub/Sub partition loss)! Purging domain cache"
                );
                let _ = self.invalidate_namespace(&msg.domain).await;
                self.apply_resilient_targets(msg.strategy, &msg.targets, &msg.domain)
                    .await?;
            }
            SequenceStatus::DuplicateOrOld {
                last_seen,
                received,
            } => {
                debug!(
                    domain = %msg.domain,
                    last_seen = last_seen,
                    received = received,
                    "Duplicate or old invalidation sequence received; ignoring"
                );
            }
        }
        Ok(status)
    }

    async fn apply_resilient_targets(
        &self,
        strategy: InvalidationStrategy,
        targets: &[String],
        _domain: &str,
    ) -> Result<(), anyhow::Error> {
        match strategy {
            InvalidationStrategy::Key => self.invalidate_keys(targets.to_vec()).await,
            InvalidationStrategy::Pattern => {
                for pattern in targets {
                    let _ = self.invalidate_pattern(pattern).await;
                }
                Ok(())
            }
            InvalidationStrategy::Namespace => {
                for ns in targets {
                    let _ = self.invalidate_namespace(ns).await;
                }
                Ok(())
            }
            InvalidationStrategy::Tag => {
                for tag in targets {
                    let _ = self.invalidate_by_tag(tag).await;
                }
                Ok(())
            }
        }
    }

    /// Enqueue an invalidation event
    pub async fn enqueue_invalidation(
        &self,
        event: CacheInvalidationEvent,
    ) -> Result<(), anyhow::Error> {
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

        // Use SCAN iteration instead of KEYS to avoid blocking the event loop
        let mut cursor = 0u64;
        let mut deleted_count = 0;

        loop {
            let (new_cursor, keys): (u64, Vec<String>) = redis::cmd("SCAN")
                .arg(cursor)
                .arg("MATCH")
                .arg(pattern)
                .arg("COUNT")
                .arg(100)
                .query_async(&mut conn)
                .await?;

            if !keys.is_empty() {
                self.invalidate_keys(keys.clone()).await?;
                deleted_count += keys.len();
            }

            cursor = new_cursor;
            if cursor == 0 {
                break;
            }
        }

        info!(pattern = %pattern, count = deleted_count, "Invalidated keys by pattern");
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
                    Self::process_namespace_invalidation(&redis_client, &event.targets, &config)
                        .await
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

                    tokio::time::sleep(std::time::Duration::from_millis(config.retry_delay_ms))
                        .await;
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

                    tokio::time::sleep(std::time::Duration::from_millis(config.retry_delay_ms))
                        .await;
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

                    tokio::time::sleep(std::time::Duration::from_millis(config.retry_delay_ms))
                        .await;
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

                    tokio::time::sleep(std::time::Duration::from_millis(config.retry_delay_ms))
                        .await;
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

            // Use SCAN iteration instead of KEYS to avoid blocking the event loop
            let mut cursor = 0u64;

            loop {
                let (new_cursor, keys): (u64, Vec<String>) = redis::cmd("SCAN")
                    .arg(cursor)
                    .arg("MATCH")
                    .arg(pattern)
                    .arg("COUNT")
                    .arg(100)
                    .query_async(&mut conn)
                    .await?;

                if !keys.is_empty() {
                    Self::invalidate_keys_impl(redis_client, keys).await?;
                }

                cursor = new_cursor;
                if cursor == 0 {
                    break;
                }
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

/// Key structure for transaction simulation caching.
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct SimulationCacheKey {
    pub contract_id: String,
    pub function: String,
    pub args_hash: String,
    pub ledger_sequence: u64,
}

impl SimulationCacheKey {
    pub fn new(
        contract_id: impl Into<String>,
        function: impl Into<String>,
        args_hash: impl Into<String>,
        ledger_sequence: u64,
    ) -> Self {
        Self {
            contract_id: contract_id.into(),
            function: function.into(),
            args_hash: args_hash.into(),
            ledger_sequence,
        }
    }

    pub fn to_redis_key(&self) -> String {
        format!(
            "sim:{}:{}:{}:{}",
            self.contract_id, self.function, self.args_hash, self.ledger_sequence
        )
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
    sequence_tracker: Arc<GenerationSequenceTracker>,
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
            sequence_tracker: Arc::new(GenerationSequenceTracker::new()),
        }
    }

    pub fn sequence_tracker(&self) -> Arc<GenerationSequenceTracker> {
        self.sequence_tracker.clone()
    }

    pub async fn invalidate_all(&self) {
        self.moka_cache.invalidate_all();
    }

    /// Handles incoming cache generation sequence. If a gap is detected, purges the local L1 cache
    /// to prevent stale cache entries resulting from network partition or missed Pub/Sub events.
    pub async fn handle_generation_sequence(&self, domain: &str, sequence: u64) -> SequenceStatus {
        let status = self.sequence_tracker.check_and_update(domain, sequence);
        if let SequenceStatus::Gap {
            missed_count,
            last_seen,
            current,
        } = &status
        {
            warn!(
                domain,
                missed = missed_count,
                last_seen = last_seen,
                current = current,
                "Simulation cache detected sequence gap; purging local L1 Moka cache"
            );
            self.moka_cache.invalidate_all();
        }
        status
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
        self.moka_cache
            .insert(redis_key.clone(), result.clone())
            .await;

        if let Ok(mut conn) = self.redis_client.get_multiplexed_async_connection().await {
            if let Ok(json) = serde_json::to_string(&result) {
                let _: Result<(), _> = conn.set_ex(redis_key, json, self.ttl.as_secs()).await;
            }
        }
    }

    /// Invalidate simulation cache entries when a ledger modifies the target contract footprint.
    pub async fn invalidate_contract_footprint(
        &self,
        contract_id: &str,
    ) -> Result<(), anyhow::Error> {
        info!(contract_id = %contract_id, "Invalidating simulation cache for updated contract footprint");

        let pattern = format!("sim:{}*", contract_id);
        let mut conn = self.redis_client.get_multiplexed_async_connection().await?;
        let keys: Vec<String> = redis::cmd("KEYS")
            .arg(&pattern)
            .query_async(&mut conn)
            .await?;

        for key in &keys {
            self.moka_cache.invalidate(key).await;
        }

        if !keys.is_empty() {
            redis::cmd("DEL")
                .arg(&keys)
                .query_async::<()>(&mut conn)
                .await?;
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
        let manager =
            CacheInvalidationManager::new(Arc::new(client), CacheInvalidationConfig::default());

        assert!(manager.config.background_processing);
        assert_eq!(manager.config.max_concurrent, 10);
        assert_eq!(manager.sequence_tracker.last_sequence("default"), 0);
    }

    #[tokio::test]
    async fn test_invalidation_strategy_enum() {
        assert_eq!(InvalidationStrategy::Key as i32, 0);
        assert_eq!(InvalidationStrategy::Pattern as i32, 1);
        assert_eq!(InvalidationStrategy::Namespace as i32, 2);
        assert_eq!(InvalidationStrategy::Tag as i32, 3);
    }

    #[tokio::test]
    async fn test_simulation_cache_hit_vs_miss_latency() {
        let client = Arc::new(redis::Client::open("redis://127.0.0.1/").unwrap());
        let sim_cache = SimulationCacheManager::new(client, 300);

        // Use a unique key so repeated runs cannot observe a cached value.
        let key = SimulationCacheKey::new(
            format!("C{}", uuid::Uuid::new_v4().simple()),
            "transfer",
            "hash_abc_123",
            100500,
        );
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

    #[test]
    fn test_generation_sequence_tracker_flow() {
        let tracker = GenerationSequenceTracker::new();
        let domain = "contracts";

        // First event
        assert_eq!(tracker.check_and_update(domain, 1), SequenceStatus::Next);
        assert_eq!(tracker.last_sequence(domain), 1);

        // Next event in order
        assert_eq!(tracker.check_and_update(domain, 2), SequenceStatus::Next);
        assert_eq!(tracker.last_sequence(domain), 2);

        // Gap event: simulate 3 and 4 were lost during partition; 5 arrives
        let gap_status = tracker.check_and_update(domain, 5);
        assert_eq!(
            gap_status,
            SequenceStatus::Gap {
                missed_count: 2,
                last_seen: 2,
                current: 5,
            }
        );
        assert_eq!(tracker.last_sequence(domain), 5);

        // Duplicate or old event arrives
        assert_eq!(
            tracker.check_and_update(domain, 4),
            SequenceStatus::DuplicateOrOld {
                last_seen: 5,
                received: 4,
            }
        );

        // Reset domain
        tracker.reset_domain(domain);
        assert_eq!(tracker.last_sequence(domain), 0);
    }

    #[tokio::test]
    async fn test_simulation_cache_gap_purges_l1() {
        let client = Arc::new(redis::Client::open("redis://127.0.0.1/").unwrap());
        let sim_cache = SimulationCacheManager::new(client, 300);

        let key = SimulationCacheKey::new("C999", "exec", "h123", 1);
        let sim_res = SimulationResult {
            success: true,
            result_xdr: "AQ==".to_string(),
            cpu_instructions: 1000,
            memory_bytes: 256,
            footprint: vec![],
        };

        // Seed L1 cache
        sim_cache
            .moka_cache
            .insert(key.to_redis_key(), sim_res)
            .await;
        assert!(sim_cache
            .moka_cache
            .get(&key.to_redis_key())
            .await
            .is_some());

        // Initial sequence 10
        assert_eq!(
            sim_cache.handle_generation_sequence("sim", 10).await,
            SequenceStatus::Next
        );
        // Sequence 11 in-order
        assert_eq!(
            sim_cache.handle_generation_sequence("sim", 11).await,
            SequenceStatus::Next
        );
        assert!(sim_cache
            .moka_cache
            .get(&key.to_redis_key())
            .await
            .is_some());

        // Gap: sequence 15 arrives (12, 13, 14 lost in partition)
        let gap = sim_cache.handle_generation_sequence("sim", 15).await;
        assert!(matches!(
            gap,
            SequenceStatus::Gap {
                missed_count: 3,
                ..
            }
        ));

        // L1 cache must be purged to maintain consistency
        assert!(sim_cache
            .moka_cache
            .get(&key.to_redis_key())
            .await
            .is_none());
    }

    #[test]
    fn test_resilient_invalidation_message_serialization() {
        let msg = ResilientInvalidationMessage {
            sequence: 42,
            strategy: InvalidationStrategy::Key,
            targets: vec!["key1".to_string(), "key2".to_string()],
            domain: "contracts".to_string(),
            timestamp_ms: 1700000000000,
        };

        let json = serde_json::to_string(&msg).unwrap();
        let decoded: ResilientInvalidationMessage = serde_json::from_str(&json).unwrap();
        assert_eq!(msg, decoded);
    }
}
