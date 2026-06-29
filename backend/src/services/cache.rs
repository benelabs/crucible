use std::collections::HashMap;
use std::sync::Arc;
use std::time::{Duration, Instant};
use tokio::sync::RwLock;

#[derive(Clone)]
struct CacheEntry {
    value: String,
    expires_at: Option<Instant>,
}

#[derive(Clone, Default)]
pub struct MultiLevelCache {
    // Simple in-memory local layer for fast reads. This is a mock; a real
    // implementation would add Redis or another remote layer.
    local: Arc<RwLock<HashMap<String, CacheEntry>>>,
}

impl MultiLevelCache {
    pub fn new() -> Self {
        Self {
            local: Arc::new(RwLock::new(HashMap::new())),
        }
    }

    pub async fn get(&self, key: &str) -> Option<String> {
        let now = Instant::now();
        // Check local cache
        {
            let read = self.local.read().await;
            if let Some(entry) = read.get(key) {
                if let Some(exp) = entry.expires_at {
                    if now >= exp {
                        return None;
                    }
                }
                return Some(entry.value.clone());
            }
        }

        // Remote layer would be queried here in a production implementation.
        // For this mock we return None on miss.
        None
    }

    pub async fn set(&self, key: String, value: String, ttl: Option<Duration>) {
        let expires_at = ttl.map(|t| Instant::now() + t);
        let entry = CacheEntry { value, expires_at };
        let mut write = self.local.write().await;
        write.insert(key, entry);
    }

    pub async fn invalidate(&self, key: &str) {
        let mut write = self.local.write().await;
        write.remove(key);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::Duration;

    #[tokio::test]
    async fn basic_set_get() {
        let cache = MultiLevelCache::new();
        cache.set("k".to_string(), "v".to_string(), None).await;
        let v = cache.get("k").await;
        assert_eq!(v.as_deref(), Some("v"));
    }

    #[tokio::test]
    async fn ttl_expires() {
        let cache = MultiLevelCache::new();
        cache
            .set("k2".to_string(), "v2".to_string(), Some(Duration::from_millis(1)))
            .await;
        tokio::time::sleep(Duration::from_millis(5)).await;
        let v = cache.get("k2").await;
        assert!(v.is_none());
    }
}
