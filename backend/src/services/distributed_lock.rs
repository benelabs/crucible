//! Distributed lock manager implementing the Redis Redlock algorithm.
//!
//! Ensures idempotent, single-execution contract compilation and deployment
//! tasks across horizontally scaled backend pods.
//!
//! # Features
//! - Majority-quorum acquisition across N Redis nodes
//! - TTL auto-renewal heartbeats while the lock is held
//! - RAII [`LockGuard`] that releases on drop or task cancellation
//!
//! Location: backend/src/services/distributed_lock.rs
//! Production requirement: Distributed Lock Manager with Redis Redlock Algorithm

use std::collections::HashMap;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use thiserror::Error;
use tokio::sync::Notify;
use tokio::task::JoinHandle;
use tracing::{debug, warn};
use uuid::Uuid;

/// Default lock TTL before auto-expiry if the holder dies.
pub const DEFAULT_LOCK_TTL: Duration = Duration::from_secs(30);
/// How often heartbeats renew the TTL (must be < TTL).
pub const DEFAULT_HEARTBEAT_INTERVAL: Duration = Duration::from_secs(10);

/// Errors produced by the Redlock manager.
#[derive(Debug, Error, Clone, PartialEq, Eq)]
pub enum LockError {
    #[error("failed to acquire lock '{resource}' (quorum not reached)")]
    NotAcquired { resource: String },
    #[error("lock '{resource}' is already held")]
    AlreadyHeld { resource: String },
    #[error("lock expired or was stolen before release")]
    LostLock,
    #[error("invalid configuration: {0}")]
    InvalidConfig(String),
}

/// Configuration for Redlock acquisition and renewal.
#[derive(Debug, Clone)]
pub struct RedlockConfig {
    /// Lock time-to-live after last successful heartbeat.
    pub ttl: Duration,
    /// Interval between TTL renewal heartbeats.
    pub heartbeat_interval: Duration,
    /// Maximum time spent retrying acquisition.
    pub acquire_timeout: Duration,
    /// Delay between acquisition retries.
    pub retry_delay: Duration,
}

impl Default for RedlockConfig {
    fn default() -> Self {
        Self {
            ttl: DEFAULT_LOCK_TTL,
            heartbeat_interval: DEFAULT_HEARTBEAT_INTERVAL,
            acquire_timeout: Duration::from_secs(5),
            retry_delay: Duration::from_millis(50),
        }
    }
}

impl RedlockConfig {
    fn validate(&self) -> Result<(), LockError> {
        if self.ttl.is_zero() {
            return Err(LockError::InvalidConfig("ttl must be positive".into()));
        }
        if self.heartbeat_interval >= self.ttl {
            return Err(LockError::InvalidConfig(
                "heartbeat_interval must be less than ttl".into(),
            ));
        }
        Ok(())
    }
}

/// Abstract Redis node used by Redlock (allows in-memory test doubles).
#[async_trait::async_trait]
pub trait RedisNode: Send + Sync {
    /// `SET key value NX PX ttl_ms` — returns true when the key was set.
    async fn set_nx_px(&self, key: &str, value: &str, ttl_ms: u64) -> bool;
    /// Renew TTL only if the value still matches (compare-and-expire).
    async fn compare_and_pexpire(&self, key: &str, value: &str, ttl_ms: u64) -> bool;
    /// Delete only if the value still matches.
    async fn compare_and_del(&self, key: &str, value: &str) -> bool;
}

/// In-memory Redis node suitable for unit / concurrency tests.
#[derive(Debug, Default)]
pub struct InMemoryRedisNode {
    inner: Mutex<HashMap<String, (String, Instant)>>,
}

impl InMemoryRedisNode {
    pub fn new() -> Self {
        Self::default()
    }

    fn purge_expired(map: &mut HashMap<String, (String, Instant)>) {
        let now = Instant::now();
        map.retain(|_, (_, expires_at)| *expires_at > now);
    }
}

#[async_trait::async_trait]
impl RedisNode for InMemoryRedisNode {
    async fn set_nx_px(&self, key: &str, value: &str, ttl_ms: u64) -> bool {
        let mut map = self.inner.lock().expect("lock");
        Self::purge_expired(&mut map);
        if map.contains_key(key) {
            return false;
        }
        map.insert(
            key.to_string(),
            (value.to_string(), Instant::now() + Duration::from_millis(ttl_ms)),
        );
        true
    }

    async fn compare_and_pexpire(&self, key: &str, value: &str, ttl_ms: u64) -> bool {
        let mut map = self.inner.lock().expect("lock");
        Self::purge_expired(&mut map);
        match map.get_mut(key) {
            Some((stored, expires_at)) if stored == value => {
                *expires_at = Instant::now() + Duration::from_millis(ttl_ms);
                true
            }
            _ => false,
        }
    }

    async fn compare_and_del(&self, key: &str, value: &str) -> bool {
        let mut map = self.inner.lock().expect("lock");
        Self::purge_expired(&mut map);
        match map.get(key) {
            Some((stored, _)) if stored == value => {
                map.remove(key);
                true
            }
            _ => false,
        }
    }
}

/// Redlock distributed mutual-exclusion manager.
pub struct RedlockManager {
    nodes: Vec<Arc<dyn RedisNode>>,
    config: RedlockConfig,
}

impl RedlockManager {
    /// Create a manager over one or more Redis nodes.
    pub fn new(nodes: Vec<Arc<dyn RedisNode>>, config: RedlockConfig) -> Result<Self, LockError> {
        config.validate()?;
        if nodes.is_empty() {
            return Err(LockError::InvalidConfig(
                "at least one redis node is required".into(),
            ));
        }
        Ok(Self { nodes, config })
    }

    /// Quorum size — majority of configured nodes.
    pub fn quorum(&self) -> usize {
        self.nodes.len() / 2 + 1
    }

    /// Attempt to acquire `resource` until timeout, returning an RAII guard.
    pub async fn acquire(&self, resource: impl Into<String>) -> Result<LockGuard, LockError> {
        let resource = resource.into();
        let token = Uuid::new_v4().to_string();
        let deadline = Instant::now() + self.config.acquire_timeout;
        let ttl_ms = self.config.ttl.as_millis() as u64;

        loop {
            let mut votes = 0usize;
            for node in &self.nodes {
                if node.set_nx_px(&resource, &token, ttl_ms).await {
                    votes += 1;
                }
            }

            if votes >= self.quorum() {
                debug!(%resource, votes, quorum = self.quorum(), "redlock acquired");
                return Ok(LockGuard::spawn(
                    self.nodes.clone(),
                    resource,
                    token,
                    self.config.clone(),
                ));
            }

            // Best-effort unlock of partial acquisitions before retrying.
            for node in &self.nodes {
                let _ = node.compare_and_del(&resource, &token).await;
            }

            if Instant::now() >= deadline {
                return Err(LockError::NotAcquired { resource });
            }
            tokio::time::sleep(self.config.retry_delay).await;
        }
    }
}

/// RAII lock guard. Releases the lock on drop and runs TTL heartbeats while held.
pub struct LockGuard {
    nodes: Vec<Arc<dyn RedisNode>>,
    resource: String,
    token: String,
    stop: Arc<AtomicBool>,
    notify: Arc<Notify>,
    heartbeat: Option<JoinHandle<()>>,
    released: bool,
}

impl LockGuard {
    fn spawn(
        nodes: Vec<Arc<dyn RedisNode>>,
        resource: String,
        token: String,
        config: RedlockConfig,
    ) -> Self {
        let stop = Arc::new(AtomicBool::new(false));
        let notify = Arc::new(Notify::new());
        let heartbeat = {
            let nodes = nodes.clone();
            let resource = resource.clone();
            let token = token.clone();
            let stop = stop.clone();
            let notify = notify.clone();
            let ttl_ms = config.ttl.as_millis() as u64;
            let interval = config.heartbeat_interval;

            Some(tokio::spawn(async move {
                loop {
                    tokio::select! {
                        _ = notify.notified() => break,
                        _ = tokio::time::sleep(interval) => {
                            if stop.load(Ordering::SeqCst) {
                                break;
                            }
                            let mut renewed = 0usize;
                            for node in &nodes {
                                if node.compare_and_pexpire(&resource, &token, ttl_ms).await {
                                    renewed += 1;
                                }
                            }
                            let quorum = nodes.len() / 2 + 1;
                            if renewed < quorum {
                                warn!(%resource, renewed, quorum, "redlock heartbeat lost quorum");
                                break;
                            }
                        }
                    }
                }
            }))
        };

        Self {
            nodes,
            resource,
            token,
            stop,
            notify,
            heartbeat,
            released: false,
        }
    }

    /// Resource name this guard holds.
    pub fn resource(&self) -> &str {
        &self.resource
    }

    /// Unique ownership token.
    pub fn token(&self) -> &str {
        &self.token
    }

    /// Explicitly release the lock (also called from [`Drop`]).
    pub async fn release(mut self) -> Result<(), LockError> {
        self.release_inner().await
    }

    async fn release_inner(&mut self) -> Result<(), LockError> {
        if self.released {
            return Ok(());
        }
        self.released = true;
        self.stop.store(true, Ordering::SeqCst);
        self.notify.notify_one();
        if let Some(handle) = self.heartbeat.take() {
            let _ = handle.await;
        }

        let mut released = 0usize;
        for node in &self.nodes {
            if node.compare_and_del(&self.resource, &self.token).await {
                released += 1;
            }
        }
        let quorum = self.nodes.len() / 2 + 1;
        if released < quorum {
            return Err(LockError::LostLock);
        }
        Ok(())
    }
}

impl Drop for LockGuard {
    fn drop(&mut self) {
        if self.released {
            return;
        }
        self.stop.store(true, Ordering::SeqCst);
        self.notify.notify_one();
        // Best-effort synchronous release for cancellation / unwind paths.
        let nodes = self.nodes.clone();
        let resource = self.resource.clone();
        let token = self.token.clone();
        if let Ok(handle) = tokio::runtime::Handle::try_current() {
            handle.spawn(async move {
                for node in nodes {
                    let _ = node.compare_and_del(&resource, &token).await;
                }
            });
        }
        self.released = true;
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::AtomicUsize;

    fn three_node_manager(ttl: Duration) -> RedlockManager {
        let nodes: Vec<Arc<dyn RedisNode>> = vec![
            Arc::new(InMemoryRedisNode::new()),
            Arc::new(InMemoryRedisNode::new()),
            Arc::new(InMemoryRedisNode::new()),
        ];
        RedlockManager::new(
            nodes,
            RedlockConfig {
                ttl,
                heartbeat_interval: ttl / 3,
                acquire_timeout: Duration::from_millis(200),
                retry_delay: Duration::from_millis(10),
            },
        )
        .unwrap()
    }

    #[tokio::test]
    async fn acquire_and_release_round_trip() {
        let mgr = three_node_manager(Duration::from_secs(2));
        let guard = mgr.acquire("deploy:contract-a").await.unwrap();
        assert_eq!(guard.resource(), "deploy:contract-a");
        guard.release().await.unwrap();
    }

    #[tokio::test]
    async fn mutual_exclusion_across_workers() {
        let mgr = Arc::new(three_node_manager(Duration::from_secs(3)));
        let counter = Arc::new(AtomicUsize::new(0));
        let max_seen = Arc::new(AtomicUsize::new(0));

        let mut handles = Vec::new();
        for _ in 0..8 {
            let mgr = mgr.clone();
            let counter = counter.clone();
            let max_seen = max_seen.clone();
            handles.push(tokio::spawn(async move {
                let guard = mgr.acquire("compile:wasm").await.unwrap();
                let active = counter.fetch_add(1, Ordering::SeqCst) + 1;
                max_seen.fetch_max(active, Ordering::SeqCst);
                tokio::time::sleep(Duration::from_millis(20)).await;
                counter.fetch_sub(1, Ordering::SeqCst);
                guard.release().await.unwrap();
            }));
        }

        for h in handles {
            h.await.unwrap();
        }

        assert_eq!(
            max_seen.load(Ordering::SeqCst),
            1,
            "more than one worker held the lock concurrently"
        );
    }

    #[tokio::test]
    async fn second_acquire_fails_while_held() {
        let mgr = three_node_manager(Duration::from_secs(5));
        let _held = mgr.acquire("deploy:unique").await.unwrap();
        let err = mgr.acquire("deploy:unique").await.unwrap_err();
        assert!(matches!(err, LockError::NotAcquired { .. }));
    }

    #[tokio::test]
    async fn drop_releases_lock_for_next_acquirer() {
        let mgr = three_node_manager(Duration::from_secs(3));
        {
            let _guard = mgr.acquire("deploy:drop").await.unwrap();
        }
        // Allow the spawned drop release to complete.
        tokio::time::sleep(Duration::from_millis(30)).await;
        let guard = mgr.acquire("deploy:drop").await.unwrap();
        guard.release().await.unwrap();
    }

    #[test]
    fn rejects_invalid_heartbeat_config() {
        let err = RedlockManager::new(
            vec![Arc::new(InMemoryRedisNode::new())],
            RedlockConfig {
                ttl: Duration::from_secs(1),
                heartbeat_interval: Duration::from_secs(2),
                ..RedlockConfig::default()
            },
        )
        .unwrap_err();
        assert!(matches!(err, LockError::InvalidConfig(_)));
    }
}
