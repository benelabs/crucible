//! Contract performance benchmarking service.
//!
//! The service accepts already-measured Soroban contract invocation samples,
//! validates a bounded workload, computes deterministic aggregate statistics,
//! and stores a small in-memory history for operational inspection. It is
//! intentionally independent from HTTP and persistence concerns so API handlers
//! can remain thin and tests can exercise the domain logic directly.

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
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
        self.durations.sort_unstable();
        let p95_duration_us = percentile_nearest_rank(&self.durations, 95);
        let sample_count = self.sample_count as f64;

        OperationBenchmarkSummary {
            operation,
            sample_count: self.sample_count,
            success_rate: self.success_count as f64 / sample_count,
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

/// Stateless benchmark calculator with bounded in-memory report history.
#[derive(Clone, Default)]
pub struct ContractBenchmarkService {
    history: Arc<RwLock<HashMap<String, VecDeque<ContractBenchmarkReport>>>>,
}

impl ContractBenchmarkService {
    /// Creates a benchmark service.
    pub fn new() -> Self {
        Self::default()
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

        self.record_report(report.clone()).await;
        Ok(report)
    }

    /// Returns recent reports for one contract, newest first.
    pub async fn recent_reports(&self, contract_id: &str) -> Vec<ContractBenchmarkReport> {
        self.history
            .read()
            .await
            .get(contract_id)
            .map(|reports| reports.iter().rev().cloned().collect())
            .unwrap_or_default()
    }

    async fn record_report(&self, report: ContractBenchmarkReport) {
        let mut history = self.history.write().await;
        let reports = history.entry(report.contract_id.clone()).or_default();
        reports.push_back(report);
        while reports.len() > HISTORY_LIMIT_PER_CONTRACT {
            reports.pop_front();
        }
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

    for sample in &request.samples {
        validate_identifier("operation", &sample.operation)?;
    }

    Ok(())
}

fn default_resource_fee_regression_threshold_pct() -> f64 {
    10.0
}

fn validate_identifier(field: &str, value: &str) -> Result<(), ContractBenchmarkError> {
    if value.is_empty() || value.len() > MAX_ID_LEN {
        return Err(ContractBenchmarkError::Validation(format!(
            "{field} must be between 1 and {MAX_ID_LEN} characters"
        )));
    }

    if !value
        .bytes()
        .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'_' | b'-' | b':' | b'.'))
    {
        return Err(ContractBenchmarkError::Validation(format!(
            "{field} may contain only letters, numbers, '_', '-', ':' or '.'"
        )));
    }

    Ok(())
}

fn validate_percentage(field: &str, value: f64) -> Result<(), ContractBenchmarkError> {
    validate_non_negative_finite(field, value).map_err(|_| {
        ContractBenchmarkError::Validation(format!(
            "{field} must be a finite percentage greater than or equal to 0"
        ))
    })
}

fn validate_non_negative_finite(field: &str, value: f64) -> Result<(), ContractBenchmarkError> {
    if value.is_finite() && value >= 0.0 {
        return Ok(());
    }

    Err(ContractBenchmarkError::Validation(format!(
        "{field} must be finite and greater than or equal to 0"
    )))
}

fn percentile_nearest_rank(sorted_values: &[u64], percentile: usize) -> u64 {
    let rank = (percentile * sorted_values.len()).div_ceil(100);
    sorted_values[rank.saturating_sub(1)]
}

fn saturating_u128_to_u64(value: u128) -> u64 {
    value.min(u128::from(u64::MAX)) as u64
}

fn compare_baseline(
    overall: &OperationBenchmarkSummary,
    baseline: &ContractBenchmarkBaseline,
    thresholds: ContractBenchmarkThresholds,
) -> Vec<BenchmarkRegression> {
    let mut regressions = Vec::with_capacity(3);

    push_regression(
        &mut regressions,
        "p95_duration_us",
        baseline.p95_duration_us as f64,
        overall.p95_duration_us as f64,
        thresholds.max_p95_latency_regression_pct,
    );
    push_regression(
        &mut regressions,
        "avg_cpu_instructions",
        baseline.avg_cpu_instructions,
        overall.avg_cpu_instructions,
        thresholds.max_cpu_instruction_regression_pct,
    );
    push_regression(
        &mut regressions,
        "peak_memory_bytes",
        baseline.peak_memory_bytes as f64,
        overall.peak_memory_bytes as f64,
        thresholds.max_memory_regression_pct,
    );
    push_regression(
        &mut regressions,
        "avg_resource_fee_stroops",
        baseline.avg_resource_fee_stroops,
        overall.avg_resource_fee_stroops,
        thresholds.max_resource_fee_regression_pct,
    );

    regressions
}

fn push_regression(
    regressions: &mut Vec<BenchmarkRegression>,
    metric: &str,
    baseline: f64,
    current: f64,
    threshold_pct: f64,
) {
    if baseline <= 0.0 {
        return;
    }

    let change_pct = ((current - baseline) / baseline) * 100.0;
    if change_pct > threshold_pct {
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
}
