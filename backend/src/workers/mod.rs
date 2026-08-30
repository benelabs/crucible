//! Background worker modules for the Crucible backend.
//!
//! This module groups all async worker implementations including retry logic,
//! job processing, and other background task utilities.

pub mod cache_warm;
pub mod dlq;
pub mod error;
pub mod executor;
pub mod health;
pub mod ingestion;
pub mod job_history;
pub mod priority;
pub mod progress;
pub mod retry;
pub mod scheduler;

pub use cache_warm::CacheWarmWorker;
pub use dlq::{DeadLetterJob, DeadLetterQueue};
pub use executor::TaskExecutor;
pub use health::WorkerHealthMonitor;
pub use progress::JobProgressTracker;
pub use scheduler::{Scheduler, SchedulerHandle, JobContext, JobHandler, JobDefinition};

use std::sync::Arc;
use std::time::Duration;
use tokio::sync::watch;
use tracing::{info, warn};

/// Graceful shutdown coordinator for background workers.
#[derive(Clone)]
pub struct ShutdownCoordinator {
    shutdown_tx: Arc<watch::Sender<bool>>,
    active_tasks: Arc<tokio::sync::Mutex<Vec<tokio::task::JoinHandle<()>>>>,
}

impl ShutdownCoordinator {
    pub fn new() -> Self {
        let (tx, _) = watch::channel(false);
        Self {
            shutdown_tx: Arc::new(tx),
            active_tasks: Arc::new(tokio::sync::Mutex::new(Vec::new())),
        }
    }

    /// Returns a receiver to monitor shutdown signal.
    pub fn subscribe(&self) -> watch::Receiver<bool> {
        self.shutdown_tx.subscribe()
    }

    /// Register a handle of a running background task.
    pub async fn register(&self, handle: tokio::task::JoinHandle<()>) {
        self.active_tasks.lock().await.push(handle);
    }

    /// Triggers shutdown and waits for registered tasks to complete within the grace period.
    pub async fn shutdown_with_grace(&self, grace_period: Duration) {
        info!("Graceful shutdown initiated. Signaling workers...");
        let _ = self.shutdown_tx.send(true);

        let mut tasks = self.active_tasks.lock().await;
        let futures = tasks.drain(..).map(|handle| async move {
            let _ = handle.await;
        });

        info!("Waiting for active jobs to complete (grace period: {:?})...", grace_period);
        let wait_all = futures_util::future::join_all(futures);

        if let Err(_) = tokio::time::timeout(grace_period, wait_all).await {
            warn!("Grace period exceeded. Some tasks did not finish in time.");
        } else {
            info!("All registered tasks completed successfully.");
        }
    }
}

/// Helper function to wait for standard termination signals (SIGINT / SIGTERM).
#[cfg(unix)]
pub async fn wait_for_signals() -> std::io::Result<()> {
    use tokio::signal::unix::{signal, SignalKind};
    let mut sigint = signal(SignalKind::interrupt())?;
    let mut sigterm = signal(SignalKind::terminate())?;

    tokio::select! {
        _ = sigint.recv() => {
            info!("Received SIGINT signal");
        }
        _ = sigterm.recv() => {
            info!("Received SIGTERM signal");
        }
    }
    Ok(())
}

/// Helper function to wait for Ctrl+C on non-Unix platforms.
#[cfg(not(unix))]
pub async fn wait_for_signals() -> std::io::Result<()> {
    tokio::signal::ctrl_c().await?;
    info!("Received Ctrl+C signal");
    Ok(())
}

#[cfg(test)]
mod shutdown_tests {
    use super::*;
    use std::sync::atomic::{AtomicBool, Ordering};
    use tokio::time::sleep;

    #[tokio::test]
    async fn test_shutdown_coordinator_waits_for_completion() {
        let coordinator = ShutdownCoordinator::new();
        let job_completed = Arc::new(AtomicBool::new(false));
        let job_completed_clone = job_completed.clone();
        let mut rx = coordinator.subscribe();

        let handle = tokio::spawn(async move {
            loop {
                if *rx.borrow() {
                    sleep(Duration::from_millis(50)).await;
                    job_completed_clone.store(true, Ordering::SeqCst);
                    break;
                }
                sleep(Duration::from_millis(10)).await;
            }
        });

        coordinator.register(handle).await;
        coordinator.shutdown_with_grace(Duration::from_millis(200)).await;

        assert!(job_completed.load(Ordering::SeqCst));
    }
}
