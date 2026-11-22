#![allow(dead_code)]

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Arc;
use thiserror::Error;
use tokio::sync::mpsc;
use tracing::{debug, error, info, warn};

#[derive(Debug, Error, PartialEq, Eq)]
pub enum LogAggregatorError {
    #[error("Log aggregator buffer is full due to backpressure")]
    BufferFull,
    #[error("Log aggregator channel is closed")]
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
    /// Default channel buffer capacity increased from 100 to 10,000 to prevent message drops under burst load.
    pub const DEFAULT_CHANNEL_CAPACITY: usize = 10_000;

    /// Create a new `LogAggregator` and its receiver with default channel capacity (10,000).
    pub fn new() -> (Self, mpsc::Receiver<LogEntry>) {
        Self::with_capacity(Self::DEFAULT_CHANNEL_CAPACITY)
    }

    /// Create a new `LogAggregator` and its receiver with custom buffer capacity.
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

    #[inline]
    fn record_dropped(&self) -> u64 {
        let count = self.dropped_logs_total.fetch_add(1, Ordering::Relaxed) + 1;
        crate::services::metrics::inc_dropped_logs_metric();
        count
    }

    /// Return the total number of log entries dropped due to buffer backpressure or closed channel.
    pub fn dropped_logs_total(&self) -> u64 {
        self.dropped_logs_total.load(Ordering::Relaxed)
    }

    /// Reset dropped logs counter and return previous value.
    pub fn reset_dropped_logs(&self) -> u64 {
        self.dropped_logs_total.swap(0, Ordering::Relaxed)
    }

    /// Non-blocking attempt to send a log entry.
    ///
    /// If the channel buffer is full, increments the `dropped_logs_total` counter and returns `Err(LogAggregatorError::BufferFull)`.
    pub fn try_log(
        &self,
        level: &str,
        message: &str,
        service: &str,
    ) -> Result<(), LogAggregatorError> {
        let entry = LogEntry {
            timestamp: Utc::now(),
            level: level.to_string(),
            message: message.to_string(),
            service: service.to_string(),
        };

        match self.sender.try_send(entry) {
            Ok(()) => Ok(()),
            Err(mpsc::error::TrySendError::Full(_)) => {
                let dropped = self.record_dropped();
                warn!(
                    dropped_logs_total = dropped,
                    level = %level,
                    service = %service,
                    "Log aggregator channel buffer full, log entry dropped"
                );
                Err(LogAggregatorError::BufferFull)
            }
            Err(mpsc::error::TrySendError::Closed(_)) => {
                let dropped = self.record_dropped();
                error!(
                    dropped_logs_total = dropped,
                    "Log aggregator channel closed, log entry dropped"
                );
                Err(LogAggregatorError::ChannelClosed)
            }
        }
    }

    /// Asynchronously submit a log entry, waiting if capacity is temporarily unavailable.
    ///
    /// If the channel is disconnected, increments `dropped_logs_total` and returns an error.
    pub async fn log(&self, level: &str, message: &str, service: &str) -> anyhow::Result<()> {
        let entry = LogEntry {
            timestamp: Utc::now(),
            level: level.to_string(),
            message: message.to_string(),
            service: service.to_string(),
        };

        debug!(entry = ?entry, "Aggregating log entry");
        match self.sender.try_send(entry) {
            Ok(()) => Ok(()),
            Err(mpsc::error::TrySendError::Full(entry)) => {
                // Channel buffer full under high burst, wait for worker to consume
                if let Err(e) = self.sender.send(entry).await {
                    let dropped = self.record_dropped();
                    error!(
                        dropped_logs_total = dropped,
                        error = %e,
                        "Log aggregator channel disconnected while awaiting send capacity"
                    );
                    return Err(e.into());
                }
                Ok(())
            }
            Err(mpsc::error::TrySendError::Closed(_)) => {
                let dropped = self.record_dropped();
                error!(
                    dropped_logs_total = dropped,
                    "Log aggregator channel closed"
                );
                anyhow::bail!("log aggregator channel closed")
            }
        }
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
    }

    #[tokio::test]
    async fn test_channel_capacity_is_increased() {
        let (aggregator, _receiver) = LogAggregator::new();
        // Should be able to buffer at least 1,000 items without blocking or dropping
        for i in 0..1000 {
            assert!(aggregator
                .try_log("WARN", &format!("burst log {i}"), "api")
                .is_ok());
        }
        assert_eq!(aggregator.dropped_logs_total(), 0);
    }

    #[tokio::test]
    async fn test_try_log_records_dropped_counter_on_buffer_full() {
        let (aggregator, _receiver) = LogAggregator::with_capacity(2);

        assert!(aggregator.try_log("INFO", "log 1", "test").is_ok());
        assert!(aggregator.try_log("INFO", "log 2", "test").is_ok());

        let initial_metric = crate::services::metrics::dropped_logs_counter().get();
        let res = aggregator.try_log("WARN", "dropped log 3", "test");
        assert_eq!(res, Err(LogAggregatorError::BufferFull));
        assert_eq!(aggregator.dropped_logs_total(), 1);
        assert!(crate::services::metrics::dropped_logs_counter().get() >= initial_metric + 1.0);

        let res4 = aggregator.try_log("ERROR", "dropped log 4", "test");
        assert_eq!(res4, Err(LogAggregatorError::BufferFull));
        assert_eq!(aggregator.dropped_logs_total(), 2);
        assert!(crate::services::metrics::dropped_logs_counter().get() >= initial_metric + 2.0);
    }

    #[tokio::test]
    async fn test_try_log_records_dropped_counter_on_channel_closed() {
        let (aggregator, receiver) = LogAggregator::with_capacity(2);
        drop(receiver); // Close the channel

        let initial_drops = aggregator.dropped_logs_total();
        let res = aggregator.try_log("ERROR", "closed channel log", "test");
        assert_eq!(res, Err(LogAggregatorError::ChannelClosed));
        assert_eq!(aggregator.dropped_logs_total(), initial_drops + 1);
    }

    #[tokio::test]
    async fn test_log_await_records_dropped_if_channel_closed() {
        let (aggregator, receiver) = LogAggregator::with_capacity(1);
        drop(receiver);

        let initial_drops = aggregator.dropped_logs_total();
        let res = aggregator.log("WARN", "fail to send", "test").await;
        assert!(res.is_err());
        assert_eq!(aggregator.dropped_logs_total(), initial_drops + 1);
    }
}
