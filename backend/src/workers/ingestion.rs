//! High-throughput Soroban RPC ledger event ingestion with backpressure.
//!
//! Streams transaction changes from Soroban RPC through a bounded Tokio
//! channel, applies rate-limited RPC backoff under pressure, and persists a
//! ledger-sequence cursor in PostgreSQL (or an in-memory store for tests) so
//! workers can resume after crashes.
//!
//! Location: backend/src/workers/ingestion.rs
//! Production requirement: High-Throughput Soroban RPC Ingestion Pipeline with Backpressure

use std::collections::VecDeque;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use serde::{Deserialize, Serialize};
use thiserror::Error;
use tokio::sync::mpsc;
use tracing::{debug, info, warn};

/// Default bounded channel capacity (ledgers waiting to be processed).
pub const DEFAULT_BUFFER_CAPACITY: usize = 256;
/// Target sustained ingestion rate for load tests (ledgers / minute).
pub const TARGET_LEDGERS_PER_MINUTE: u64 = 500;

/// Errors raised by the ingestion pipeline.
#[derive(Debug, Error, Clone, PartialEq, Eq)]
pub enum IngestionError {
    #[error("rpc temporarily unavailable: {0}")]
    RpcUnavailable(String),
    #[error("cursor persistence failed: {0}")]
    CursorStore(String),
    #[error("pipeline shut down")]
    ShutDown,
    #[error("backpressure timeout waiting for buffer space")]
    BackpressureTimeout,
    #[error("invalid configuration: {0}")]
    InvalidConfig(String),
}

/// Pipeline tuning knobs.
#[derive(Debug, Clone)]
pub struct IngestionConfig {
    /// Bounded channel size — producers block (backpressure) when full.
    pub buffer_capacity: usize,
    /// Maximum ledgers accepted per minute (token-bucket style).
    pub max_ledgers_per_minute: u64,
    /// Initial RPC backoff after a failure.
    pub initial_backoff: Duration,
    /// Cap on exponential RPC backoff.
    pub max_backoff: Duration,
    /// How long a producer waits for buffer space before timing out.
    pub enqueue_timeout: Duration,
}

impl Default for IngestionConfig {
    fn default() -> Self {
        Self {
            buffer_capacity: DEFAULT_BUFFER_CAPACITY,
            max_ledgers_per_minute: TARGET_LEDGERS_PER_MINUTE,
            initial_backoff: Duration::from_millis(50),
            max_backoff: Duration::from_secs(5),
            enqueue_timeout: Duration::from_secs(2),
        }
    }
}

impl IngestionConfig {
    fn validate(&self) -> Result<(), IngestionError> {
        if self.buffer_capacity == 0 {
            return Err(IngestionError::InvalidConfig(
                "buffer_capacity must be > 0".into(),
            ));
        }
        if self.max_ledgers_per_minute == 0 {
            return Err(IngestionError::InvalidConfig(
                "max_ledgers_per_minute must be > 0".into(),
            ));
        }
        Ok(())
    }
}

/// A single ledger batch pulled from Soroban RPC.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct LedgerBatch {
    pub sequence: u64,
    pub tx_count: u32,
    pub closed_at: u64,
}

/// Persists the last successfully processed ledger sequence.
#[async_trait::async_trait]
pub trait LedgerCursorStore: Send + Sync {
    async fn load_cursor(&self) -> Result<Option<u64>, IngestionError>;
    async fn save_cursor(&self, sequence: u64) -> Result<(), IngestionError>;
}

/// In-memory cursor store used by unit / load tests.
#[derive(Debug, Default)]
pub struct InMemoryCursorStore {
    cursor: Mutex<Option<u64>>,
}

impl InMemoryCursorStore {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn current(&self) -> Option<u64> {
        *self.cursor.lock().expect("lock")
    }
}

#[async_trait::async_trait]
impl LedgerCursorStore for InMemoryCursorStore {
    async fn load_cursor(&self) -> Result<Option<u64>, IngestionError> {
        Ok(*self.cursor.lock().expect("lock"))
    }

    async fn save_cursor(&self, sequence: u64) -> Result<(), IngestionError> {
        *self.cursor.lock().expect("lock") = Some(sequence);
        Ok(())
    }
}

/// PostgreSQL-backed cursor store (crash-resilient resumption).
pub struct PostgresCursorStore {
    pool: sqlx::PgPool,
    worker_id: String,
}

impl PostgresCursorStore {
    pub fn new(pool: sqlx::PgPool, worker_id: impl Into<String>) -> Self {
        Self {
            pool,
            worker_id: worker_id.into(),
        }
    }
}

#[async_trait::async_trait]
impl LedgerCursorStore for PostgresCursorStore {
    async fn load_cursor(&self) -> Result<Option<u64>, IngestionError> {
        let row: Option<(i64,)> = sqlx::query_as(
            r#"
            SELECT last_ledger
            FROM ingestion_cursors
            WHERE worker_id = $1
            "#,
        )
        .bind(&self.worker_id)
        .fetch_optional(&self.pool)
        .await
        .map_err(|e| IngestionError::CursorStore(e.to_string()))?;

        Ok(row.map(|(seq,)| seq as u64))
    }

    async fn save_cursor(&self, sequence: u64) -> Result<(), IngestionError> {
        sqlx::query(
            r#"
            INSERT INTO ingestion_cursors (worker_id, last_ledger, updated_at)
            VALUES ($1, $2, NOW())
            ON CONFLICT (worker_id) DO UPDATE
            SET last_ledger = EXCLUDED.last_ledger,
                updated_at = NOW()
            "#,
        )
        .bind(&self.worker_id)
        .bind(sequence as i64)
        .execute(&self.pool)
        .await
        .map_err(|e| IngestionError::CursorStore(e.to_string()))?;
        Ok(())
    }
}

/// Source of ledger batches (RPC client abstraction).
#[async_trait::async_trait]
pub trait LedgerSource: Send + Sync {
    async fn fetch_from(&self, after_sequence: u64) -> Result<Vec<LedgerBatch>, IngestionError>;
}

/// Rate limiter using a sliding one-minute window.
#[derive(Debug)]
struct RateLimiter {
    max_per_minute: u64,
    stamps: Mutex<VecDeque<Instant>>,
}

impl RateLimiter {
    fn new(max_per_minute: u64) -> Self {
        Self {
            max_per_minute,
            stamps: Mutex::new(VecDeque::new()),
        }
    }

    async fn acquire(&self) {
        loop {
            let wait = {
                let mut stamps = self.stamps.lock().expect("lock");
                let cutoff = Instant::now() - Duration::from_secs(60);
                while stamps.front().is_some_and(|t| *t < cutoff) {
                    stamps.pop_front();
                }
                if (stamps.len() as u64) < self.max_per_minute {
                    stamps.push_back(Instant::now());
                    None
                } else {
                    stamps
                        .front()
                        .map(|t| (*t + Duration::from_secs(60)).saturating_duration_since(Instant::now()))
                }
            };
            match wait {
                None => return,
                Some(d) => tokio::time::sleep(d.max(Duration::from_millis(1))).await,
            }
        }
    }
}

/// Metrics snapshot for observability / load tests.
#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq, Eq)]
pub struct IngestionMetrics {
    pub ingested: u64,
    pub rpc_failures: u64,
    pub backpressure_waits: u64,
    pub last_sequence: u64,
}

/// Multi-worker ingestion pipeline with bounded buffers and cursor persistence.
pub struct IngestionPipeline {
    config: IngestionConfig,
    source: Arc<dyn LedgerSource>,
    cursor: Arc<dyn LedgerCursorStore>,
    tx: mpsc::Sender<LedgerBatch>,
    rx: Mutex<Option<mpsc::Receiver<LedgerBatch>>>,
    ingested: AtomicU64,
    rpc_failures: AtomicU64,
    backpressure_waits: AtomicU64,
    last_sequence: AtomicU64,
    rate: RateLimiter,
}

impl IngestionPipeline {
    pub fn new(
        config: IngestionConfig,
        source: Arc<dyn LedgerSource>,
        cursor: Arc<dyn LedgerCursorStore>,
    ) -> Result<Self, IngestionError> {
        config.validate()?;
        let (tx, rx) = mpsc::channel(config.buffer_capacity);
        let rate = RateLimiter::new(config.max_ledgers_per_minute);
        Ok(Self {
            config,
            source,
            cursor,
            tx,
            rx: Mutex::new(Some(rx)),
            ingested: AtomicU64::new(0),
            rpc_failures: AtomicU64::new(0),
            backpressure_waits: AtomicU64::new(0),
            last_sequence: AtomicU64::new(0),
            rate,
        })
    }

    pub fn metrics(&self) -> IngestionMetrics {
        IngestionMetrics {
            ingested: self.ingested.load(Ordering::SeqCst),
            rpc_failures: self.rpc_failures.load(Ordering::SeqCst),
            backpressure_waits: self.backpressure_waits.load(Ordering::SeqCst),
            last_sequence: self.last_sequence.load(Ordering::SeqCst),
        }
    }

    /// Pull ledgers from RPC (with backoff) and enqueue under backpressure.
    pub async fn produce_once(&self) -> Result<usize, IngestionError> {
        let after = self
            .cursor
            .load_cursor()
            .await?
            .unwrap_or(0)
            .max(self.last_sequence.load(Ordering::SeqCst));

        let mut backoff = self.config.initial_backoff;
        let batches = loop {
            match self.source.fetch_from(after).await {
                Ok(batches) => break batches,
                Err(e) => {
                    self.rpc_failures.fetch_add(1, Ordering::SeqCst);
                    warn!(error = %e, ?backoff, "rpc fetch failed; backing off");
                    tokio::time::sleep(backoff).await;
                    backoff = (backoff * 2).min(self.config.max_backoff);
                }
            }
        };

        let mut enqueued = 0usize;
        for batch in batches {
            self.rate.acquire().await;
            match self.tx.try_send(batch.clone()) {
                Ok(()) => enqueued += 1,
                Err(mpsc::error::TrySendError::Full(batch)) => {
                    self.backpressure_waits.fetch_add(1, Ordering::SeqCst);
                    debug!(seq = batch.sequence, "buffer full; applying backpressure");
                    tokio::time::timeout(self.config.enqueue_timeout, self.tx.send(batch))
                        .await
                        .map_err(|_| IngestionError::BackpressureTimeout)?
                        .map_err(|_| IngestionError::ShutDown)?;
                    enqueued += 1;
                }
                Err(mpsc::error::TrySendError::Closed(_)) => {
                    return Err(IngestionError::ShutDown);
                }
            }
        }
        Ok(enqueued)
    }

    /// Drain the bounded buffer, persist cursor after each ledger.
    pub async fn consume_available(&self) -> Result<usize, IngestionError> {
        let mut rx_guard = self.rx.lock().expect("lock");
        let rx = rx_guard
            .as_mut()
            .ok_or(IngestionError::ShutDown)?;

        let mut processed = 0usize;
        while let Ok(batch) = rx.try_recv() {
            self.cursor.save_cursor(batch.sequence).await?;
            self.last_sequence
                .store(batch.sequence, Ordering::SeqCst);
            self.ingested.fetch_add(1, Ordering::SeqCst);
            processed += 1;
        }
        Ok(processed)
    }

    /// Run produce+consume until `ledger_count` ledgers are ingested (load tests).
    pub async fn run_until(&self, ledger_count: u64) -> Result<IngestionMetrics, IngestionError> {
        while self.ingested.load(Ordering::SeqCst) < ledger_count {
            let _ = self.produce_once().await?;
            let _ = self.consume_available().await?;
            if self.ingested.load(Ordering::SeqCst) < ledger_count {
                tokio::task::yield_now().await;
            }
        }
        let metrics = self.metrics();
        info!(
            ingested = metrics.ingested,
            last = metrics.last_sequence,
            bp = metrics.backpressure_waits,
            "ingestion target reached"
        );
        Ok(metrics)
    }
}

/// Synthetic RPC source that emits contiguous ledgers for load testing.
pub struct SyntheticLedgerSource {
    next: AtomicU64,
    batch_size: u32,
    /// Remaining forced failures before fetches succeed (for backoff tests).
    failures_remaining: AtomicU64,
}

impl SyntheticLedgerSource {
    pub fn new(start_after: u64, batch_size: u32) -> Self {
        Self {
            next: AtomicU64::new(start_after + 1),
            batch_size,
            failures_remaining: AtomicU64::new(0),
        }
    }

    /// Fail the next `n` RPC calls, then succeed (exercises backoff).
    pub fn with_transient_failures(self, n: u64) -> Self {
        self.failures_remaining.store(n, Ordering::SeqCst);
        self
    }
}

#[async_trait::async_trait]
impl LedgerSource for SyntheticLedgerSource {
    async fn fetch_from(&self, after_sequence: u64) -> Result<Vec<LedgerBatch>, IngestionError> {
        let prev = self.failures_remaining.load(Ordering::SeqCst);
        if prev > 0
            && self
                .failures_remaining
                .compare_exchange(prev, prev - 1, Ordering::SeqCst, Ordering::SeqCst)
                .is_ok()
        {
            return Err(IngestionError::RpcUnavailable("simulated blip".into()));
        }

        let start = after_sequence
            .saturating_add(1)
            .max(self.next.load(Ordering::SeqCst));
        let mut out = Vec::with_capacity(self.batch_size as usize);
        for i in 0..self.batch_size {
            let seq = start + u64::from(i);
            out.push(LedgerBatch {
                sequence: seq,
                tx_count: 1 + (seq % 7) as u32,
                closed_at: 1_700_000_000 + seq,
            });
        }
        if let Some(last) = out.last() {
            self.next.store(last.sequence + 1, Ordering::SeqCst);
        }
        Ok(out)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn cursor_persists_across_pipeline_restarts() {
        let cursor = Arc::new(InMemoryCursorStore::new());
        let source: Arc<dyn LedgerSource> = Arc::new(SyntheticLedgerSource::new(0, 5));
        let pipeline = IngestionPipeline::new(
            IngestionConfig {
                buffer_capacity: 32,
                max_ledgers_per_minute: 10_000,
                ..IngestionConfig::default()
            },
            source.clone(),
            cursor.clone(),
        )
        .unwrap();

        pipeline.produce_once().await.unwrap();
        pipeline.consume_available().await.unwrap();
        let first = cursor.current().unwrap();
        assert!(first >= 5);

        // New pipeline resumes from persisted cursor.
        let pipeline2 = IngestionPipeline::new(
            IngestionConfig {
                buffer_capacity: 32,
                max_ledgers_per_minute: 10_000,
                ..IngestionConfig::default()
            },
            source,
            cursor.clone(),
        )
        .unwrap();
        pipeline2.produce_once().await.unwrap();
        pipeline2.consume_available().await.unwrap();
        let second = cursor.current().unwrap();
        assert!(second > first);
    }

    #[tokio::test]
    async fn bounded_buffer_applies_backpressure() {
        let cursor = Arc::new(InMemoryCursorStore::new());
        let source: Arc<dyn LedgerSource> = Arc::new(SyntheticLedgerSource::new(0, 8));
        let pipeline = Arc::new(
            IngestionPipeline::new(
                IngestionConfig {
                    buffer_capacity: 2,
                    max_ledgers_per_minute: 10_000,
                    enqueue_timeout: Duration::from_millis(200),
                    ..IngestionConfig::default()
                },
                source,
                cursor,
            )
            .unwrap(),
        );

        // Consumer lags: spawn a slow drain so produce hits a full buffer.
        let consumer = {
            let pipeline = pipeline.clone();
            tokio::spawn(async move {
                tokio::time::sleep(Duration::from_millis(30)).await;
                let mut total = 0;
                for _ in 0..10 {
                    total += pipeline.consume_available().await.unwrap_or(0);
                    tokio::time::sleep(Duration::from_millis(5)).await;
                }
                total
            })
        };

        let produced = pipeline.produce_once().await.unwrap();
        assert!(produced > 0);
        let _ = consumer.await.unwrap();
        assert!(pipeline.metrics().backpressure_waits > 0);
    }

    #[tokio::test]
    async fn rpc_backoff_recovers_from_transient_failures() {
        let cursor = Arc::new(InMemoryCursorStore::new());
        let source: Arc<dyn LedgerSource> = Arc::new(
            SyntheticLedgerSource::new(0, 3).with_transient_failures(2),
        );
        // First two RPC calls fail; produce_once backs off then succeeds.
        let pipeline = IngestionPipeline::new(
            IngestionConfig {
                buffer_capacity: 16,
                max_ledgers_per_minute: 10_000,
                initial_backoff: Duration::from_millis(1),
                max_backoff: Duration::from_millis(5),
                ..IngestionConfig::default()
            },
            source,
            cursor,
        )
        .unwrap();

        let n = pipeline.produce_once().await.unwrap();
        assert!(n > 0);
        assert!(pipeline.metrics().rpc_failures >= 1);
    }

    /// Load test: sustain ~500 ledgers/minute ingestion rate.
    #[tokio::test]
    async fn load_test_five_hundred_ledgers_per_minute() {
        let cursor = Arc::new(InMemoryCursorStore::new());
        // Compress the minute window for the test by raising the rate cap and
        // measuring throughput over a short wall-clock window, then extrapolating.
        let source: Arc<dyn LedgerSource> = Arc::new(SyntheticLedgerSource::new(0, 25));
        let pipeline = IngestionPipeline::new(
            IngestionConfig {
                buffer_capacity: 128,
                max_ledgers_per_minute: TARGET_LEDGERS_PER_MINUTE * 20,
                initial_backoff: Duration::from_millis(1),
                max_backoff: Duration::from_millis(10),
                enqueue_timeout: Duration::from_secs(1),
                ..IngestionConfig::default()
            },
            source,
            cursor.clone(),
        )
        .unwrap();

        let target = TARGET_LEDGERS_PER_MINUTE;
        let started = Instant::now();
        let metrics = pipeline.run_until(target).await.unwrap();
        let elapsed = started.elapsed();

        assert_eq!(metrics.ingested, target);
        assert_eq!(cursor.current(), Some(metrics.last_sequence));

        // Extrapolate to ledgers/minute; require at least the production target.
        let per_minute =
            (metrics.ingested as f64) / elapsed.as_secs_f64().max(0.001) * 60.0;
        assert!(
            per_minute >= TARGET_LEDGERS_PER_MINUTE as f64,
            "throughput {per_minute:.1} ledgers/min below target {}",
            TARGET_LEDGERS_PER_MINUTE
        );
    }

    #[test]
    fn rejects_zero_buffer_capacity() {
        let err = IngestionPipeline::new(
            IngestionConfig {
                buffer_capacity: 0,
                ..IngestionConfig::default()
            },
            Arc::new(SyntheticLedgerSource::new(0, 1)),
            Arc::new(InMemoryCursorStore::new()),
        )
        .unwrap_err();
        assert!(matches!(err, IngestionError::InvalidConfig(_)));
    }
}
