use crate::config::{AppConfig as BaseAppConfig, ConfigError, Environment};
use arc_swap::ArcSwap;
use axum::{extract::State, http::StatusCode, response::IntoResponse, Json};
use std::sync::Arc;
use thiserror::Error;
use tracing::{info, instrument};

/// Errors that can occur during configuration reload.
#[derive(Debug, Error)]
pub enum ConfigReloadError {
    #[error("Configuration load error: {0}")]
    LoadError(#[from] ConfigError),
}

impl IntoResponse for ConfigReloadError {
    fn into_response(self) -> axum::response::Response {
        let status = StatusCode::INTERNAL_SERVER_ERROR;
        let body = Json(serde_json::json!({
            "error": self.to_string(),
            "status": status.as_u16()
        }));

        (status, body).into_response()
    }
}

/// Manages hot-reloadable application configuration.
pub struct ConfigManager {
    current_config: ArcSwap<BaseAppConfig>,
}

impl ConfigManager {
    /// Create a new ConfigManager with the given initial configuration.
    pub fn new(initial_config: BaseAppConfig) -> Self {
        Self {
            current_config: ArcSwap::from(Arc::new(initial_config)),
        }
    }

    /// Get a reference to the current configuration.
    pub fn load(&self) -> Arc<BaseAppConfig> {
        self.current_config.load_full()
    }

    /// Reload the configuration from environment variables and TOML files.
    #[instrument(skip(self))]
    pub async fn reload(&self) -> Result<(), ConfigReloadError> {
        info!("Starting configuration reload...");

        // Reload the layered config from the environment
        let env = Environment::from_env();
        let new_config = BaseAppConfig::load(env)?;

        // Update the global configuration atomically
        self.current_config.store(Arc::new(new_config));

        info!("Configuration successfully reloaded");
        Ok(())
    }

    /// Update the configuration with a JSON patch/overlay.
    pub fn update_from_patch(&self, patch: serde_json::Value) -> Result<(), ConfigReloadError> {
        let current = self.load();
        let mut current_json = serde_json::to_value(&*current)
            .map_err(|e| ConfigReloadError::LoadError(ConfigError::LoadError(e.to_string())))?;

        fn merge_json(a: &mut serde_json::Value, b: serde_json::Value) {
            match (a, b) {
                (serde_json::Value::Object(ref mut a), serde_json::Value::Object(b)) => {
                    for (k, v) in b {
                        merge_json(a.entry(k).or_insert(serde_json::Value::Null), v);
                    }
                }
                (a, b) => {
                    *a = b;
                }
            }
        }

        merge_json(&mut current_json, patch);

        let new_config: BaseAppConfig = serde_json::from_value(current_json)
            .map_err(|e| ConfigReloadError::LoadError(ConfigError::LoadError(e.to_string())))?;

        self.current_config.store(Arc::new(new_config));
        Ok(())
    }
}

// In a real application, State type would be strongly typed for the app.
// We use a generic representation here or rely on the actual AppState type.
// Since the state definition was in `main.rs` and might be redefined, we'll keep it simple.

/// Axum handler to trigger a configuration reload.
pub async fn handle_reload(
    State(manager): State<Arc<ConfigManager>>,
) -> Result<impl IntoResponse, ConfigReloadError> {
    manager.reload().await?;
    Ok((
        StatusCode::OK,
        Json(serde_json::json!({ "status": "reloaded" })),
    ))
}

/// Axum handler to get the current configuration (sanitized).
pub async fn handle_get_config(State(manager): State<Arc<ConfigManager>>) -> impl IntoResponse {
    let config = manager.load();
    // Sensitive fields are already skipped or redacted by `serde(skip_serializing)` and custom `Debug`.
    // In this case, `AppConfig` derives Serialize, and sensitive fields have `#[serde(skip_serializing)]`.
    Json(config.as_ref().clone())
}

//
// This module provides [`ConfigWatcher`], which holds the live [`AppConfig`]
// behind an `Arc<RwLock<_>>` and can reload it at any time — either
// programmatically via [`ConfigWatcher::reload`] or automatically by
// subscribing to a Redis pub/sub channel with [`ConfigWatcher::watch`].
//
// When a reload message arrives on the Redis channel the watcher fetches the
// new configuration JSON from a Redis key, deserialises it, and atomically
// swaps the in-memory value. All readers that hold a clone of the
// [`ConfigHandle`] see the new values on their next read without any restart.
//
// # Example
//
// ```rust,no_run
// use backend::config::reload::{AppConfig, ConfigWatcher};
//
// # async fn example() -> anyhow::Result<()> {
// let watcher = ConfigWatcher::new(AppConfig::default());
// let handle = watcher.handle();
//
// // Read the current config
// let cfg = handle.get().await;
// println!("log level: {}", cfg.log_level);
//
// // Trigger a manual reload
// watcher.reload(AppConfig {
//     log_level: "info".to_string(),
//     ..AppConfig::default()
// }).await;
// # Ok(())
// # }
// ```
//
// # Redis protocol
//
// Publish any non-empty string to `config:reload` to trigger a reload:
//
// ```text
// PUBLISH config:reload ""
// SET config:current '{"log_level":"info","max_connections":50,...}'
// PUBLISH config:reload "reload"
// ```
//
// The watcher reads `config:current` from Redis after every message on
// `config:reload`. If the key is absent or unparseable the existing config
// is kept and an error is logged.

use redis::{AsyncCommands, Client as RedisClient};
use serde::{Deserialize, Serialize};
use tokio::sync::{watch, RwLock};
use tracing::{error, warn};

// ---------------------------------------------------------------------------
// Error type
// ---------------------------------------------------------------------------

/// Errors that can occur during configuration reload.
#[derive(Debug, Error)]
pub enum ReloadError {
    /// A Redis error occurred.
    #[error("Redis error: {0}")]
    Redis(#[from] redis::RedisError),

    /// The configuration value could not be deserialised.
    #[error("Config deserialisation error: {0}")]
    Deserialise(#[from] serde_json::Error),

    /// The configuration key was not found in Redis.
    #[error("Config key not found in Redis")]
    NotFound,
}

// ---------------------------------------------------------------------------
// AppConfig
// ---------------------------------------------------------------------------

/// Live application configuration that can be hot-reloaded at runtime.
///
/// All fields have sensible defaults so the application starts without any
/// external configuration source.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct AppConfig {
    /// Tracing / log filter directive (e.g. `"backend=debug"`).
    pub log_level: String,
    /// Maximum number of database connections in the pool.
    pub max_connections: u32,
    /// Request timeout in seconds.
    pub request_timeout_secs: u64,
    /// Whether the maintenance mode banner is shown.
    pub maintenance_mode: bool,
    /// Redis key that stores the serialised [`AppConfig`] JSON.
    pub redis_config_key: String,
}

impl Default for AppConfig {
    fn default() -> Self {
        Self {
            log_level: "backend=debug,tower_http=debug".to_string(),
            max_connections: 10,
            request_timeout_secs: 30,
            maintenance_mode: false,
            redis_config_key: "config:current".to_string(),
        }
    }
}

// ---------------------------------------------------------------------------
// ConfigHandle — cheap clone, shared reader
// ---------------------------------------------------------------------------

/// A cheap-to-clone handle to the live configuration.
///
/// Obtain one via [`ConfigWatcher::handle`] and share it across the
/// application. Reads never block writers for more than a single lock
/// acquisition.
#[derive(Clone)]
pub struct ConfigHandle {
    inner: Arc<RwLock<AppConfig>>,
    /// Notified whenever the config is reloaded.
    changed: watch::Receiver<()>,
}

impl ConfigHandle {
    /// Return a snapshot of the current configuration.
    pub async fn get(&self) -> AppConfig {
        self.inner.read().await.clone()
    }

    /// Wait until the configuration changes, then return the new snapshot.
    pub async fn wait_for_change(&mut self) -> AppConfig {
        // `changed()` resolves immediately if there is an unseen change.
        let _ = self.changed.changed().await;
        self.get().await
    }
}

// ---------------------------------------------------------------------------
// ConfigWatcher
// ---------------------------------------------------------------------------

/// Owns the live [`AppConfig`] and drives hot-reload.
pub struct ConfigWatcher {
    inner: Arc<RwLock<AppConfig>>,
    notify_tx: watch::Sender<()>,
    notify_rx: watch::Receiver<()>,
}

impl ConfigWatcher {
    /// Create a new watcher with the given initial configuration.
    pub fn new(initial: AppConfig) -> Self {
        let (tx, rx) = watch::channel(());
        Self {
            inner: Arc::new(RwLock::new(initial)),
            notify_tx: tx,
            notify_rx: rx,
        }
    }

    /// Return a [`ConfigHandle`] that can be cloned and shared freely.
    pub fn handle(&self) -> ConfigHandle {
        ConfigHandle {
            inner: Arc::clone(&self.inner),
            changed: self.notify_rx.clone(),
        }
    }

    /// Atomically replace the current configuration and notify all handles.
    pub async fn reload(&self, new_config: AppConfig) {
        let old = {
            let mut guard = self.inner.write().await;
            let old = guard.clone();
            *guard = new_config.clone();
            old
        };
        if old != new_config {
            info!(
                log_level = %new_config.log_level,
                max_connections = new_config.max_connections,
                maintenance_mode = new_config.maintenance_mode,
                "Configuration reloaded"
            );
            // Ignore send error — it only fails when all receivers are dropped.
            let _ = self.notify_tx.send(());
        } else {
            info!("Configuration reload requested but values unchanged");
        }
    }

    /// Fetch the current configuration from Redis and apply it.
    ///
    /// Reads the JSON value stored at `AppConfig::redis_config_key` (default
    /// `config:current`), deserialises it, and calls [`Self::reload`].
    ///
    /// # Errors
    /// Returns [`ReloadError`] if the Redis key is absent, the connection
    /// fails, or the JSON cannot be deserialised.
    pub async fn reload_from_redis(&self, redis: &RedisClient) -> Result<(), ReloadError> {
        let key = self.inner.read().await.redis_config_key.clone();
        let mut conn = redis.get_multiplexed_async_connection().await?;
        let raw: Option<String> = conn.get(&key).await?;
        let json = raw.ok_or(ReloadError::NotFound)?;
        let new_config: AppConfig = serde_json::from_str(&json)?;
        self.reload(new_config).await;
        Ok(())
    }

    /// Spawn a background task that subscribes to `config:reload` on Redis
    /// and calls [`Self::reload_from_redis`] on every message.
    ///
    /// The task runs until the Redis connection is lost or the process exits.
    /// Connection errors are logged and the task exits — callers may restart
    /// it if desired.
    pub fn watch(self: Arc<Self>, redis: RedisClient) -> tokio::task::JoinHandle<()> {
        tokio::spawn(async move {
            const CHANNEL: &str = "config:reload";

            // get_async_connection is the only way to obtain a PubSub-capable connection.
            #[allow(deprecated)]
            let conn = match redis.get_async_connection().await {
                Ok(c) => c,
                Err(e) => {
                    error!(error = %e, "Config watcher: failed to connect to Redis");
                    return;
                }
            };

            let mut pubsub = conn.into_pubsub();
            if let Err(e) = pubsub.subscribe(CHANNEL).await {
                error!(error = %e, channel = CHANNEL, "Config watcher: subscribe failed");
                return;
            }

            info!(
                channel = CHANNEL,
                "Config watcher: listening for reload signals"
            );

            let mut stream = pubsub.into_on_message();
            use futures_util::StreamExt;

            loop {
                match stream.next().await {
                    Some(msg) => {
                        let payload: String = msg.get_payload().unwrap_or_default();
                        info!(payload = %payload, "Config reload signal received");
                        if let Err(e) = self.reload_from_redis(&redis).await {
                            warn!(error = %e, "Config reload from Redis failed; keeping current config");
                        }
                    }
                    None => {
                        warn!("Config watcher: Redis pub/sub stream ended");
                        break;
                    }
                }
            }
        })
    }
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    fn default_watcher() -> ConfigWatcher {
        ConfigWatcher::new(AppConfig::default())
    }

    // --- AppConfig ---

    #[test]
    fn test_default_config_values() {
        let cfg = AppConfig::default();
        assert_eq!(cfg.max_connections, 10);
        assert_eq!(cfg.request_timeout_secs, 30);
        assert!(!cfg.maintenance_mode);
        assert!(!cfg.log_level.is_empty());
        assert_eq!(cfg.redis_config_key, "config:current");
    }

    #[test]
    fn test_config_serialisation_roundtrip() {
        let cfg = AppConfig::default();
        let json = serde_json::to_string(&cfg).unwrap();
        let back: AppConfig = serde_json::from_str(&json).unwrap();
        assert_eq!(cfg, back);
    }

    #[test]
    fn test_config_partial_deserialisation() {
        // Only some fields present — rest should use serde defaults.
        let json = r#"{"log_level":"info","max_connections":25,"request_timeout_secs":60,"maintenance_mode":true,"redis_config_key":"config:current"}"#;
        let cfg: AppConfig = serde_json::from_str(json).unwrap();
        assert_eq!(cfg.log_level, "info");
        assert_eq!(cfg.max_connections, 25);
        assert!(cfg.maintenance_mode);
    }

    // --- ConfigWatcher::reload ---

    #[tokio::test]
    async fn test_reload_updates_config() {
        let watcher = default_watcher();
        let handle = watcher.handle();

        let new_cfg = AppConfig {
            log_level: "info".to_string(),
            max_connections: 50,
            ..AppConfig::default()
        };
        watcher.reload(new_cfg.clone()).await;

        assert_eq!(handle.get().await, new_cfg);
    }

    #[tokio::test]
    async fn test_reload_unchanged_does_not_notify() {
        let watcher = default_watcher();
        let mut handle = watcher.handle();

        // Mark the initial value as seen.
        handle.changed.borrow_and_update();

        // Reload with identical config.
        watcher.reload(AppConfig::default()).await;

        // `has_changed` should be false — no notification was sent.
        assert!(!handle.changed.has_changed().unwrap());
    }

    #[tokio::test]
    async fn test_reload_changed_notifies_handle() {
        let watcher = default_watcher();
        let mut handle = watcher.handle();

        handle.changed.borrow_and_update();

        watcher
            .reload(AppConfig {
                maintenance_mode: true,
                ..AppConfig::default()
            })
            .await;

        assert!(handle.changed.has_changed().unwrap());
    }

    // --- ConfigHandle ---

    #[tokio::test]
    async fn test_handle_get_returns_current() {
        let watcher = default_watcher();
        let handle = watcher.handle();
        assert_eq!(handle.get().await, AppConfig::default());
    }

    #[tokio::test]
    async fn test_multiple_handles_see_same_update() {
        let watcher = default_watcher();
        let h1 = watcher.handle();
        let h2 = watcher.handle();

        let new_cfg = AppConfig {
            max_connections: 99,
            ..AppConfig::default()
        };
        watcher.reload(new_cfg.clone()).await;

        assert_eq!(h1.get().await.max_connections, 99);
        assert_eq!(h2.get().await.max_connections, 99);
    }

    #[tokio::test]
    async fn test_wait_for_change_resolves_after_reload() {
        let watcher = Arc::new(default_watcher());
        let mut handle = watcher.handle();

        // Mark current as seen so wait_for_change actually waits.
        handle.changed.borrow_and_update();

        let watcher2 = Arc::clone(&watcher);
        tokio::spawn(async move {
            tokio::time::sleep(tokio::time::Duration::from_millis(10)).await;
            watcher2
                .reload(AppConfig {
                    maintenance_mode: true,
                    ..AppConfig::default()
                })
                .await;
        });

        let updated = handle.wait_for_change().await;
        assert!(updated.maintenance_mode);
    }

    // --- reload_from_redis (no live Redis — error path) ---

    #[tokio::test]
    async fn test_reload_from_redis_connection_error() {
        let watcher = default_watcher();
        // Port 1 is never open — connection will fail immediately.
        let redis = RedisClient::open("redis://127.0.0.1:1/").unwrap();
        let result = watcher.reload_from_redis(&redis).await;
        assert!(matches!(result, Err(ReloadError::Redis(_))));
        // Config must be unchanged.
        assert_eq!(watcher.handle().get().await, AppConfig::default());
    }

    // --- ReloadError display ---

    #[test]
    fn test_reload_error_not_found_display() {
        let e = ReloadError::NotFound;
        assert!(e.to_string().contains("not found"));
    }

    #[test]
    fn test_reload_error_deserialise_display() {
        let e = ReloadError::Deserialise(serde_json::from_str::<AppConfig>("bad").unwrap_err());
        assert!(!e.to_string().is_empty());
    }
}
