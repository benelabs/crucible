//! Error recovery service.
//!
//! Tracks retry state for failing tasks with configurable max retries.
//! Implements exponential backoff with jitter to prevent thundering herd.

#![allow(dead_code)]

use serde::{Deserialize, Serialize};
use std::sync::Arc;
use thiserror::Error;
use tokio::sync::RwLock;
use tokio::time::{sleep, Duration};
use tracing::{error, info, warn, instrument};
use crate::services::tracing::TracingService;

/// Exponential backoff configuration
#[derive(Debug, Clone, Copy, Serialize, Deserialize)]
pub struct BackoffConfig {
    /// Initial retry delay in milliseconds
    pub initial_delay_ms: u64,
    /// Maximum retry delay in milliseconds
    pub max_delay_ms: u64,
    /// Backoff multiplier (typically 2 for exponential backoff)
    pub multiplier: f64,
    /// Enable jitter (random component) to prevent thundering herd
    pub jitter: bool,
}

impl Default for BackoffConfig {
    fn default() -> Self {
        Self {
            initial_delay_ms: 100,
            max_delay_ms: 30000, // 30 seconds
            multiplier: 2.0,
            jitter: true,
        }
    }
}

impl BackoffConfig {
    /// Calculate delay for a given retry attempt
    pub fn calculate_delay(&self, attempt: u32) -> Duration {
        let mut delay = self.initial_delay_ms as f64
            * self.multiplier.powi(attempt as i32);
        
        // Cap at max_delay_ms
        delay = delay.min(self.max_delay_ms as f64);
        
        // Apply jitter: add random ±10% to prevent thundering herd
        if self.jitter {
            let jitter_factor = 0.9 + (rand::random::<f64>() * 0.2);
            delay = delay * jitter_factor;
        }
        
        Duration::from_millis(delay as u64)
    }
}

#[derive(Error, Debug, Serialize, Deserialize)]
pub enum RecoveryError {
    #[error("Database error: {0}")]
    Database(String),
    #[error("Redis error: {0}")]
    Redis(String),
    #[error("Internal service error: {0}")]
    Internal(String),
    #[error("Max retries reached for task: {0}")]
    MaxRetriesReached(String),
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RecoveryTask {
    pub id: uuid::Uuid,
    pub name: String,
    pub retries: u32,
    pub max_retries: u32,
    /// Time of last retry attempt
    pub last_attempt: i64,
    /// Backoff configuration for this task
    pub backoff_config: Option<BackoffConfig>,
}

pub struct ErrorManager {
    tasks: Arc<RwLock<Vec<RecoveryTask>>>,
    backoff_config: BackoffConfig,
}

impl Default for ErrorManager {
    fn default() -> Self {
        Self::new()
    }
}

impl ErrorManager {
    pub fn new() -> Self {
        Self::with_backoff(BackoffConfig::default())
    }

    pub fn with_backoff(backoff_config: BackoffConfig) -> Self {
        Self {
            tasks: Arc::new(RwLock::new(Vec::new())),
            backoff_config,
        }
    }

    #[instrument(skip(self), fields(service.name = "ErrorManager", service.method = "handle_error"))]
    pub async fn handle_error(&self, error: RecoveryError, task_name: &str) -> Result<(), RecoveryError> {
        let span = TracingService::service_method_span("ErrorManager", "handle_error");
        let _enter = span.enter();

        warn!(task = %task_name, error = %error, "Handling error");

        let mut tasks = self.tasks.write().await;
        if let Some(task) = tasks.iter_mut().find(|t| t.name == task_name) {
            task.retries += 1;
            task.last_attempt = chrono::Utc::now().timestamp_millis();
            
            if task.retries > task.max_retries {
                error!(task = %task_name, "Max retries reached");
                TracingService::record_error(
                    &span,
                    &format!("Max retries reached for {}", task_name),
                    "max_retries",
                );
                return Err(RecoveryError::MaxRetriesReached(task_name.to_string()));
            }
            
            // Calculate exponential backoff with jitter
            let backoff_config = task.backoff_config.unwrap_or(self.backoff_config);
            let delay = backoff_config.calculate_delay(task.retries - 1);
            
            info!(task = %task_name, retry = task.retries, delay_ms = delay.as_millis(), "Retrying task with backoff");
            
            // Drop the write lock before sleeping
            drop(tasks);
            
            // Wait before retrying
            sleep(delay).await;
        } else {
            tasks.push(RecoveryTask {
                id: uuid::Uuid::new_v4(),
                name: task_name.to_string(),
                retries: 1,
                max_retries: 3,
                last_attempt: chrono::Utc::now().timestamp_millis(),
                backoff_config: Some(self.backoff_config),
            });
            info!(task = %task_name, "Registered new recovery task");
        }

        Ok(())
    }

    #[instrument(skip(self), fields(service.name = "ErrorManager", service.method = "get_active_tasks"))]
    pub async fn get_active_tasks(&self) -> Vec<RecoveryTask> {
        let span = TracingService::service_method_span("ErrorManager", "get_active_tasks");
        let _enter = span.enter();
        self.tasks.read().await.clone()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn test_error_recovery_workflow() {
        let manager = ErrorManager::new();
        let task_name = "test_task";

        manager
            .handle_error(RecoveryError::Database("connection lost".to_string()), task_name)
            .await
            .unwrap();
        assert_eq!(manager.get_active_tasks().await.len(), 1);
        assert_eq!(manager.get_active_tasks().await[0].retries, 1);

        manager
            .handle_error(RecoveryError::Redis("timeout".to_string()), task_name)
            .await
            .unwrap();
        assert_eq!(manager.get_active_tasks().await[0].retries, 2);

        manager
            .handle_error(RecoveryError::Internal("unknown".to_string()), task_name)
            .await
            .unwrap();
        assert_eq!(manager.get_active_tasks().await[0].retries, 3);

        let result = manager
            .handle_error(RecoveryError::Internal("last straw".to_string()), task_name)
            .await;
        assert!(matches!(result, Err(RecoveryError::MaxRetriesReached(_))));
    }

    #[test]
    fn test_backoff_config_exponential_delay() {
        let config = BackoffConfig {
            initial_delay_ms: 100,
            max_delay_ms: 30000,
            multiplier: 2.0,
            jitter: false,
        };

        let delay_0 = config.calculate_delay(0);
        let delay_1 = config.calculate_delay(1);
        let delay_2 = config.calculate_delay(2);

        assert_eq!(delay_0.as_millis(), 100);
        assert_eq!(delay_1.as_millis(), 200);
        assert_eq!(delay_2.as_millis(), 400);
    }

    #[test]
    fn test_backoff_config_capped_at_max() {
        let config = BackoffConfig {
            initial_delay_ms: 100,
            max_delay_ms: 1000,
            multiplier: 2.0,
            jitter: false,
        };

        let delay_high = config.calculate_delay(10);
        assert!(delay_high.as_millis() <= 1000);
    }

    #[test]
    fn test_backoff_with_jitter() {
        let config = BackoffConfig {
            initial_delay_ms: 100,
            max_delay_ms: 30000,
            multiplier: 2.0,
            jitter: true,
        };

        // Jitter adds variance, so we just check it's within bounds
        let delay = config.calculate_delay(1);
        let expected_center = 200;
        let jitter_variance = (expected_center as f64 * 0.2) as u128; // ±10%

        assert!(delay.as_millis() >= expected_center - jitter_variance);
        assert!(delay.as_millis() <= expected_center + jitter_variance);
    }

    #[tokio::test]
    async fn test_error_manager_with_custom_backoff() {
        let custom_config = BackoffConfig {
            initial_delay_ms: 50,
            max_delay_ms: 5000,
            multiplier: 1.5,
            jitter: false,
        };

        let manager = ErrorManager::with_backoff(custom_config);
        let task_name = "custom_task";

        manager
            .handle_error(RecoveryError::Database("error".to_string()), task_name)
            .await
            .unwrap();

        let tasks = manager.get_active_tasks().await;
        assert_eq!(tasks.len(), 1);
        assert_eq!(tasks[0].backoff_config.unwrap().initial_delay_ms, 50);
    }
}
