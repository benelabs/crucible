//! Contract performance benchmarking service.
//!
//! The service accepts already-measured Soroban contract invocation samples,
//! validates a bounded workload, computes deterministic aggregate statistics,
//! stores a bounded in-memory cache, and persists benchmark runs and baselines
//! into PostgreSQL tables with an indexed schema for historical time-series queries (#1212).

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use sqlx::{PgPool, Row};
use std::collections::{HashMap, VecDeque};
use std::sync::Arc;
use thiserror::Error;
use tokio::sync::RwLock;
use uuid::Uuid;

const MAX_ID_LEN: usize = 128;
const MAX_SAMPLES: usize = 10_000;
const HISTORY_LIMIT_PER_CONTRACT: usize = 50;

/// Errors returned by [`ContractBenchmarkService`].
#[derive(Debug, Error, PartialEq, Eq)]
pub enum ContractBenchmarkError {
    /// Request payload failed domain validation.
    #[error("validation error: {0}")]
    Validation(String),
    /// Database persistence error.
    #[error("database error: {0}")]
    Database(String),
}

/// One measured contract operation invocation.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct ContractBenchmarkSample {
    pub operation: String,
    pub duration_us: u64,
    #[serde(alias = "instructions")]
    pub cpu_instructions: u64,
    pub memory_bytes: u64,
    #[serde(default)]
    pub ledger_read_entries: u64,
    #[serde(default)]
    pub ledger_write_entries: u64,
    #[serde(default)]
    pub ledger_read_bytes: u64,
    #[serde(default)]
    pub ledger_write_bytes: u64,
    #[serde(default)]
    pub transaction_size_bytes: u64,
    #[serde(default)]
    pub events_return_bytes: u64,
    #[serde(default)]
    pub ledger_space_rent_stroops: u64,
    #[serde(default)]
    pub resource_fee_stroops: u64,
    pub success: bool,
}

/// Optional baseline used to flag regressions.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct ContractBenchmarkBaseline {
    pub p95_duration_us: u64,
    #[serde(alias = "avg_instructions")]
    pub avg_cpu_instructions: f64,
    pub peak_memory_bytes: u64,
    #[serde(default)]
    pub avg_resource_fee_stroops: f64,
}

/// Optional percentage thresholds applied to a baseline comparison.
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq)]
pub struct ContractBenchmarkThresholds {
    pub max_p95_latency_regression_pct: f64,
    #[serde(alias = "max_instruction_regression_pct")]
    pub max_cpu_instruction_regression_pct: f64,
    pub max_memory_regression_pct: f64,
    #[serde(default = "default_resource_fee_regression_threshold_pct")]
    pub max_resource_fee_regression_pct: f64,
}

fn default_resource_fee_regression_threshold_pct() -> f64 {
    10.0
}

impl Default for ContractBenchmarkThresholds {
    fn default() -> Self {
        Self {
            max_p95_latency_regression_pct: 10.0,
            max_cpu_instruction_regression_pct: 10.0,
            max_memory_regression_pct: 10.0,
            max_resource_fee_regression_pct: default_resource_fee_regression_threshold_pct(),
        }
    }
}

/// Benchmark request consumed by the service and API.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct ContractBenchmarkRequest {
    pub contract_id: String,
    pub benchmark_name: String,
    pub samples: Vec<ContractBenchmarkSample>,
    #[serde(default)]
    pub baseline: Option<ContractBenchmarkBaseline>,
    #[serde(default)]
    pub thresholds: Option<ContractBenchmarkThresholds>,
}

/// Health classification for a benchmark run.
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum BenchmarkStatus {
    Passed,
    Warning,
    Failed,
}

/// Aggregate statistics for one contract operation.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct OperationBenchmarkSummary {
    pub operation: String,
    pub sample_count: u64,
    pub success_rate: f64,
    pub min_duration_us: u64,
    pub avg_duration_us: f64,
    pub p95_duration_us: u64,
    pub max_duration_us: u64,
    pub avg_cpu_instructions: f64,
    pub total_cpu_instructions: u64,
    pub avg_memory_bytes: f64,
    pub peak_memory_bytes: u64,
    pub total_ledger_read_entries: u64,
    pub total_ledger_write_entries: u64,
    pub total_ledger_read_bytes: u64,
    pub total_ledger_write_bytes: u64,
    pub total_transaction_size_bytes: u64,
    pub total_events_return_bytes: u64,
    pub total_ledger_space_rent_stroops: u64,
    pub total_resource_fee_stroops: u64,
    pub avg_resource_fee_stroops: f64,
}

impl OperationBenchmarkSummary {
    /// Constructs an explicit zeroed summary for operations with empty samples (#1213).
    pub fn zeroed(operation: impl Into<String>) -> Self {
        Self {
            operation: operation.into(),
            sample_count: 0,
            success_rate: 0.0,
            min_duration_us: 0,
            avg_duration_us: 0.0,
            p95_duration_us: 0,
            max_duration_us: 0,
            avg_cpu_instructions: 0.0,
            total_cpu_instructions: 0,
            avg_memory_bytes: 0.0,
            peak_memory_bytes: 0,
            total_ledger_read_entries: 0,
            total_ledger_write_entries: 0,
            total_ledger_read_bytes: 0,
            total_ledger_write_bytes: 0,
            total_transaction_size_bytes: 0,
            total_events_return_bytes: 0,
            total_ledger_space_rent_stroops: 0,
            total_resource_fee_stroops: 0,
            avg_resource_fee_stroops: 0.0,
        }
    }
}

/// Regression detail emitted when a metric exceeds its threshold.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct BenchmarkRegression {
    pub metric: String,
    pub baseline: f64,
    pub current: f64,
    pub change_pct: f64,
    pub threshold_pct: f64,
}

/// Complete benchmark report.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct ContractBenchmarkReport {
    pub benchmark_id: Uuid,
    pub contract_id: String,
    pub benchmark_name: String,
    pub status: BenchmarkStatus,
    pub sample_count: u64,
    pub generated_at: DateTime<Utc>,
    pub operations: Vec<OperationBenchmarkSummary>,
    pub overall: OperationBenchmarkSummary,
    pub regressions: Vec<BenchmarkRegression>,
}

#[derive(Default)]
struct OperationAccumulator {
    sample_count: u64,
    success_count: u64,
    total_duration_us: u128,
    min_duration_us: u64,
    max_duration_us: u64,
    total_cpu_instructions: u128,
    total_memory_bytes: u128,
    peak_memory_bytes: u64,
    total_ledger_read_entries: u128,
    total_ledger_write_entries: u128,
    total_ledger_read_bytes: u128,
    total_ledger_write_bytes: u128,
    total_transaction_size_bytes: u128,
    total_events_return_bytes: u128,
    total_ledger_space_rent_stroops: u128,
    total_resource_fee_stroops: u128,
    durations: Vec<u64>,
}

impl OperationAccumulator {
    fn push(&mut self, sample: &ContractBenchmarkSample) {
        if self.sample_count == 0 {
            self.min_duration_us = sample.duration_us;
        }

        self.sample_count += 1;
        self.success_count += u64::from(sample.success);
        self.total_duration_us += u128::from(sample.duration_us);
        self.min_duration_us = self.min_duration_us.min(sample.duration_us);
        self.max_duration_us = self.max_duration_us.max(sample.duration_us);
        self.total_cpu_instructions += u128::from(sample.cpu_instructions);
        self.total_memory_bytes += u128::from(sample.memory_bytes);
        self.peak_memory_bytes = self.peak_memory_bytes.max(sample.memory_bytes);
        self.total_ledger_read_entries += u128::from(sample.ledger_read_entries);
        self.total_ledger_write_entries += u128::from(sample.ledger_write_entries);
        self.total_ledger_read_bytes += u128::from(sample.ledger_read_bytes);
        self.total_ledger_write_bytes += u128::from(sample.ledger_write_bytes);
        self.total_transaction_size_bytes += u128::from(sample.transaction_size_bytes);
        self.total_events_return_bytes += u128::from(sample.events_return_bytes);
        self.total_ledger_space_rent_stroops += u128::from(sample.ledger_space_rent_stroops);
        self.total_resource_fee_stroops += u128::from(sample.resource_fee_stroops);
        self.durations.push(sample.duration_us);
    }

    fn into_summary(mut self, operation: String) -> OperationBenchmarkSummary {
        if self.sample_count == 0 {
            return OperationBenchmarkSummary::zeroed(operation);
        }

        self.durations.sort_unstable();
        let p95_duration_us = percentile_nearest_rank(&self.durations, 95);
        let sample_count = self.sample_count as f64;

        OperationBenchmarkSummary {
            operation,
            sample_count: self.sample_count,
            success_rate: (self.success_count as f64 / sample_count).clamp(0.0, 1.0),
            min_duration_us: self.min_duration_us,
            avg_duration_us: self.total_duration_us as f64 / sample_count,
            p95_duration_us,
            max_duration_us: self.max_duration_us,
            avg_cpu_instructions: self.total_cpu_instructions as f64 / sample_count,
            total_cpu_instructions: saturating_u128_to_u64(self.total_cpu_instructions),
            avg_memory_bytes: self.total_memory_bytes as f64 / sample_count,
            peak_memory_bytes: self.peak_memory_bytes,
            total_ledger_read_entries: saturating_u128_to_u64(self.total_ledger_read_entries),
            total_ledger_write_entries: saturating_u128_to_u64(self.total_ledger_write_entries),
            total_ledger_read_bytes: saturating_u128_to_u64(self.total_ledger_read_bytes),
            total_ledger_write_bytes: saturating_u128_to_u64(self.total_ledger_write_bytes),
            total_transaction_size_bytes: saturating_u128_to_u64(self.total_transaction_size_bytes),
            total_events_return_bytes: saturating_u128_to_u64(self.total_events_return_bytes),
            total_ledger_space_rent_stroops: saturating_u128_to_u64(
                self.total_ledger_space_rent_stroops,
            ),
            total_resource_fee_stroops: saturating_u128_to_u64(self.total_resource_fee_stroops),
            avg_resource_fee_stroops: self.total_resource_fee_stroops as f64 / sample_count,
        }
    }
}

/// Benchmark service with in-memory caching and optional PostgreSQL persistence (#1212).
#[derive(Clone, Default)]
pub struct ContractBenchmarkService {
    history: Arc<RwLock<HashMap<String, VecDeque<ContractBenchmarkReport>>>>,
    pool: Option<PgPool>,
}

impl ContractBenchmarkService {
    /// Creates a benchmark service with in-memory storage.
    pub fn new() -> Self {
        Self::default()
    }

    /// Creates a benchmark service backed by a PostgreSQL connection pool.
    pub fn with_db(pool: PgPool) -> Self {
        Self {
            history: Arc::new(RwLock::new(HashMap::new())),
            pool: Some(pool),
        }
    }

    /// Attaches a PostgreSQL connection pool to an existing service instance.
    pub fn with_pool(mut self, pool: PgPool) -> Self {
        self.pool = Some(pool);
        self
    }

    /// Returns whether the service has an active database connection pool.
    pub fn has_db(&self) -> bool {
        self.pool.is_some()
    }

    /// Initializes PostgreSQL tables and indexes for benchmark runs and baselines.
    pub async fn init_schema(&self) -> Result<(), ContractBenchmarkError> {
        if let Some(ref pool) = self.pool {
            sqlx::query(
                r#"
                CREATE TABLE IF NOT EXISTS contract_benchmark_runs (
                    id UUID PRIMARY KEY,
                    contract_id TEXT NOT NULL,
                    benchmark_name TEXT NOT NULL,
                    status TEXT NOT NULL CHECK (status IN ('passed', 'warning', 'failed')),
                    sample_count BIGINT NOT NULL,
                    generated_at TIMESTAMPTZ NOT NULL DEFAULT NOW(),
                    overall_metrics JSONB NOT NULL,
                    operations JSONB NOT NULL DEFAULT '[]',
                    regressions JSONB NOT NULL DEFAULT '[]',
                    baseline_metrics JSONB
                );

                CREATE INDEX IF NOT EXISTS idx_benchmark_runs_contract_generated
                    ON contract_benchmark_runs (contract_id, generated_at DESC);
                CREATE INDEX IF NOT EXISTS idx_benchmark_runs_name_generated
                    ON contract_benchmark_runs (benchmark_name, generated_at DESC);
                CREATE INDEX IF NOT EXISTS idx_benchmark_runs_status
                    ON contract_benchmark_runs (status);

                CREATE TABLE IF NOT EXISTS contract_benchmark_baselines (
                    contract_id TEXT NOT NULL,
                    benchmark_name TEXT NOT NULL,
                    p95_duration_us BIGINT NOT NULL,
                    avg_cpu_instructions DOUBLE PRECISION NOT NULL,
                    peak_memory_bytes BIGINT NOT NULL,
                    avg_resource_fee_stroops DOUBLE PRECISION NOT NULL,
                    updated_at TIMESTAMPTZ NOT NULL DEFAULT NOW(),
                    PRIMARY KEY (contract_id, benchmark_name)
                );

                CREATE INDEX IF NOT EXISTS idx_benchmark_baselines_lookup
                    ON contract_benchmark_baselines (contract_id, benchmark_name);
                "#,
            )
            .execute(pool)
            .await
            .map_err(|e| ContractBenchmarkError::Database(e.to_string()))?;
        }
        Ok(())
    }

    /// Validates, aggregates, and records a contract benchmark report.
    pub async fn run_benchmark(
        &self,
        request: ContractBenchmarkRequest,
    ) -> Result<ContractBenchmarkReport, ContractBenchmarkError> {
        validate_request(&request)?;

        let mut by_operation: HashMap<String, OperationAccumulator> = HashMap::new();
        let mut overall = OperationAccumulator::default();

        for sample in &request.samples {
            by_operation
                .entry(sample.operation.clone())
                .or_default()
                .push(sample);
            overall.push(sample);
        }

        let mut operations = by_operation
            .into_iter()
            .map(|(operation, accumulator)| accumulator.into_summary(operation))
            .collect::<Vec<_>>();
        operations.sort_by(|left, right| left.operation.cmp(&right.operation));

        let overall = overall.into_summary("overall".to_string());
        let thresholds = request.thresholds.unwrap_or_default();
        let regressions = request
            .baseline
            .as_ref()
            .map(|baseline| compare_baseline(&overall, baseline, thresholds))
            .unwrap_or_default();

        let status = classify_status(overall.success_rate, !regressions.is_empty());
        let report = ContractBenchmarkReport {
            benchmark_id: Uuid::new_v4(),
            contract_id: request.contract_id,
            benchmark_name: request.benchmark_name,
            status,
            sample_count: overall.sample_count,
            generated_at: Utc::now(),
            operations,
            overall,
            regressions,
        };

        self.record_report(report.clone(), request.baseline.as_ref())
            .await;
        Ok(report)
    }

    /// Returns recent reports for one contract, newest first.
    /// If PostgreSQL pool is configured, loads from persistent storage; otherwise uses in-memory cache.
    pub async fn recent_reports(&self, contract_id: &str) -> Vec<ContractBenchmarkReport> {
        if let Some(ref pool) = self.pool {
            if let Ok(reports) = self
                .query_historical_reports(
                    pool,
                    contract_id,
                    Some(HISTORY_LIMIT_PER_CONTRACT as i64),
                    None,
                )
                .await
            {
                if !reports.is_empty() {
                    return reports;
                }
            }
        }

        self.history
            .read()
            .await
            .get(contract_id)
            .map(|reports| reports.iter().rev().cloned().collect())
            .unwrap_or_default()
    }

    /// Queries historical benchmark reports from PostgreSQL time-series with optional limit and since filter.
    pub async fn historical_reports(
        &self,
        contract_id: &str,
        limit: Option<i64>,
        since: Option<DateTime<Utc>>,
    ) -> Result<Vec<ContractBenchmarkReport>, ContractBenchmarkError> {
        if let Some(ref pool) = self.pool {
            self.query_historical_reports(pool, contract_id, limit, since)
                .await
        } else {
            let history = self.history.read().await;
            let reports = history.get(contract_id);
            let mut filtered = match reports {
                Some(entries) => entries
                    .iter()
                    .filter(|r| since.map(|s| r.generated_at >= s).unwrap_or(true))
                    .rev()
                    .cloned()
                    .collect::<Vec<_>>(),
                None => Vec::new(),
            };
            if let Some(lim) = limit {
                filtered.truncate(lim.max(0) as usize);
            }
            Ok(filtered)
        }
    }

    /// Retrieves the latest saved baseline metrics for a contract and benchmark.
    pub async fn get_baseline(
        &self,
        contract_id: &str,
        benchmark_name: &str,
    ) -> Result<Option<ContractBenchmarkBaseline>, ContractBenchmarkError> {
        if let Some(ref pool) = self.pool {
            let row_opt = sqlx::query(
                r#"
                SELECT p95_duration_us, avg_cpu_instructions, peak_memory_bytes, avg_resource_fee_stroops
                FROM contract_benchmark_baselines
                WHERE contract_id = $1 AND benchmark_name = $2
                "#,
            )
            .bind(contract_id)
            .bind(benchmark_name)
            .fetch_optional(pool)
            .await
            .map_err(|e| ContractBenchmarkError::Database(e.to_string()))?;

            if let Some(row) = row_opt {
                let p95: i64 = row
                    .try_get("p95_duration_us")
                    .map_err(|e| ContractBenchmarkError::Database(e.to_string()))?;
                let avg_cpu: f64 = row
                    .try_get("avg_cpu_instructions")
                    .map_err(|e| ContractBenchmarkError::Database(e.to_string()))?;
                let peak_mem: i64 = row
                    .try_get("peak_memory_bytes")
                    .map_err(|e| ContractBenchmarkError::Database(e.to_string()))?;
                let avg_fee: f64 = row
                    .try_get("avg_resource_fee_stroops")
                    .map_err(|e| ContractBenchmarkError::Database(e.to_string()))?;

                return Ok(Some(ContractBenchmarkBaseline {
                    p95_duration_us: p95 as u64,
                    avg_cpu_instructions: avg_cpu,
                    peak_memory_bytes: peak_mem as u64,
                    avg_resource_fee_stroops: avg_fee,
                }));
            }
        }
        Ok(None)
    }

    async fn record_report(
        &self,
        report: ContractBenchmarkReport,
        baseline: Option<&ContractBenchmarkBaseline>,
    ) {
        {
            let mut history = self.history.write().await;
            let reports = history.entry(report.contract_id.clone()).or_default();
            reports.push_back(report.clone());
            while reports.len() > HISTORY_LIMIT_PER_CONTRACT {
                reports.pop_front();
            }
        }

        if let Some(ref pool) = self.pool {
            let overall_json = serde_json::to_value(&report.overall).unwrap_or_default();
            let operations_json = serde_json::to_value(&report.operations).unwrap_or_default();
            let regressions_json = serde_json::to_value(&report.regressions).unwrap_or_default();
            let baseline_json = baseline.map(|b| serde_json::to_value(b).unwrap_or_default());
            let status_str = match report.status {
                BenchmarkStatus::Passed => "passed",
                BenchmarkStatus::Warning => "warning",
                BenchmarkStatus::Failed => "failed",
            };

            let _ = sqlx::query(
                r#"
                INSERT INTO contract_benchmark_runs (
                    id, contract_id, benchmark_name, status, sample_count,
                    generated_at, overall_metrics, operations, regressions, baseline_metrics
                ) VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9, $10)
                ON CONFLICT (id) DO NOTHING
                "#,
            )
            .bind(report.benchmark_id)
            .bind(&report.contract_id)
            .bind(&report.benchmark_name)
            .bind(status_str)
            .bind(report.sample_count as i64)
            .bind(report.generated_at)
            .bind(overall_json)
            .bind(operations_json)
            .bind(regressions_json)
            .bind(baseline_json)
            .execute(pool)
            .await;

            if let Some(b) = baseline {
                let _ = sqlx::query(
                    r#"
                    INSERT INTO contract_benchmark_baselines (
                        contract_id, benchmark_name, p95_duration_us, avg_cpu_instructions,
                        peak_memory_bytes, avg_resource_fee_stroops, updated_at
                    ) VALUES ($1, $2, $3, $4, $5, $6, NOW())
                    ON CONFLICT (contract_id, benchmark_name) DO UPDATE SET
                        p95_duration_us = EXCLUDED.p95_duration_us,
                        avg_cpu_instructions = EXCLUDED.avg_cpu_instructions,
                        peak_memory_bytes = EXCLUDED.peak_memory_bytes,
                        avg_resource_fee_stroops = EXCLUDED.avg_resource_fee_stroops,
                        updated_at = NOW()
                    "#,
                )
                .bind(&report.contract_id)
                .bind(&report.benchmark_name)
                .bind(b.p95_duration_us as i64)
                .bind(b.avg_cpu_instructions)
                .bind(b.peak_memory_bytes as i64)
                .bind(b.avg_resource_fee_stroops)
                .execute(pool)
                .await;
            }
        }
    }

    async fn query_historical_reports(
        &self,
        pool: &PgPool,
        contract_id: &str,
        limit: Option<i64>,
        since: Option<DateTime<Utc>>,
    ) -> Result<Vec<ContractBenchmarkReport>, ContractBenchmarkError> {
        let limit_val = limit.unwrap_or(50).clamp(1, 1000);
        let rows = if let Some(since_dt) = since {
            sqlx::query(
                r#"
                SELECT id, contract_id, benchmark_name, status, sample_count,
                       generated_at, overall_metrics, operations, regressions
                FROM contract_benchmark_runs
                WHERE contract_id = $1 AND generated_at >= $2
                ORDER BY generated_at DESC
                LIMIT $3
                "#,
            )
            .bind(contract_id)
            .bind(since_dt)
            .bind(limit_val)
            .fetch_all(pool)
            .await
        } else {
            sqlx::query(
                r#"
                SELECT id, contract_id, benchmark_name, status, sample_count,
                       generated_at, overall_metrics, operations, regressions
                FROM contract_benchmark_runs
                WHERE contract_id = $1
                ORDER BY generated_at DESC
                LIMIT $2
                "#,
            )
            .bind(contract_id)
            .bind(limit_val)
            .fetch_all(pool)
            .await
        }
        .map_err(|e| ContractBenchmarkError::Database(e.to_string()))?;

        let mut reports = Vec::with_capacity(rows.len());
        for row in rows {
            let id: Uuid = row
                .try_get("id")
                .map_err(|e| ContractBenchmarkError::Database(e.to_string()))?;
            let cid: String = row
                .try_get("contract_id")
                .map_err(|e| ContractBenchmarkError::Database(e.to_string()))?;
            let bname: String = row
                .try_get("benchmark_name")
                .map_err(|e| ContractBenchmarkError::Database(e.to_string()))?;
            let status_str: String = row
                .try_get("status")
                .map_err(|e| ContractBenchmarkError::Database(e.to_string()))?;
            let sample_count: i64 = row
                .try_get("sample_count")
                .map_err(|e| ContractBenchmarkError::Database(e.to_string()))?;
            let generated_at: DateTime<Utc> = row
                .try_get("generated_at")
                .map_err(|e| ContractBenchmarkError::Database(e.to_string()))?;
            let overall_val: serde_json::Value = row
                .try_get("overall_metrics")
                .map_err(|e| ContractBenchmarkError::Database(e.to_string()))?;
            let operations_val: serde_json::Value = row
                .try_get("operations")
                .map_err(|e| ContractBenchmarkError::Database(e.to_string()))?;
            let regressions_val: serde_json::Value = row
                .try_get("regressions")
                .map_err(|e| ContractBenchmarkError::Database(e.to_string()))?;

            let status = match status_str.as_str() {
                "passed" => BenchmarkStatus::Passed,
                "warning" => BenchmarkStatus::Warning,
                _ => BenchmarkStatus::Failed,
            };
            let overall: OperationBenchmarkSummary = serde_json::from_value(overall_val)
                .map_err(|e| ContractBenchmarkError::Database(e.to_string()))?;
            let operations: Vec<OperationBenchmarkSummary> = serde_json::from_value(operations_val)
                .map_err(|e| ContractBenchmarkError::Database(e.to_string()))?;
            let regressions: Vec<BenchmarkRegression> = serde_json::from_value(regressions_val)
                .map_err(|e| ContractBenchmarkError::Database(e.to_string()))?;

            reports.push(ContractBenchmarkReport {
                benchmark_id: id,
                contract_id: cid,
                benchmark_name: bname,
                status,
                sample_count: sample_count as u64,
                generated_at,
                operations,
                overall,
                regressions,
            });
        }

        Ok(reports)
    }
}

fn validate_request(request: &ContractBenchmarkRequest) -> Result<(), ContractBenchmarkError> {
    validate_identifier("contract_id", &request.contract_id)?;
    validate_identifier("benchmark_name", &request.benchmark_name)?;

    if let Some(baseline) = &request.baseline {
        validate_non_negative_finite(
            "baseline.avg_cpu_instructions",
            baseline.avg_cpu_instructions,
        )?;
        validate_non_negative_finite(
            "baseline.avg_resource_fee_stroops",
            baseline.avg_resource_fee_stroops,
        )?;
    }

    if request.samples.is_empty() || request.samples.len() > MAX_SAMPLES {
        return Err(ContractBenchmarkError::Validation(format!(
            "samples must contain between 1 and {MAX_SAMPLES} entries"
        )));
    }

    if let Some(thresholds) = request.thresholds {
        validate_percentage(
            "max_p95_latency_regression_pct",
            thresholds.max_p95_latency_regression_pct,
        )?;
        validate_percentage(
            "max_cpu_instruction_regression_pct",
            thresholds.max_cpu_instruction_regression_pct,
        )?;
        validate_percentage(
            "max_memory_regression_pct",
            thresholds.max_memory_regression_pct,
        )?;
        validate_percentage(
            "max_resource_fee_regression_pct",
            thresholds.max_resource_fee_regression_pct,
        )?;
    }

    for (index, sample) in request.samples.iter().enumerate() {
        validate_identifier(&format!("samples[{index}].operation"), &sample.operation)?;
    }

    Ok(())
}

fn validate_identifier(field: &str, value: &str) -> Result<(), ContractBenchmarkError> {
    if value.trim().is_empty() {
        return Err(ContractBenchmarkError::Validation(format!(
            "{field} cannot be empty"
        )));
    }

    if value.len() > MAX_ID_LEN {
        return Err(ContractBenchmarkError::Validation(format!(
            "{field} exceeds max length of {MAX_ID_LEN} bytes"
        )));
    }

    let is_valid = value
        .chars()
        .all(|character| character.is_ascii_alphanumeric() || character == '_' || character == '-');

    if !is_valid {
        return Err(ContractBenchmarkError::Validation(format!(
            "{field} may contain only ASCII alphanumeric characters, underscores, and dashes"
        )));
    }

    Ok(())
}

fn validate_percentage(field: &str, value: f64) -> Result<(), ContractBenchmarkError> {
    if !value.is_finite() || !(0.0..=1_000.0).contains(&value) {
        return Err(ContractBenchmarkError::Validation(format!(
            "{field} must be a finite percentage between 0 and 1000"
        )));
    }

    Ok(())
}

fn validate_non_negative_finite(field: &str, value: f64) -> Result<(), ContractBenchmarkError> {
    if !value.is_finite() || value < 0.0 {
        return Err(ContractBenchmarkError::Validation(format!(
            "{field} must be finite and non-negative"
        )));
    }

    Ok(())
}

fn percentile_nearest_rank(sorted: &[u64], percentile: u8) -> u64 {
    if sorted.is_empty() {
        return 0;
    }

    let rank = ((f64::from(percentile) / 100.0) * sorted.len() as f64).ceil() as usize;
    let index = rank.saturating_sub(1).min(sorted.len() - 1);
    sorted[index]
}

fn saturating_u128_to_u64(value: u128) -> u64 {
    u64::try_from(value).unwrap_or(u64::MAX)
}

fn compare_baseline(
    overall: &OperationBenchmarkSummary,
    baseline: &ContractBenchmarkBaseline,
    thresholds: ContractBenchmarkThresholds,
) -> Vec<BenchmarkRegression> {
    let mut regressions = Vec::new();

    check_metric(
        &mut regressions,
        "p95_duration_us",
        baseline.p95_duration_us as f64,
        overall.p95_duration_us as f64,
        thresholds.max_p95_latency_regression_pct,
    );
    check_metric(
        &mut regressions,
        "avg_cpu_instructions",
        baseline.avg_cpu_instructions,
        overall.avg_cpu_instructions,
        thresholds.max_cpu_instruction_regression_pct,
    );
    check_metric(
        &mut regressions,
        "peak_memory_bytes",
        baseline.peak_memory_bytes as f64,
        overall.peak_memory_bytes as f64,
        thresholds.max_memory_regression_pct,
    );
    check_metric(
        &mut regressions,
        "avg_resource_fee_stroops",
        baseline.avg_resource_fee_stroops,
        overall.avg_resource_fee_stroops,
        thresholds.max_resource_fee_regression_pct,
    );

    regressions
}

fn check_metric(
    regressions: &mut Vec<BenchmarkRegression>,
    metric: &str,
    baseline: f64,
    current: f64,
    threshold_pct: f64,
) {
    if !baseline.is_finite() || baseline <= 0.0 || !current.is_finite() {
        return;
    }

    let change_pct = ((current - baseline) / baseline) * 100.0;
    if change_pct.is_finite() && change_pct > threshold_pct {
        regressions.push(BenchmarkRegression {
            metric: metric.to_string(),
            baseline,
            current,
            change_pct,
            threshold_pct,
        });
    }
}

fn classify_status(success_rate: f64, has_regressions: bool) -> BenchmarkStatus {
    if success_rate < 1.0 {
        BenchmarkStatus::Failed
    } else if has_regressions {
        BenchmarkStatus::Warning
    } else {
        BenchmarkStatus::Passed
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sample(operation: &str, duration_us: u64, instructions: u64) -> ContractBenchmarkSample {
        ContractBenchmarkSample {
            operation: operation.to_string(),
            duration_us,
            cpu_instructions: instructions,
            memory_bytes: duration_us * 10,
            ledger_read_entries: 1,
            ledger_write_entries: 0,
            ledger_read_bytes: 256,
            ledger_write_bytes: 0,
            transaction_size_bytes: 512,
            events_return_bytes: 64,
            ledger_space_rent_stroops: 0,
            resource_fee_stroops: duration_us * 2,
            success: true,
        }
    }

    fn request(samples: Vec<ContractBenchmarkSample>) -> ContractBenchmarkRequest {
        ContractBenchmarkRequest {
            contract_id: "counter".to_string(),
            benchmark_name: "increment".to_string(),
            samples,
            baseline: None,
            thresholds: None,
        }
    }

    #[tokio::test]
    async fn benchmark_aggregates_overall_and_operation_summaries() {
        let service = ContractBenchmarkService::new();
        let report = service
            .run_benchmark(request(vec![
                sample("inc", 10, 100),
                sample("inc", 30, 300),
                sample("get", 20, 200),
            ]))
            .await
            .unwrap();

        assert_eq!(report.status, BenchmarkStatus::Passed);
        assert_eq!(report.sample_count, 3);
        assert_eq!(report.overall.min_duration_us, 10);
        assert_eq!(report.overall.max_duration_us, 30);
        assert_eq!(report.overall.p95_duration_us, 30);
        assert_eq!(report.overall.avg_duration_us, 20.0);
        assert_eq!(report.operations.len(), 2);
        assert_eq!(report.operations[0].operation, "get");
        assert_eq!(report.operations[1].operation, "inc");
    }

    #[tokio::test]
    async fn benchmark_aggregates_soroban_resource_fields() {
        let service = ContractBenchmarkService::new();
        let mut first = sample("inc", 10, 100);
        first.ledger_write_entries = 1;
        first.ledger_write_bytes = 128;
        first.ledger_space_rent_stroops = 25;
        first.resource_fee_stroops = 200;

        let mut second = sample("inc", 20, 200);
        second.ledger_read_entries = 2;
        second.ledger_read_bytes = 512;
        second.transaction_size_bytes = 700;
        second.events_return_bytes = 100;
        second.resource_fee_stroops = 300;

        let report = service
            .run_benchmark(request(vec![first, second]))
            .await
            .unwrap();

        assert_eq!(report.overall.total_ledger_read_entries, 3);
        assert_eq!(report.overall.total_ledger_write_entries, 1);
        assert_eq!(report.overall.total_ledger_read_bytes, 768);
        assert_eq!(report.overall.total_ledger_write_bytes, 128);
        assert_eq!(report.overall.total_transaction_size_bytes, 1212);
        assert_eq!(report.overall.total_events_return_bytes, 164);
        assert_eq!(report.overall.total_ledger_space_rent_stroops, 25);
        assert_eq!(report.overall.total_resource_fee_stroops, 500);
        assert_eq!(report.overall.avg_resource_fee_stroops, 250.0);
    }

    #[tokio::test]
    async fn benchmark_flags_baseline_regression() {
        let service = ContractBenchmarkService::new();
        let mut req = request(vec![sample("inc", 120, 150), sample("inc", 140, 170)]);
        req.baseline = Some(ContractBenchmarkBaseline {
            p95_duration_us: 100,
            avg_cpu_instructions: 100.0,
            peak_memory_bytes: 1_000,
            avg_resource_fee_stroops: 1_000.0,
        });
        req.thresholds = Some(ContractBenchmarkThresholds {
            max_p95_latency_regression_pct: 10.0,
            max_cpu_instruction_regression_pct: 20.0,
            max_memory_regression_pct: 100.0,
            max_resource_fee_regression_pct: 100.0,
        });

        let report = service.run_benchmark(req).await.unwrap();

        assert_eq!(report.status, BenchmarkStatus::Warning);
        assert_eq!(report.regressions.len(), 2);
        assert_eq!(report.regressions[0].metric, "p95_duration_us");
        assert_eq!(report.regressions[1].metric, "avg_cpu_instructions");
    }

    #[tokio::test]
    async fn benchmark_rejects_non_finite_baseline_values() {
        let service = ContractBenchmarkService::new();
        let mut req = request(vec![sample("inc", 120, 150)]);
        req.baseline = Some(ContractBenchmarkBaseline {
            p95_duration_us: 100,
            avg_cpu_instructions: f64::NAN,
            peak_memory_bytes: 1_000,
            avg_resource_fee_stroops: 1_000.0,
        });

        let err = service.run_benchmark(req).await.unwrap_err();

        assert!(err
            .to_string()
            .contains("baseline.avg_cpu_instructions must be finite"));
    }

    #[tokio::test]
    async fn benchmark_fails_when_any_sample_failed() {
        let service = ContractBenchmarkService::new();
        let mut failed = sample("inc", 10, 100);
        failed.success = false;

        let report = service
            .run_benchmark(request(vec![sample("inc", 8, 80), failed]))
            .await
            .unwrap();

        assert_eq!(report.status, BenchmarkStatus::Failed);
        assert_eq!(report.overall.success_rate, 0.5);
    }

    #[tokio::test]
    async fn benchmark_rejects_empty_samples() {
        let service = ContractBenchmarkService::new();
        let err = service.run_benchmark(request(vec![])).await.unwrap_err();

        assert_eq!(
            err,
            ContractBenchmarkError::Validation(
                "samples must contain between 1 and 10000 entries".to_string()
            )
        );
    }

    #[tokio::test]
    async fn benchmark_rejects_unsafe_identifiers() {
        let service = ContractBenchmarkService::new();
        let mut req = request(vec![sample("bad op", 10, 100)]);
        req.contract_id = "counter".to_string();

        let err = service.run_benchmark(req).await.unwrap_err();

        assert!(err.to_string().contains("operation may contain only"));
    }

    #[tokio::test]
    async fn benchmark_keeps_bounded_recent_history() {
        let service = ContractBenchmarkService::new();

        for index in 0..60 {
            let mut req = request(vec![sample("inc", 10 + index, 100)]);
            req.benchmark_name = format!("run_{index}");
            service.run_benchmark(req).await.unwrap();
        }

        let reports = service.recent_reports("counter").await;
        assert_eq!(reports.len(), HISTORY_LIMIT_PER_CONTRACT);
        assert_eq!(reports[0].benchmark_name, "run_59");
        assert_eq!(reports[49].benchmark_name, "run_10");
    }

    #[test]
    fn test_empty_accumulator_produces_zeroed_summary_without_nan() {
        let acc = OperationAccumulator::default();
        let summary = acc.into_summary("empty_op".to_string());

        assert_eq!(summary.operation, "empty_op");
        assert_eq!(summary.sample_count, 0);
        assert_eq!(summary.success_rate, 0.0);
        assert_eq!(summary.avg_duration_us, 0.0);
        assert_eq!(summary.avg_cpu_instructions, 0.0);
        assert_eq!(summary.avg_memory_bytes, 0.0);
        assert_eq!(summary.avg_resource_fee_stroops, 0.0);

        // Verify that this summary serializes cleanly to JSON without NaN errors
        let json_str = serde_json::to_string(&summary);
        assert!(json_str.is_ok());
        let val: serde_json::Value = serde_json::from_str(&json_str.unwrap()).unwrap();
        assert_eq!(val["sample_count"], 0);
        assert_eq!(val["avg_duration_us"], 0.0);
    }

    #[test]
    fn test_operation_benchmark_summary_zeroed_helper() {
        let summary = OperationBenchmarkSummary::zeroed("custom_zero");
        assert_eq!(summary.operation, "custom_zero");
        assert_eq!(summary.sample_count, 0);
        assert_eq!(summary.avg_duration_us, 0.0);
        assert!(summary.avg_duration_us.is_finite());
        assert!(!summary.avg_duration_us.is_nan());
    }

    #[test]
    fn test_push_regression_guards_against_nan_and_zero_baseline() {
        let mut regressions = Vec::new();
        check_metric(&mut regressions, "test_metric", 0.0, 100.0, 10.0);
        assert!(regressions.is_empty());

        check_metric(&mut regressions, "test_metric", f64::NAN, 100.0, 10.0);
        assert!(regressions.is_empty());

        check_metric(&mut regressions, "test_metric", 100.0, f64::NAN, 10.0);
        assert!(regressions.is_empty());
    }

    #[tokio::test]
    async fn benchmark_historical_reports_with_time_series_filter() {
        let service = ContractBenchmarkService::new();
        let start_time = Utc::now();

        for index in 0..5 {
            let mut req = request(vec![sample("op", 10, 100)]);
            req.benchmark_name = format!("bench_{index}");
            service.run_benchmark(req).await.unwrap();
        }

        let all_hist = service
            .historical_reports("counter", Some(10), None)
            .await
            .unwrap();
        assert_eq!(all_hist.len(), 5);

        let limited = service
            .historical_reports("counter", Some(2), None)
            .await
            .unwrap();
        assert_eq!(limited.len(), 2);
        assert_eq!(limited[0].benchmark_name, "bench_4");
        assert_eq!(limited[1].benchmark_name, "bench_3");

        let filtered_since = service
            .historical_reports("counter", None, Some(start_time))
            .await
            .unwrap();
        assert_eq!(filtered_since.len(), 5);

        let future = Utc::now() + chrono::Duration::hours(1);
        let future_filtered = service
            .historical_reports("counter", None, Some(future))
            .await
            .unwrap();
        assert_eq!(future_filtered.len(), 0);
    }

    #[test]
    fn test_benchmark_report_json_roundtrip_for_db_jsonb() {
        let service = ContractBenchmarkService::new();
        let runtime = tokio::runtime::Runtime::new().unwrap();
        let report = runtime.block_on(async {
            service
                .run_benchmark(request(vec![sample("inc", 42, 1024)]))
                .await
                .unwrap()
        });

        let json_val = serde_json::to_value(&report).unwrap();
        let deserialized: ContractBenchmarkReport = serde_json::from_value(json_val).unwrap();

        assert_eq!(deserialized.benchmark_id, report.benchmark_id);
        assert_eq!(deserialized.contract_id, report.contract_id);
        assert_eq!(deserialized.overall.min_duration_us, 42);
    }
}
