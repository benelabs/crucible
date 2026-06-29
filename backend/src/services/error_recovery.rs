//! Error recovery service.
//!
//! Tracks retry state for failing tasks with configurable max retries.

#![allow(dead_code)]

use serde::{Deserialize, Serialize};
use std::sync::Arc;
use thiserror::Error;
use tokio::sync::RwLock;
use tracing::{error, info, warn, instrument};
use crate::services::tracing::TracingService;

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
}

pub struct ErrorManager {
    tasks: Arc<RwLock<Vec<RecoveryTask>>>,
}

impl Default for ErrorManager {
    fn default() -> Self {
        Self::new()
    }
}

impl ErrorManager {
    pub fn new() -> Self {
        Self {
            tasks: Arc::new(RwLock::new(Vec::new())),
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
            if task.retries > task.max_retries {
                error!(task = %task_name, "Max retries reached");
                TracingService::record_error(
                    &span,
                    &format!("Max retries reached for {}", task_name),
                    "max_retries",
                );
                return Err(RecoveryError::MaxRetriesReached(task_name.to_string()));
            }
            info!(task = %task_name, retry = task.retries, "Retrying task");
        } else {
            tasks.push(RecoveryTask {
                id: uuid::Uuid::new_v4(),
                name: task_name.to_string(),
                retries: 1,
                max_retries: 3,
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
}
