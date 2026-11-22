use chrono::{DateTime, Datelike, Utc};
use serde::{Deserialize, Serialize};
use sqlx::PgPool;
use std::time::Duration;
use tokio::sync::mpsc::{channel, Sender};
use tracing::{info, instrument, warn};

use crate::error::AppError;
use crate::services::tracing::TracingService;

pub const DEFAULT_LOG_BUFFER_CAPACITY: usize = 10_000;
pub const DEFAULT_BATCH_SIZE: usize = 100;
pub const DEFAULT_FLUSH_INTERVAL: Duration = Duration::from_millis(500);

#[derive(Debug, Serialize, Deserialize, Clone, sqlx::FromRow)]
#[serde(rename_all = "camelCase")]
pub struct ContractCallLog {
    pub id: Option<i32>,
    pub contract_id: String,
    pub function_name: String,
    pub arguments: serde_json::Value,
    pub caller: Option<String>,
    pub status: String,
    pub gas_used: f64,
    pub timestamp: Option<DateTime<Utc>>,
}

/// Retention configuration policy for contract call logs (#1217).
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct LogRetentionPolicy {
    /// Number of days to retain invocation logs before pruning.
    pub retention_days: u32,
    /// Whether automatic retention pruning is enabled.
    pub auto_prune_enabled: bool,
    /// Number of future months of partitions to pre-create.
    pub precreate_months_ahead: u32,
}

impl Default for LogRetentionPolicy {
    fn default() -> Self {
        Self {
            retention_days: 90,
            auto_prune_enabled: true,
            precreate_months_ahead: 3,
        }
    }
}

/// Execution summary of retention maintenance and partition rotation (#1217).
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct RetentionMaintenanceReport {
    pub purged_records_count: u64,
    pub created_partitions: Vec<String>,
    pub dropped_partitions: Vec<String>,
    pub execution_timestamp: DateTime<Utc>,
}

#[derive(Clone)]
pub struct ContractCallLogger {
    pub db: PgPool,
    sender: Sender<ContractCallLog>,
}

impl ContractCallLogger {
    pub fn new(db: PgPool) -> Self {
        Self::with_config(
            db,
            DEFAULT_LOG_BUFFER_CAPACITY,
            DEFAULT_BATCH_SIZE,
            DEFAULT_FLUSH_INTERVAL,
        )
    }

    /// Formats standard partition table name for a specific year and month.
    pub fn get_partition_table_name(year: i32, month: u32) -> String {
        format!("contract_call_logs_{:04}_{:02}", year, month)
    }

    /// Generates PostgreSQL declarative partition DDL for a given month.
    pub fn generate_monthly_partition_ddl(year: i32, month: u32) -> String {
        let (next_year, next_month) = if month == 12 {
            (year + 1, 1)
        } else {
            (year, month + 1)
        };

        let partition_name = Self::get_partition_table_name(year, month);
        let from_bound = format!("{:04}-{:02}-01 00:00:00+00", year, month);
        let to_bound = format!("{:04}-{:02}-01 00:00:00+00", next_year, next_month);

        format!(
            "CREATE TABLE IF NOT EXISTS {partition_name} PARTITION OF contract_call_logs \
             FOR VALUES FROM ('{from_bound}') TO ('{to_bound}');"
        )
    }

    /// Computes the list of (year, month) pairs spanning current date to `months_ahead`.
    pub fn get_months_sequence(from_date: DateTime<Utc>, months_ahead: u32) -> Vec<(i32, u32)> {
        let mut result = Vec::new();
        let mut cur_year = from_date.year();
        let mut cur_month = from_date.month();

        for _ in 0..=months_ahead {
            result.push((cur_year, cur_month));
            if cur_month == 12 {
                cur_year += 1;
                cur_month = 1;
            } else {
                cur_month += 1;
            }
        }
        result
    }

    /// Pre-creates declarative monthly partitions up to `months_ahead` in advance.
    pub async fn ensure_monthly_partitions(
        &self,
        from_date: DateTime<Utc>,
        months_ahead: u32,
    ) -> Result<Vec<String>, AppError> {
        let months = Self::get_months_sequence(from_date, months_ahead);
        let mut created = Vec::new();

        for (year, month) in months {
            let partition_name = Self::get_partition_table_name(year, month);
            let ddl = Self::generate_monthly_partition_ddl(year, month);
            let _ = sqlx::query(&ddl).execute(&self.db).await;
            created.push(partition_name);
        }

        Ok(created)
    }

    /// Prunes invocation logs older than the specified retention window in days.
    pub async fn purge_expired_logs(&self, retention_days: u32) -> Result<u64, AppError> {
        let cutoff = Utc::now() - chrono::Duration::days(retention_days as i64);
        let result = sqlx::query("DELETE FROM contract_call_logs WHERE timestamp < $1")
            .bind(cutoff)
            .execute(&self.db)
            .await
            .map_err(AppError::db)?;

        Ok(result.rows_affected())
    }

    /// Drops monthly partitions whose upper boundary is older than the retention threshold.
    pub async fn drop_expired_partitions(
        &self,
        from_date: DateTime<Utc>,
        retention_months: u32,
    ) -> Result<Vec<String>, AppError> {
        let total_months = (from_date.year() * 12) + (from_date.month() as i32) - 1;
        let threshold_total = total_months - (retention_months as i32);
        let threshold_year = threshold_total / 12;
        let threshold_month = ((threshold_total % 12) + 1) as u32;

        let threshold_name = Self::get_partition_table_name(threshold_year, threshold_month);

        let rows = sqlx::query_scalar::<_, String>(
            "SELECT tablename FROM pg_tables WHERE schemaname = 'public' AND tablename LIKE 'contract_call_logs_20%'"
        )
        .fetch_all(&self.db)
        .await
        .unwrap_or_default();

        let mut dropped = Vec::new();
        for table in rows {
            if table < threshold_name {
                let drop_sql = format!("DROP TABLE IF EXISTS {} CASCADE;", table);
                if sqlx::query(&drop_sql).execute(&self.db).await.is_ok() {
                    dropped.push(table);
                }
            }
        }

        Ok(dropped)
    }

    /// Runs complete scheduled retention maintenance (partition generation + pruning).
    pub async fn run_retention_maintenance(
        &self,
        policy: &LogRetentionPolicy,
    ) -> Result<RetentionMaintenanceReport, AppError> {
        let now = Utc::now();
        let created_partitions = self
            .ensure_monthly_partitions(now, policy.precreate_months_ahead)
            .await?;

        let purged_records_count = if policy.auto_prune_enabled {
            self.purge_expired_logs(policy.retention_days).await?
        } else {
            0
        };

        let retention_months = (policy.retention_days / 30).max(1);
        let dropped_partitions = self.drop_expired_partitions(now, retention_months).await?;

        Ok(RetentionMaintenanceReport {
            purged_records_count,
            created_partitions,
            dropped_partitions,
            execution_timestamp: now,
        })
    }

    pub fn with_config(
        db: PgPool,
        capacity: usize,
        batch_size: usize,
        flush_interval: Duration,
    ) -> Self {
        let (tx, mut rx) = channel::<ContractCallLog>(capacity);
        let db_worker = db.clone();

        tokio::spawn(async move {
            let mut batch = Vec::with_capacity(batch_size);
            let mut interval = tokio::time::interval(flush_interval);

            loop {
                tokio::select! {
                    maybe_log = rx.recv() => {
                        match maybe_log {
                            Some(log) => {
                                batch.push(log);
                                if batch.len() >= batch_size {
                                    Self::flush_batch_internal(&db_worker, &batch).await;
                                    batch.clear();
                                }
                            }
                            None => {
                                if !batch.is_empty() {
                                    Self::flush_batch_internal(&db_worker, &batch).await;
                                    batch.clear();
                                }
                                break;
                            }
                        }
                    }
                    _ = interval.tick() => {
                        if !batch.is_empty() {
                            Self::flush_batch_internal(&db_worker, &batch).await;
                            batch.clear();
                        }
                    }
                }
            }
        });

        Self { db, sender: tx }
    }

    async fn flush_batch_internal(db: &PgPool, batch: &[ContractCallLog]) {
        for log in batch {
            let res = sqlx::query(
                "INSERT INTO contract_call_logs (contract_id, function_name, arguments, caller, status, gas_used, timestamp) \
                 VALUES ($1, $2, $3, $4, $5, $6, $7)"
            )
            .bind(&log.contract_id)
            .bind(&log.function_name)
            .bind(&log.arguments)
            .bind(&log.caller)
            .bind(&log.status)
            .bind(log.gas_used)
            .bind(log.timestamp.unwrap_or_else(chrono::Utc::now))
            .execute(db)
            .await;

            if let Err(e) = res {
                warn!(error = %e, contract_id = %log.contract_id, "Failed to flush contract call log to database");
            }
        }
    }

    /// Asynchronously enqueues contract invocation log into bounded buffer without blocking request thread.
    #[instrument(skip(self))]
    pub async fn log_call(&self, log: ContractCallLog) -> Result<(), AppError> {
        let contract_id = log.contract_id.clone();
        let function_name = log.function_name.clone();

        self.sender.send(log).await.map_err(|e| {
            AppError::InternalError(format!("Failed to buffer contract call log: {}", e))
        })?;

        info!(contract_id = %contract_id, function_name = %function_name, "Contract call buffered for batch flush");
        Ok(())
    }

    /// Non-blocking try_send for latency-critical paths where dropping/failing on full buffer is preferred
    pub fn try_log_call(&self, log: ContractCallLog) -> Result<(), AppError> {
        self.sender.try_send(log).map_err(|e| {
            AppError::InternalError(format!("Log channel buffer full or closed: {}", e))
        })
    }

    /// Direct synchronous database write for explicit synchronous workflows or tests
    #[instrument(skip(self))]
    pub async fn log_call_sync(&self, log: ContractCallLog) -> Result<(), AppError> {
        let db_span =
            TracingService::db_query_span("INSERT INTO contract_call_logs", "postgres", "INSERT");
        let _db_enter = db_span.enter();

        sqlx::query(
            "INSERT INTO contract_call_logs (contract_id, function_name, arguments, caller, status, gas_used, timestamp) \
             VALUES ($1, $2, $3, $4, $5, $6, $7)"
        )
        .bind(&log.contract_id)
        .bind(&log.function_name)
        .bind(&log.arguments)
        .bind(&log.caller)
        .bind(&log.status)
        .bind(log.gas_used)
        .bind(log.timestamp.unwrap_or_else(Utc::now))
        .execute(&self.db)
        .await
        .map_err(AppError::db)?;

        info!(contract_id = %log.contract_id, function_name = %log.function_name, "Contract call synchronously logged");
        Ok(())
    }

    #[instrument(skip(self))]
    pub async fn get_logs(
        &self,
        contract_id: Option<String>,
        limit: i64,
    ) -> Result<Vec<ContractCallLog>, AppError> {
        let logs = if let Some(cid) = contract_id {
            sqlx::query_as::<_, ContractCallLog>(
                "SELECT id, contract_id, function_name, arguments, caller, status, gas_used, timestamp \
                 FROM contract_call_logs \
                 WHERE contract_id = $1 \
                 ORDER BY timestamp DESC \
                 LIMIT $2"
            )
            .bind(cid)
            .bind(limit)
            .fetch_all(&self.db)
            .await
            .map_err(AppError::db)?
        } else {
            sqlx::query_as::<_, ContractCallLog>(
                "SELECT id, contract_id, function_name, arguments, caller, status, gas_used, timestamp \
                 FROM contract_call_logs \
                 ORDER BY timestamp DESC \
                 LIMIT $1"
            )
            .bind(limit)
            .fetch_all(&self.db)
            .await
            .map_err(AppError::db)?
        };

        Ok(logs)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::TimeZone;

    #[test]
    fn test_partition_table_name_formatting() {
        assert_eq!(
            ContractCallLogger::get_partition_table_name(2026, 5),
            "contract_call_logs_2026_05"
        );
        assert_eq!(
            ContractCallLogger::get_partition_table_name(2026, 12),
            "contract_call_logs_2026_12"
        );
    }

    #[test]
    fn test_generate_monthly_partition_ddl() {
        let ddl = ContractCallLogger::generate_monthly_partition_ddl(2026, 5);
        assert!(ddl.contains("contract_call_logs_2026_05 PARTITION OF contract_call_logs"));
        assert!(ddl.contains("FROM ('2026-05-01 00:00:00+00')"));
        assert!(ddl.contains("TO ('2026-06-01 00:00:00+00')"));

        // Year boundary transition test
        let dec_ddl = ContractCallLogger::generate_monthly_partition_ddl(2026, 12);
        assert!(dec_ddl.contains("contract_call_logs_2026_12 PARTITION OF contract_call_logs"));
        assert!(dec_ddl.contains("FROM ('2026-12-01 00:00:00+00')"));
        assert!(dec_ddl.contains("TO ('2027-01-01 00:00:00+00')"));
    }

    #[test]
    fn test_get_months_sequence() {
        let date = Utc.with_ymd_and_hms(2026, 11, 15, 0, 0, 0).unwrap();
        let seq = ContractCallLogger::get_months_sequence(date, 3);
        assert_eq!(seq, vec![(2026, 11), (2026, 12), (2027, 1), (2027, 2)]);
    }

    #[test]
    fn test_retention_policy_defaults() {
        let policy = LogRetentionPolicy::default();
        assert_eq!(policy.retention_days, 90);
        assert!(policy.auto_prune_enabled);
        assert_eq!(policy.precreate_months_ahead, 3);
    }

    #[tokio::test]
    async fn test_buffered_contract_call_logger_non_blocking_send() {
        let pool = sqlx::PgPool::connect_lazy("postgres://localhost/test").unwrap();
        let logger = ContractCallLogger::with_config(pool, 1000, 10, Duration::from_millis(50));

        let log = ContractCallLog {
            id: None,
            contract_id: "test_contract".to_string(),
            function_name: "test_fn".to_string(),
            arguments: serde_json::json!({}),
            caller: Some("caller1".to_string()),
            status: "success".to_string(),
            gas_used: 100.0,
            timestamp: Some(chrono::Utc::now()),
        };

        // Verifies non-blocking buffer enqueue without synchronous db stall
        let result = logger.log_call(log).await;
        assert!(result.is_ok());
    }
}
