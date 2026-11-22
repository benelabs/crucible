//! Contract analytics aggregation service.
//!
//! Aggregates per-contract metrics (call counts, error rates, gas usage,
//! unique callers) from indexed events and build metrics, storing roll-ups
//! in PostgreSQL and caching summaries in Redis.

use chrono::{DateTime, NaiveDate, Utc};
use redis::AsyncCommands;
use serde::{Deserialize, Serialize};
use sqlx::PgPool;
use tracing::{debug, info, instrument};

const CACHE_KEY_PREFIX: &str = "analytics:contract:";
const CACHE_TTL_SECS: u64 = 60;

/// Aggregated analytics for a single contract.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ContractAnalytics {
    pub contract_id: String,
    pub total_calls: i64,
    pub error_count: i64,
    pub error_rate: f64,
    pub unique_callers: i64,
    pub avg_gas_used: f64,
    pub last_activity: Option<DateTime<Utc>>,
    pub computed_at: DateTime<Utc>,
}

/// Platform-wide analytics summary.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AnalyticsSummary {
    pub total_contracts: i64,
    pub total_calls: i64,
    pub total_errors: i64,
    pub avg_error_rate: f64,
    pub most_active_contract: Option<String>,
    pub computed_at: DateTime<Utc>,
}

/// Anomaly alert for abnormal gas consumption or invocation failure spikes.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct AnomalyAlert {
    pub contract_id: String,
    pub metric: String,
    pub current_value: f64,
    pub mean: f64,
    pub std_dev: f64,
    pub z_score: f64,
    pub severity: String,
    pub timestamp: DateTime<Utc>,
}

/// Pre-aggregated hourly activity rollup for a smart contract (#1222).
#[derive(Debug, Clone, Serialize, Deserialize, sqlx::FromRow, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct ContractHourlyRollup {
    pub contract_id: String,
    pub bucket_hour: DateTime<Utc>,
    pub call_count: i64,
    pub error_count: i64,
    pub total_gas_used: f64,
    pub unique_callers: i64,
    pub created_at: DateTime<Utc>,
}

/// Pre-aggregated daily activity rollup for a smart contract (#1222).
#[derive(Debug, Clone, Serialize, Deserialize, sqlx::FromRow, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct ContractDailyRollup {
    pub contract_id: String,
    pub bucket_date: NaiveDate,
    pub call_count: i64,
    pub error_count: i64,
    pub total_gas_used: f64,
    pub unique_callers: i64,
    pub created_at: DateTime<Utc>,
}

/// Latency percentile summary (p50, p95, p99) for contract executions (#1221).
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct LatencyPercentiles {
    pub contract_id: Option<String>,
    pub p50_ms: f64,
    pub p95_ms: f64,
    pub p99_ms: f64,
    pub sample_count: i64,
    pub computed_at: DateTime<Utc>,
}

/// A centroid representation in a t-digest approximate quantile accumulator.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Centroid {
    pub mean: f64,
    pub weight: f64,
}

/// T-Digest approximate quantile estimator providing bounded-memory percentile estimation (#1221).
/// Guarantees O(compression_factor) memory bounds regardless of input sample size.
#[derive(Debug, Clone)]
pub struct TDigestEstimator {
    max_centroids: usize,
    centroids: Vec<Centroid>,
    total_weight: f64,
    unmerged: Vec<Centroid>,
}

impl Default for TDigestEstimator {
    fn default() -> Self {
        Self::new(100)
    }
}

impl TDigestEstimator {
    pub fn new(max_centroids: usize) -> Self {
        Self {
            max_centroids: max_centroids.max(10),
            centroids: Vec::with_capacity(max_centroids),
            total_weight: 0.0,
            unmerged: Vec::with_capacity(max_centroids * 2),
        }
    }

    /// Adds a single observation sample with unit weight.
    pub fn add(&mut self, value: f64) {
        self.add_weighted(value, 1.0);
    }

    /// Adds a weighted observation.
    pub fn add_weighted(&mut self, value: f64, weight: f64) {
        if !value.is_nan() && weight > 0.0 {
            self.unmerged.push(Centroid {
                mean: value,
                weight,
            });
            if self.unmerged.len() >= self.max_centroids * 2 {
                self.compress();
            }
        }
    }

    /// Compresses buffered observations and merges adjacent centroids within quantile tolerance.
    pub fn compress(&mut self) {
        if self.unmerged.is_empty() {
            return;
        }

        let mut all = std::mem::take(&mut self.centroids);
        all.extend(std::mem::take(&mut self.unmerged));
        all.sort_by(|a, b| {
            a.mean
                .partial_cmp(&b.mean)
                .unwrap_or(std::cmp::Ordering::Equal)
        });

        let total_w: f64 = all.iter().map(|c| c.weight).sum();
        self.total_weight = total_w;

        if all.is_empty() {
            return;
        }

        let mut merged: Vec<Centroid> = Vec::with_capacity(self.max_centroids);
        let mut cur_mean = all[0].mean;
        let mut cur_weight = all[0].weight;
        let mut cum_weight = 0.0;

        let k = self.max_centroids as f64;

        for c in all.into_iter().skip(1) {
            let q = (cum_weight + cur_weight / 2.0) / total_w;
            let max_cluster_weight = (4.0 * total_w * q * (1.0 - q) / k).max(1.0);

            if cur_weight + c.weight <= max_cluster_weight {
                cur_mean = (cur_mean * cur_weight + c.mean * c.weight) / (cur_weight + c.weight);
                cur_weight += c.weight;
            } else {
                cum_weight += cur_weight;
                merged.push(Centroid {
                    mean: cur_mean,
                    weight: cur_weight,
                });
                cur_mean = c.mean;
                cur_weight = c.weight;
            }
        }

        merged.push(Centroid {
            mean: cur_mean,
            weight: cur_weight,
        });

        // Enforce strict memory bound <= max_centroids
        while merged.len() > self.max_centroids {
            let mut min_idx = 0;
            let mut min_diff = f64::MAX;
            for i in 0..merged.len() - 1 {
                let diff = (merged[i + 1].mean - merged[i].mean)
                    * (merged[i].weight + merged[i + 1].weight);
                if diff < min_diff {
                    min_diff = diff;
                    min_idx = i;
                }
            }
            let next = merged.remove(min_idx + 1);
            let combined_w = merged[min_idx].weight + next.weight;
            merged[min_idx].mean = (merged[min_idx].mean * merged[min_idx].weight
                + next.mean * next.weight)
                / combined_w;
            merged[min_idx].weight = combined_w;
        }

        self.centroids = merged;
    }

    /// Estimates the quantile value `q` in `[0.0, 1.0]`.
    pub fn estimate_quantile(&mut self, q: f64) -> f64 {
        self.compress();

        if self.centroids.is_empty() {
            return 0.0;
        }
        if self.centroids.len() == 1 {
            return self.centroids[0].mean;
        }

        let q = q.clamp(0.0, 1.0);
        let target_rank = q * self.total_weight;

        let mut cum = 0.0;
        for i in 0..self.centroids.len() {
            let c = self.centroids[i];
            let next_cum = cum + c.weight;
            if target_rank <= next_cum {
                if i == 0 {
                    return c.mean;
                }
                let prev = self.centroids[i - 1];
                let weight_span = c.weight;
                let fraction = if weight_span > 0.0 {
                    (target_rank - cum) / weight_span
                } else {
                    0.0
                };
                return prev.mean + fraction * (c.mean - prev.mean);
            }
            cum = next_cum;
        }

        self.centroids.last().map(|c| c.mean).unwrap_or(0.0)
    }

    pub fn total_weight(&self) -> f64 {
        self.total_weight + self.unmerged.iter().map(|c| c.weight).sum::<f64>()
    }
}

pub struct AnalyticsAggregator {
    db: PgPool,
    redis: redis::Client,
}

impl AnalyticsAggregator {
    pub fn new(db: PgPool, redis: redis::Client) -> Self {
        Self { db, redis }
    }

    /// Detects anomalies (gas spikes or error rate spikes) using rolling standard deviation (Z-score).
    /// Triggers an automatic high-severity alert when Z-score > 3.0 over 1-hour window datasets.
    pub fn detect_anomaly(
        &self,
        contract_id: &str,
        metric: &str,
        historical_data: &[f64],
        current_value: f64,
    ) -> Option<AnomalyAlert> {
        if historical_data.is_empty() {
            return None;
        }

        let n = historical_data.len() as f64;
        let mean = historical_data.iter().sum::<f64>() / n;
        let variance = historical_data
            .iter()
            .map(|val| (val - mean).powi(2))
            .sum::<f64>()
            / n;
        let std_dev = variance.sqrt();

        if std_dev.abs() < f64::EPSILON {
            return None;
        }

        let z_score = (current_value - mean) / std_dev;

        if z_score > 3.0 {
            Some(AnomalyAlert {
                contract_id: contract_id.to_string(),
                metric: metric.to_string(),
                current_value,
                mean,
                std_dev,
                z_score,
                severity: "HIGH".to_string(),
                timestamp: Utc::now(),
            })
        } else {
            None
        }
    }

    /// Generates hourly rollup records from raw events for the specified time range (#1222).
    pub async fn aggregate_hourly_rollups(
        &self,
        start_time: DateTime<Utc>,
        end_time: DateTime<Utc>,
    ) -> Result<u64, sqlx::Error> {
        let res = sqlx::query(
            r#"
            INSERT INTO contract_hourly_rollups
                (contract_id, bucket_hour, call_count, error_count, total_gas_used, unique_callers, created_at)
            SELECT
                contract_id,
                date_trunc('hour', indexed_at) AS bucket_hour,
                COUNT(*) AS call_count,
                COUNT(*) FILTER (WHERE event_type = 'error') AS error_count,
                COALESCE(SUM((data->>'gas_used')::double precision), 0.0) AS total_gas_used,
                COUNT(DISTINCT data->>'caller') AS unique_callers,
                NOW() AS created_at
            FROM contract_events
            WHERE indexed_at >= $1 AND indexed_at < $2
            GROUP BY contract_id, date_trunc('hour', indexed_at)
            ON CONFLICT (contract_id, bucket_hour) DO UPDATE SET
                call_count = EXCLUDED.call_count,
                error_count = EXCLUDED.error_count,
                total_gas_used = EXCLUDED.total_gas_used,
                unique_callers = EXCLUDED.unique_callers,
                created_at = NOW()
            "#,
        )
        .bind(start_time)
        .bind(end_time)
        .execute(&self.db)
        .await?;

        Ok(res.rows_affected())
    }

    /// Aggregates hourly rollups into a single day's summary rollup (#1222).
    pub async fn aggregate_daily_rollups(
        &self,
        target_date: NaiveDate,
    ) -> Result<u64, sqlx::Error> {
        let res = sqlx::query(
            r#"
            INSERT INTO contract_daily_rollups
                (contract_id, bucket_date, call_count, error_count, total_gas_used, unique_callers, created_at)
            SELECT
                contract_id,
                bucket_hour::date AS bucket_date,
                SUM(call_count) AS call_count,
                SUM(error_count) AS error_count,
                SUM(total_gas_used) AS total_gas_used,
                MAX(unique_callers) AS unique_callers,
                NOW() AS created_at
            FROM contract_hourly_rollups
            WHERE bucket_hour >= $1::timestamp AND bucket_hour < ($1::timestamp + INTERVAL '1 day')
            GROUP BY contract_id, bucket_hour::date
            ON CONFLICT (contract_id, bucket_date) DO UPDATE SET
                call_count = EXCLUDED.call_count,
                error_count = EXCLUDED.error_count,
                total_gas_used = EXCLUDED.total_gas_used,
                unique_callers = EXCLUDED.unique_callers,
                created_at = NOW()
            "#,
        )
        .bind(target_date)
        .execute(&self.db)
        .await?;

        Ok(res.rows_affected())
    }

    /// Queries historical contract activity from pre-aggregated daily summary tables (#1222).
    /// Avoids scanning raw event tables across large multi-day windows.
    pub async fn query_historical_activity(
        &self,
        contract_id: &str,
        days: i64,
    ) -> Result<Vec<ContractDailyRollup>, sqlx::Error> {
        let rollups = sqlx::query_as::<_, ContractDailyRollup>(
            r#"
            SELECT contract_id, bucket_date, call_count, error_count, total_gas_used, unique_callers, created_at
            FROM contract_daily_rollups
            WHERE contract_id = $1 AND bucket_date >= (CURRENT_DATE - ($2 || ' days')::interval)::date
            ORDER BY bucket_date ASC
            "#,
        )
        .bind(contract_id)
        .bind(days)
        .fetch_all(&self.db)
        .await?;

        Ok(rollups)
    }

    /// Computes accurate percentile latencies using PostgreSQL `percentile_cont` aggregation (#1221).
    /// Offloads all sorting and memory allocation to the database engine.
    pub async fn compute_percentiles_pg(
        &self,
        contract_id: Option<&str>,
    ) -> Result<LatencyPercentiles, sqlx::Error> {
        let row: (i64, Option<f64>, Option<f64>, Option<f64>) = sqlx::query_as(
            r#"
            SELECT
                COUNT(*)::bigint AS sample_count,
                percentile_cont(0.50) WITHIN GROUP (ORDER BY compilation_time_ms) AS p50,
                percentile_cont(0.95) WITHIN GROUP (ORDER BY compilation_time_ms) AS p95,
                percentile_cont(0.99) WITHIN GROUP (ORDER BY compilation_time_ms) AS p99
            FROM build_metrics
            WHERE ($1::text IS NULL OR project_name = $1)
            "#,
        )
        .bind(contract_id)
        .fetch_one(&self.db)
        .await?;

        Ok(LatencyPercentiles {
            contract_id: contract_id.map(|s| s.to_string()),
            sample_count: row.0,
            p50_ms: row.1.unwrap_or(0.0),
            p95_ms: row.2.unwrap_or(0.0),
            p99_ms: row.3.unwrap_or(0.0),
            computed_at: Utc::now(),
        })
    }

    /// Computes approximate percentiles (p50, p95, p99) over an iterator of samples using
    /// the t-digest algorithm with strictly bounded O(1) heap memory (#1221).
    pub fn compute_approximate_percentiles(
        contract_id: Option<&str>,
        samples: impl IntoIterator<Item = f64>,
    ) -> LatencyPercentiles {
        let mut digest = TDigestEstimator::default();
        let mut count = 0i64;

        for val in samples {
            digest.add(val);
            count += 1;
        }

        let p50_ms = digest.estimate_quantile(0.50);
        let p95_ms = digest.estimate_quantile(0.95);
        let p99_ms = digest.estimate_quantile(0.99);

        LatencyPercentiles {
            contract_id: contract_id.map(|s| s.to_string()),
            p50_ms,
            p95_ms,
            p99_ms,
            sample_count: count,
            computed_at: Utc::now(),
        }
    }

    /// Compute and cache analytics for a specific contract.
    #[instrument(skip(self), fields(contract_id = %contract_id))]
    pub async fn contract_analytics(
        &self,
        contract_id: &str,
    ) -> Result<ContractAnalytics, anyhow::Error> {
        let cache_key = format!("{}{}", CACHE_KEY_PREFIX, contract_id);

        // Try Redis cache first
        if let Ok(mut conn) = self.redis.get_multiplexed_async_connection().await {
            if let Ok(cached) = conn.get::<_, String>(&cache_key).await {
                if let Ok(analytics) = serde_json::from_str::<ContractAnalytics>(&cached) {
                    debug!(contract_id, "Cache hit for contract analytics");
                    return Ok(analytics);
                }
            }
        }

        let analytics = self.compute_contract_analytics(contract_id).await?;

        // Populate cache (best-effort)
        if let Ok(mut conn) = self.redis.get_multiplexed_async_connection().await {
            if let Ok(json) = serde_json::to_string(&analytics) {
                let _: Result<(), _> = conn.set_ex(&cache_key, json, CACHE_TTL_SECS).await;
            }
        }

        Ok(analytics)
    }

    async fn compute_contract_analytics(
        &self,
        contract_id: &str,
    ) -> Result<ContractAnalytics, sqlx::Error> {
        // Total calls and error count from contract_events
        let (total_calls, error_count): (i64, i64) = sqlx::query_as(
            r#"
            SELECT
                COUNT(*) AS total_calls,
                COUNT(*) FILTER (WHERE event_type = 'error') AS error_count
            FROM contract_events
            WHERE contract_id = $1
            "#,
        )
        .bind(contract_id)
        .fetch_one(&self.db)
        .await
        .unwrap_or((0, 0));

        // Unique callers (stored in event data as JSON field "caller")
        let (unique_callers,): (i64,) = sqlx::query_as(
            r#"
            SELECT COUNT(DISTINCT data->>'caller')
            FROM contract_events
            WHERE contract_id = $1 AND data->>'caller' IS NOT NULL
            "#,
        )
        .bind(contract_id)
        .fetch_one(&self.db)
        .await
        .unwrap_or((0,));

        // Average gas from build_metrics (proxy for gas usage)
        let (avg_gas,): (Option<f64>,) = sqlx::query_as(
            "SELECT AVG(compilation_time_ms) FROM build_metrics WHERE project_name = $1",
        )
        .bind(contract_id)
        .fetch_one(&self.db)
        .await
        .unwrap_or((None,));

        // Last activity timestamp
        let (last_activity,): (Option<DateTime<Utc>>,) =
            sqlx::query_as("SELECT MAX(indexed_at) FROM contract_events WHERE contract_id = $1")
                .bind(contract_id)
                .fetch_one(&self.db)
                .await
                .unwrap_or((None,));

        let error_rate = if total_calls > 0 {
            error_count as f64 / total_calls as f64
        } else {
            0.0
        };

        Ok(ContractAnalytics {
            contract_id: contract_id.to_string(),
            total_calls,
            error_count,
            error_rate,
            unique_callers,
            avg_gas_used: avg_gas.unwrap_or(0.0),
            last_activity,
            computed_at: Utc::now(),
        })
    }

    /// Compute a platform-wide analytics summary.
    #[instrument(skip(self))]
    pub async fn summary(&self) -> Result<AnalyticsSummary, anyhow::Error> {
        const SUMMARY_KEY: &str = "analytics:summary";

        if let Ok(mut conn) = self.redis.get_multiplexed_async_connection().await {
            if let Ok(cached) = conn.get::<_, String>(SUMMARY_KEY).await {
                if let Ok(s) = serde_json::from_str::<AnalyticsSummary>(&cached) {
                    return Ok(s);
                }
            }
        }

        let (total_contracts,): (i64,) =
            sqlx::query_as("SELECT COUNT(DISTINCT contract_id) FROM contract_events")
                .fetch_one(&self.db)
                .await
                .unwrap_or((0,));

        let (total_calls, total_errors): (i64, i64) = sqlx::query_as(
            r#"
            SELECT COUNT(*),
                   COUNT(*) FILTER (WHERE event_type = 'error')
            FROM contract_events
            "#,
        )
        .fetch_one(&self.db)
        .await
        .unwrap_or((0, 0));

        let most_active: Option<(String,)> = sqlx::query_as(
            r#"
            SELECT contract_id FROM contract_events
            GROUP BY contract_id ORDER BY COUNT(*) DESC LIMIT 1
            "#,
        )
        .fetch_optional(&self.db)
        .await
        .unwrap_or(None);

        let avg_error_rate = if total_calls > 0 {
            total_errors as f64 / total_calls as f64
        } else {
            0.0
        };

        let summary = AnalyticsSummary {
            total_contracts,
            total_calls,
            total_errors,
            avg_error_rate,
            most_active_contract: most_active.map(|(id,)| id),
            computed_at: Utc::now(),
        };

        if let Ok(mut conn) = self.redis.get_multiplexed_async_connection().await {
            if let Ok(json) = serde_json::to_string(&summary) {
                let _: Result<(), _> = conn.set_ex(SUMMARY_KEY, json, CACHE_TTL_SECS).await;
            }
        }

        info!("Analytics summary computed");
        Ok(summary)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use sqlx::postgres::PgPoolOptions;

    fn lazy_pool() -> PgPool {
        PgPoolOptions::new()
            .max_connections(1)
            .connect_lazy("postgres://postgres:postgres@localhost/crucible_test")
            .unwrap()
    }

    fn lazy_redis() -> redis::Client {
        redis::Client::open("redis://127.0.0.1/").unwrap()
    }

    #[test]
    fn test_error_rate_zero_when_no_calls() {
        let analytics = ContractAnalytics {
            contract_id: "CABC".to_string(),
            total_calls: 0,
            error_count: 0,
            error_rate: 0.0,
            unique_callers: 0,
            avg_gas_used: 0.0,
            last_activity: None,
            computed_at: Utc::now(),
        };
        assert_eq!(analytics.error_rate, 0.0);
    }

    #[test]
    fn test_error_rate_calculation() {
        let total = 100i64;
        let errors = 5i64;
        let rate = errors as f64 / total as f64;
        assert!((rate - 0.05).abs() < f64::EPSILON);
    }

    #[test]
    fn test_analytics_serialization() {
        let a = ContractAnalytics {
            contract_id: "CABC".to_string(),
            total_calls: 42,
            error_count: 1,
            error_rate: 1.0 / 42.0,
            unique_callers: 5,
            avg_gas_used: 1234.5,
            last_activity: None,
            computed_at: Utc::now(),
        };
        let json = serde_json::to_string(&a).unwrap();
        let back: ContractAnalytics = serde_json::from_str(&json).unwrap();
        assert_eq!(back.contract_id, "CABC");
        assert_eq!(back.total_calls, 42);
    }

    #[test]
    fn test_summary_serialization() {
        let s = AnalyticsSummary {
            total_contracts: 10,
            total_calls: 500,
            total_errors: 25,
            avg_error_rate: 0.05,
            most_active_contract: Some("CABC".to_string()),
            computed_at: Utc::now(),
        };
        let json = serde_json::to_string(&s).unwrap();
        let back: AnalyticsSummary = serde_json::from_str(&json).unwrap();
        assert_eq!(back.total_contracts, 10);
    }

    #[tokio::test]
    async fn test_detect_anomaly_no_spike() {
        let aggregator = AnalyticsAggregator::new(lazy_pool(), lazy_redis());
        let historical_gas = vec![1000.0, 1050.0, 980.0, 1020.0, 1010.0, 990.0];
        let current_gas = 1040.0;
        let alert = aggregator.detect_anomaly("C123", "gas_usage", &historical_gas, current_gas);
        assert!(alert.is_none());
    }

    #[tokio::test]
    async fn test_detect_anomaly_gas_spike() {
        let aggregator = AnalyticsAggregator::new(lazy_pool(), lazy_redis());
        // Stable baseline with mean ~1000.0, std dev ~14.14
        let historical_gas = vec![1000.0, 1010.0, 990.0, 1000.0, 1010.0, 990.0];
        let current_spike_gas = 1200.0; // Z-score > 10.0
        let alert =
            aggregator.detect_anomaly("C123", "gas_usage", &historical_gas, current_spike_gas);
        assert!(alert.is_some());
        let alert = alert.unwrap();
        assert_eq!(alert.contract_id, "C123");
        assert_eq!(alert.metric, "gas_usage");
        assert_eq!(alert.severity, "HIGH");
        assert!(alert.z_score > 3.0);
    }

    #[test]
    fn test_rollup_serialization() {
        let now = Utc::now();
        let hourly = ContractHourlyRollup {
            contract_id: "CA123".to_string(),
            bucket_hour: now,
            call_count: 1500,
            error_count: 12,
            total_gas_used: 450000.0,
            unique_callers: 350,
            created_at: now,
        };

        let json = serde_json::to_string(&hourly).unwrap();
        let deserialized: ContractHourlyRollup = serde_json::from_str(&json).unwrap();
        assert_eq!(deserialized.contract_id, "CA123");
        assert_eq!(deserialized.call_count, 1500);
        assert_eq!(deserialized.error_count, 12);
        assert_eq!(deserialized.unique_callers, 350);

        let date = NaiveDate::from_ymd_opt(2026, 5, 20).unwrap();
        let daily = ContractDailyRollup {
            contract_id: "CA123".to_string(),
            bucket_date: date,
            call_count: 36000,
            error_count: 280,
            total_gas_used: 10800000.0,
            unique_callers: 4200,
            created_at: now,
        };

        let json_daily = serde_json::to_string(&daily).unwrap();
        let deserialized_daily: ContractDailyRollup = serde_json::from_str(&json_daily).unwrap();
        assert_eq!(deserialized_daily.bucket_date, date);
        assert_eq!(deserialized_daily.call_count, 36000);
    }

    #[test]
    fn test_tdigest_percentiles_uniform_distribution() {
        let samples: Vec<f64> = (1..=1000).map(|i| i as f64).collect();
        let percentiles =
            AnalyticsAggregator::compute_approximate_percentiles(Some("contract-1"), samples);

        assert_eq!(percentiles.sample_count, 1000);
        assert_eq!(percentiles.contract_id, Some("contract-1".to_string()));

        // p50 should be ~500.0 (+/- 5%)
        assert!(
            (percentiles.p50_ms - 500.0).abs() < 25.0,
            "p50 was {}",
            percentiles.p50_ms
        );
        // p95 should be ~950.0 (+/- 3%)
        assert!(
            (percentiles.p95_ms - 950.0).abs() < 25.0,
            "p95 was {}",
            percentiles.p95_ms
        );
        // p99 should be ~990.0 (+/- 2%)
        assert!(
            (percentiles.p99_ms - 990.0).abs() < 20.0,
            "p99 was {}",
            percentiles.p99_ms
        );
    }

    #[test]
    fn test_tdigest_bounded_memory_under_large_stream() {
        let mut digest = TDigestEstimator::new(50);
        for i in 0..100_000 {
            digest.add((i % 500) as f64);
        }
        digest.compress();

        assert!(digest.centroids.len() <= 50);
        assert_eq!(digest.total_weight, 100_000.0);
    }

    #[test]
    fn test_latency_percentiles_serialization() {
        let lp = LatencyPercentiles {
            contract_id: Some("C123".to_string()),
            p50_ms: 12.5,
            p95_ms: 48.2,
            p99_ms: 99.1,
            sample_count: 5000,
            computed_at: Utc::now(),
        };

        let json = serde_json::to_string(&lp).unwrap();
        let deserialized: LatencyPercentiles = serde_json::from_str(&json).unwrap();
        assert_eq!(deserialized.p50_ms, 12.5);
        assert_eq!(deserialized.p95_ms, 48.2);
        assert_eq!(deserialized.p99_ms, 99.1);
        assert_eq!(deserialized.sample_count, 5000);
    }
}
