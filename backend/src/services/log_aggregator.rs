#![allow(dead_code)]
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Arc;
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use thiserror::Error;
use tokio::sync::mpsc;
use tracing::{debug, info, warn};

/// Default capacity for log aggregator channel (increased from 100 to 10,000 to handle burst traffic).
pub const DEFAULT_LOG_CHANNEL_CAPACITY: usize = 10_000;

/// Errors produced when a log entry is dropped due to channel backpressure.
#[derive(Debug, Error, Clone, PartialEq, Eq)]
pub enum LogDropError {
    #[error("Log aggregator channel buffer is full (total dropped: {dropped_total})")]
    BufferFull { dropped_total: u64 },
    #[error("Log aggregator channel receiver is closed")]
    ChannelClosed,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct LogEntry {
    pub timestamp: DateTime<Utc>,
    pub level: String,
    pub message: String,
    pub service: String,
}

#[derive(Clone)]
pub struct LogAggregator {
    sender: mpsc::Sender<LogEntry>,
    dropped_logs_total: Arc<AtomicU64>,
}

impl LogAggregator {
    /// Create a new LogAggregator with the default high-capacity buffer (10,000).
    pub fn new() -> (Self, mpsc::Receiver<LogEntry>) {
        Self::with_capacity(DEFAULT_LOG_CHANNEL_CAPACITY)
    }

    /// Create a new LogAggregator with a custom channel capacity.
    pub fn with_capacity(capacity: usize) -> (Self, mpsc::Receiver<LogEntry>) {
        let (tx, rx) = mpsc::channel(capacity);
        (
            Self {
                sender: tx,
                dropped_logs_total: Arc::new(AtomicU64::new(0)),
            },
            rx,
        )
    }

    /// Return the total number of log entries dropped due to buffer backpressure.
    pub fn dropped_logs_total(&self) -> u64 {
        self.dropped_logs_total.load(Ordering::Relaxed)
    }

    /// Reset the dropped log counter (useful for test assertions).
    pub fn reset_dropped_logs_counter(&self) {
        self.dropped_logs_total.store(0, Ordering::Relaxed);
    }

    /// Attempt non-blocking log ingestion using try_send.
    /// When backpressure occurs, increments the atomic dropped_logs_total counter
    /// and logs a warning instead of silently discarding the log entry.
    pub fn try_log(&self, level: &str, message: &str, service: &str) -> Result<(), LogDropError> {
        let entry = LogEntry {
            timestamp: Utc::now(),
            level: level.to_string(),
            message: message.to_string(),
            service: service.to_string(),
        };

        match self.sender.try_send(entry) {
            Ok(()) => Ok(()),
            Err(mpsc::error::TrySendError::Full(dropped)) => {
                let count = self.dropped_logs_total.fetch_add(1, Ordering::Relaxed) + 1;
                warn!(
                    dropped_logs_total = count,
                    level = %dropped.level,
                    service = %dropped.service,
                    "Log aggregator buffer is full; dropped log entry under burst traffic backpressure"
                );
                Err(LogDropError::BufferFull {
                    dropped_total: count,
                })
            }
            Err(mpsc::error::TrySendError::Closed(_)) => Err(LogDropError::ChannelClosed),
        }
    }

    /// Asynchronously aggregate a log entry, incrementing dropped counter if receiver dropped.
    pub async fn log(&self, level: &str, message: &str, service: &str) -> anyhow::Result<()> {
        let entry = LogEntry {
            timestamp: Utc::now(),
            level: level.to_string(),
            message: message.to_string(),
            service: service.to_string(),
        };

        debug!(entry = ?entry, "Aggregating log entry");
        if let Err(e) = self.sender.send(entry).await {
            self.dropped_logs_total.fetch_add(1, Ordering::Relaxed);
            return Err(e.into());
        }
        Ok(())
    }

    pub async fn run_worker(mut receiver: mpsc::Receiver<LogEntry>) {
        info!("Starting log aggregator worker");
        while let Some(entry) = receiver.recv().await {
            info!(
                target: "log_aggregator",
                timestamp = %entry.timestamp,
                level = %entry.level,
                service = %entry.service,
                message = %entry.message,
                "Persisted log entry"
            );
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn test_log_aggregation() {
        let (aggregator, mut receiver) = LogAggregator::new();

        aggregator
            .log("INFO", "Test message", "test_service")
            .await
            .unwrap();

        let entry = receiver.recv().await.unwrap();
        assert_eq!(entry.level, "INFO");
        assert_eq!(entry.message, "Test message");
        assert_eq!(entry.service, "test_service");
        assert_eq!(aggregator.dropped_logs_total(), 0);
    }

    #[test]
    fn test_try_log_increments_dropped_logs_counter_on_backpressure() {
        // Create aggregator with small buffer capacity of 2
        let (aggregator, _receiver) = LogAggregator::with_capacity(2);

        // Fill buffer
        assert!(aggregator.try_log("INFO", "msg 1", "svc").is_ok());
        assert!(aggregator.try_log("WARN", "msg 2", "svc").is_ok());
        assert_eq!(aggregator.dropped_logs_total(), 0);

        // 3rd and 4th entries exceed buffer capacity and trigger backpressure
        let res1 = aggregator.try_log("WARN", "burst warning 1", "svc");
        assert_eq!(
            res1,
            Err(LogDropError::BufferFull { dropped_total: 1 })
        );
        assert_eq!(aggregator.dropped_logs_total(), 1);

        let res2 = aggregator.try_log("ERROR", "burst error 2", "svc");
        assert_eq!(
            res2,
            Err(LogDropError::BufferFull { dropped_total: 2 })
        );
        assert_eq!(aggregator.dropped_logs_total(), 2);
    }

    #[test]
    fn test_channel_capacity_is_increased() {
        let (aggregator, _receiver) = LogAggregator::new();
        // Should comfortably absorb bursts without dropping
        for i in 0..1000 {
            assert!(aggregator.try_log("INFO", &format!("burst {i}"), "svc").is_ok());
        }
        assert_eq!(aggregator.dropped_logs_total(), 0);
    }
}
