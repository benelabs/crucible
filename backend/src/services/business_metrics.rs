//! Business metrics service for tracking revenue, costs, and operational KPIs.

#![allow(dead_code)]

use std::collections::HashMap;
use std::sync::Arc;

use chrono::{DateTime, Duration, Utc};
use rust_decimal::Decimal;
use serde::{Deserialize, Serialize};
use sqlx::{PgPool, Row};
use tokio::sync::RwLock;
use tracing::{error, info, instrument};
use uuid::Uuid;
use utoipa::ToSchema;
use sqlx::FromRow;

use crate::error::AppError;

// ─── Domain Types ────────────────────────────────────────────────────────────

#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
pub struct BusinessMetric {
    pub id: Uuid,
    pub name: String,
    #[schema(value_type = f64)]
    pub value: Decimal,
    pub unit: String,
    pub category: MetricCategory,
    pub tags: HashMap<String, String>,
    pub recorded_at: DateTime<Utc>,
    pub source: MetricSource,
}
// ---------------------------------------------------------------------------
// Domain types

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, ToSchema)]
#[serde(rename_all = "snake_case")]
pub enum MetricCategory {
    Revenue,
    Costs,
    Users,
    Transactions,
    Performance,
    Custom(String),
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Default, ToSchema)]
#[serde(rename_all = "snake_case")]
pub enum MetricSource {
    OnChain,
    OffChain,
    Database,
    ExternalApi,
    #[default]
    Manual,
}

impl MetricSource {
    pub fn as_str(&self) -> String {
        match self {
            Self::OnChain => "on_chain".to_string(),
            Self::OffChain => "off_chain".to_string(),
            Self::Database => "database".to_string(),
            Self::ExternalApi => "external_api".to_string(),
            Self::Manual => "manual".to_string(),
        }
    }

    pub fn from_str(s: &str) -> Self {
        match s {
            "on_chain" => Self::OnChain,
            "off_chain" => Self::OffChain,
            "database" => Self::Database,
            "external_api" => Self::ExternalApi,
            _ => Self::Manual,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MetricsQuery {
    pub category: Option<MetricCategory>,
    pub from: Option<DateTime<Utc>>,
    pub to: Option<DateTime<Utc>>,
    pub tags: Option<HashMap<String, String>>,
    pub limit: Option<i64>,
    pub offset: Option<i64>,
}

#[derive(Debug, Clone, Serialize, Deserialize, utoipa::ToSchema)]
pub struct MetricsSummary {
    pub total_metrics: i64,
    pub categories: std::collections::HashMap<String, i64>,
    pub latest_timestamp: Option<DateTime<Utc>>,
}

// ─── Service ─────────────────────────────────────────────────────────────────
// ---------------------------------------------------------------------------
// Service

pub struct BusinessMetricsService {
    db: PgPool,
    cache: Arc<RwLock<HashMap<String, Vec<BusinessMetric>>>>,
}

impl BusinessMetricsService {
    pub fn new(db: PgPool) -> Self {
        Self {
            db,
            cache: Arc::new(RwLock::new(HashMap::new())),
        }
    }

    /// Record a new business metric with the given parameters.
    #[instrument(skip_all, fields(metric_name))]
    /// Record a new business metric.
    #[instrument(skip(self, tags, value, unit, category, source))]
    pub async fn record_metric(
        &self,
        name: String,
        value: Decimal,
        unit: String,
        category: MetricCategory,
        tags: HashMap<String, String>,
        source: MetricSource,
    ) -> Result<BusinessMetric, AppError> {
        let id = Uuid::new_v4();
        let now = Utc::now();
        let name: String = name.into();
        let unit: String = unit.into();

        tracing::Span::current().record("metric_name", &name.as_str());

        let category_str = match &category {
            MetricCategory::Revenue => "revenue".to_string(),
            MetricCategory::Costs => "costs".to_string(),
            MetricCategory::Users => "users".to_string(),
            MetricCategory::Transactions => "transactions".to_string(),
            MetricCategory::Performance => "performance".to_string(),
            MetricCategory::Custom(s) => format!("custom:{}", s),
        };

        let source_str = match &source {
            MetricSource::OnChain => "on_chain",
            MetricSource::OffChain => "off_chain",
            MetricSource::Database => "database",
            MetricSource::ExternalApi => "external_api",
            MetricSource::Manual => "manual",
        };

        let tags_json = serde_json::to_value(&tags)
            .map_err(|e| AppError::InternalError(e.to_string()))?;

        let category_str = serde_json::to_string(&category)
            .map_err(|e| AppError::InternalError(e.to_string()))?;
        let source_str = serde_json::to_string(&source)
            .map_err(|e| AppError::InternalError(e.to_string()))?;
        // Store Decimal as string to avoid sqlx type issues
        let value_str = value.to_string();

        sqlx::query(
            r#"
            INSERT INTO business_metrics (id, name, value, unit, category, tags, recorded_at, source)
            VALUES ($1, $2, $3, $4, $5, $6, $7, $8)
            "#,
        )
        .bind(id)
        .bind(&name)
        .bind(&value_str)
        .bind(&unit)
        .bind(&category_str)
        .bind(&tags_json)
        .bind(now)
        .bind(&source_str)
        .execute(&self.db)
        .await
        .map_err(|e| {
            error!(error = %e, "Failed to record metric");
            AppError::Database(e)
        })?;

        let metric = BusinessMetric {
            id,
            name: name.clone(),
            value,
            unit,
            category,
            tags,
            recorded_at: now,
            source,
        };

        // Update in-memory cache
        {
            let mut cache = self.cache.write().await;
            let entry = cache.entry(metric.name.clone()).or_default();
            entry.push(metric.clone());
            if entry.len() > 1000 {
                entry.remove(0);
            }
        }

        info!(
            metric_name = %metric.name,
            value = %metric.value,
            "Recorded business metric"
        );

        Ok(metric)
    }

    /// Record multiple metrics in a single transaction.
    #[instrument(skip(self, metrics))]
    pub async fn record_metrics_batch(
        &self,
        metrics: Vec<(
            String,
            Decimal,
            String,
            MetricCategory,
            HashMap<String, String>,
            MetricSource,
        )>,
    ) -> Result<Vec<BusinessMetric>, AppError> {
        let mut tx = self.db.begin().await?;
        let mut results = Vec::with_capacity(metrics.len());
        let now = Utc::now();

        for (name, value, unit, category, tags, source) in metrics {
            let id = Uuid::new_v4();

            let category_str = match &category {
                MetricCategory::Revenue => "revenue".to_string(),
                MetricCategory::Costs => "costs".to_string(),
                MetricCategory::Users => "users".to_string(),
                MetricCategory::Transactions => "transactions".to_string(),
                MetricCategory::Performance => "performance".to_string(),
                MetricCategory::Custom(s) => format!("custom:{}", s),
            };

            let source_str = match &source {
                MetricSource::OnChain => "on_chain",
                MetricSource::OffChain => "off_chain",
                MetricSource::Database => "database",
                MetricSource::ExternalApi => "external_api",
                MetricSource::Manual => "manual",
            };

            let tags_json = serde_json::to_value(&tags)
                .map_err(|e| AppError::InternalError(e.to_string()))?;

            sqlx::query(
                r#"
                INSERT INTO business_metrics (id, name, value, unit, category, tags, recorded_at, source)
                VALUES ($1, $2, $3, $4, $5, $6, $7, $8)
                "#,
            )
            .bind(id)
            .bind(&name)
            .bind(value)
            .bind(&unit)
            .bind(&category_str)
            .bind(&tags_json)
            .bind(now)
            .bind(source_str)
            .execute(&mut *tx)
            .await
            .map_err(|e| {
                error!(error = %e, "Failed in batch metric insert");
                AppError::Database(e)
            })?;

            results.push(BusinessMetric {
                id,
                name,
                value,
                unit,
                category,
                tags,
                recorded_at: now,
                source,
            });
        }

        tx.commit().await.map_err(|e| {
            error!(error = %e, "Failed to commit batch metrics");
            AppError::Database(e)
        })?;

        info!(count = results.len(), "Recorded batch metrics");
        Ok(results)
    }

    /// Query metrics with optional filters.
    #[instrument(skip(self))]
    pub async fn query_metrics(
        &self,
        query: MetricsQuery,
    ) -> Result<(Vec<BusinessMetric>, i64), AppError> {
        let limit = query.limit.unwrap_or(100);
        let offset = query.offset.unwrap_or(0);

        let total: i64 = sqlx::query_scalar(
            "SELECT COUNT(*) FROM business_metrics"
        )
        .fetch_one(&self.db)
        .await
        .map_err(AppError::Database)?;

        // Build metrics from raw rows
        let rows: Vec<(Uuid, String, Decimal, String, String, serde_json::Value, DateTime<Utc>, String)> =
            sqlx::query_as(
                "SELECT id, name, value, unit, category, tags, recorded_at, source \
                 FROM business_metrics ORDER BY recorded_at DESC LIMIT $1 OFFSET $2",
            )
            .bind(limit)
            .bind(offset)
            .fetch_all(&self.db)
            .await
            .map_err(AppError::Database)?;

        let metrics = rows
            .into_iter()
            .map(|(id, name, value, unit, category_str, tags_val, recorded_at, source_str)| {
                let category = parse_category(&category_str);
                let source = parse_source(&source_str);
                let tags: HashMap<String, String> =
                    serde_json::from_value(tags_val).unwrap_or_default();
                BusinessMetric { id, name, value, unit, category, tags, recorded_at, source }
            })
            .collect();

        Ok((metrics, total))
    }

    /// Get aggregated metrics summary.
    #[instrument(skip(self))]
    pub async fn get_metrics_summary(&self) -> Result<MetricsSummary, AppError> {
        let total: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM business_metrics")
            .fetch_one(&self.db)
            .await
            .map_err(AppError::Database)?;

        let latest: Option<DateTime<Utc>> =
            sqlx::query_scalar("SELECT MAX(recorded_at) FROM business_metrics")
                .fetch_one(&self.db)
                .await
                .map_err(AppError::Database)?;

        let cat_rows: Vec<(String, i64)> =
            sqlx::query_as("SELECT category, COUNT(*) FROM business_metrics GROUP BY category")
                .fetch_all(&self.db)
                .await
                .map_err(AppError::Database)?;

        let mut categories = HashMap::new();
        for (cat_str, count) in cat_rows {
            categories.insert(cat_str, count);
        }

        Ok(MetricsSummary {
            total_metrics: total,
            categories,
            latest_timestamp: latest,
        })
    }

    /// Compute aggregated values for a metric over a time range.
    #[instrument(skip(self))]
    pub async fn aggregate_metric(
        &self,
        name: &str,
        from: DateTime<Utc>,
        to: DateTime<Utc>,
    ) -> Result<Option<Decimal>, AppError> {
        let result: Option<Decimal> = sqlx::query_scalar(
            "SELECT SUM(value) FROM business_metrics WHERE name = $1 AND recorded_at >= $2 AND recorded_at <= $3",
        )
        .bind(name)
        .bind(from)
        .bind(to)
        .fetch_one(&self.db)
        .await
        .map_err(AppError::Database)?;

        Ok(result)
    }

    /// Get the latest value for a specific metric.
    #[instrument(skip(self))]
    pub async fn get_latest_metric(&self, name: &str) -> Result<Option<BusinessMetric>, AppError> {
        // Check cache first
        {
            let cache = self.cache.read().await;
            if let Some(values) = cache.get(name) {
                if let Some(latest) = values.last() {
                    return Ok(Some(latest.clone()));
                }
            }
        }

        // Fall back to database
        let row: Option<(Uuid, String, Decimal, String, String, serde_json::Value, DateTime<Utc>, String)> =
            sqlx::query_as(
                "SELECT id, name, value, unit, category, tags, recorded_at, source \
                 FROM business_metrics WHERE name = $1 ORDER BY recorded_at DESC LIMIT 1",
            )
            .bind(name)
            .fetch_optional(&self.db)
            .await
            .map_err(AppError::Database)?;

        let metric = row.map(|(id, name, value, unit, category_str, tags_val, recorded_at, source_str)| {
            let category = parse_category(&category_str);
            let source = parse_source(&source_str);
            let tags: HashMap<String, String> =
                serde_json::from_value(tags_val).unwrap_or_default();
            BusinessMetric { id, name, value, unit, category, tags, recorded_at, source }
        });

        Ok(metric)
    }

    /// Remove metrics older than the retention period.
    #[instrument(skip(self))]
    pub async fn prune_old_metrics(&self, retention_days: i64) -> Result<u64, AppError> {
        let cutoff = Utc::now() - Duration::days(retention_days);

        let _deleted = sqlx::query(
            "DELETE FROM business_metrics WHERE recorded_at < $1",
        )
        .bind(cutoff)
        .execute(&self.db)
        .await
        .map_err(AppError::Database)?
        .rows_affected();
        let result = sqlx::query("DELETE FROM business_metrics WHERE recorded_at < $1")
            .bind(cutoff)
            .execute(&self.db)
            .await
            .map_err(AppError::Database)?;

        let deleted = result.rows_affected();
        info!(deleted, retention_days, "Pruned old metrics");
        Ok(deleted)
    }

    /// Get the latest cached value for a metric (no DB call).
    pub async fn get_cached_latest(&self, name: &str) -> Option<BusinessMetric> {
        let cache = self.cache.read().await;
        cache.get(name)?.last().cloned()
    }
}

// ─── Parsing helpers ─────────────────────────────────────────────────────────

fn parse_category(s: &str) -> MetricCategory {
    match s {
        "revenue" => MetricCategory::Revenue,
        "costs" => MetricCategory::Costs,
        "users" => MetricCategory::Users,
        "transactions" => MetricCategory::Transactions,
        "performance" => MetricCategory::Performance,
        other => MetricCategory::Custom(
            other.strip_prefix("custom:").unwrap_or(other).to_string(),
        ),
    }
}

fn parse_source(s: &str) -> MetricSource {
    match s {
        "on_chain" => MetricSource::OnChain,
        "off_chain" => MetricSource::OffChain,
        "database" => MetricSource::Database,
        "external_api" => MetricSource::ExternalApi,
        _ => MetricSource::Manual,
    }
}

// ─── API Handlers ────────────────────────────────────────────────────────────

use axum::{extract::State, http::StatusCode, Json};

pub struct MetricsState {
    pub service: Arc<BusinessMetricsService>,
}

#[derive(Debug, Deserialize, ToSchema)]
pub struct RecordMetricRequest {
    pub name: String,
    #[schema(value_type = f64)]
    pub value: Decimal,
    pub unit: String,
    pub category: MetricCategory,
    #[serde(default)]
    pub tags: HashMap<String, String>,
    #[serde(default)]
    pub source: MetricSource,
}

/// POST /api/metrics — Record a new business metric.
#[utoipa::path(
    post,
    path = "/api/metrics",
    request_body = RecordMetricRequest,
    responses(
        (status = 201, description = "Metric recorded", body = BusinessMetric),
        (status = 400, description = "Invalid request"),
        (status = 500, description = "Internal server error")
    )
)]
pub async fn record_metric(
    State(state): State<Arc<MetricsState>>,
    Json(req): Json<RecordMetricRequest>,
) -> Result<(StatusCode, Json<BusinessMetric>), AppError> {
    let metric = state
        .service
        .record_metric(
            req.name,
            req.value,
            req.unit,
            req.category,
            req.tags,
            req.source,
        )
        .await?;

    Ok((StatusCode::CREATED, Json(metric)))
}

/// GET /api/metrics — Query business metrics with filters.
#[utoipa::path(
    get,
    path = "/api/metrics",
    params(
        ("category" = Option<MetricCategory>, Query, description = "Filter by category"),
        ("from" = Option<String>, Query, description = "Start of time range (ISO 8601)"),
        ("to" = Option<String>, Query, description = "End of time range (ISO 8601)"),
        ("limit" = Option<i64>, Query, description = "Max results"),
        ("offset" = Option<i64>, Query, description = "Pagination offset")
    ),
    responses(
        (status = 200, description = "List of metrics with total count"),
        (status = 500, description = "Internal server error")
    )
)]
pub async fn query_metrics(
    State(state): State<Arc<MetricsState>>,
    axum::extract::Query(params): axum::extract::Query<HashMap<String, String>>,
) -> Result<Json<serde_json::Value>, AppError> {
    let category = params
        .get("category")
        .and_then(|c| serde_json::from_str(&format!("\"{}\"", c)).ok());

    let from = params
        .get("from")
        .and_then(|v| v.parse::<DateTime<Utc>>().ok());
    let to = params
        .get("to")
        .and_then(|v| v.parse::<DateTime<Utc>>().ok());
    let limit = params.get("limit").and_then(|v| v.parse::<i64>().ok());
    let offset = params.get("offset").and_then(|v| v.parse::<i64>().ok());

    let query = MetricsQuery {
        category,
        from,
        to,
        tags: None,
        limit,
        offset,
    };

    let (metrics, total) = state.service.query_metrics(query).await?;

    Ok(Json(serde_json::json!({
        "metrics": metrics,
        "total": total,
    })))
}

/// GET /api/metrics/summary — Get aggregated metrics overview.
#[utoipa::path(
    get,
    path = "/api/metrics/summary",
    responses(
        (status = 200, description = "Metrics summary", body = MetricsSummary),
        (status = 500, description = "Internal server error")
    )
)]
pub async fn get_metrics_summary(
    State(state): State<Arc<MetricsState>>,
) -> Result<Json<MetricsSummary>, AppError> {
    let summary = state.service.get_metrics_summary().await?;
    Ok(Json(summary))
}
// ---------------------------------------------------------------------------
// Tests

#[cfg(test)]
mod tests {
    use super::*;
    use rust_decimal::Decimal;

    #[test]
    fn test_metric_source_default() {
        let s: MetricSource = Default::default();
        assert_eq!(s, MetricSource::Manual);
    }

    #[test]
    fn test_parse_category_known() {
        assert_eq!(parse_category("revenue"), MetricCategory::Revenue);
        assert_eq!(parse_category("costs"), MetricCategory::Costs);
        assert_eq!(parse_category("users"), MetricCategory::Users);
        assert_eq!(parse_category("transactions"), MetricCategory::Transactions);
        assert_eq!(parse_category("performance"), MetricCategory::Performance);
    }

    #[test]
    fn test_parse_category_custom() {
        assert_eq!(
            parse_category("custom:special"),
            MetricCategory::Custom("special".to_string())
        );
        assert_eq!(
            parse_category("unknown"),
            MetricCategory::Custom("unknown".to_string())
        );
    }

    #[test]
    fn test_parse_source_all_variants() {
        assert_eq!(parse_source("on_chain"), MetricSource::OnChain);
        assert_eq!(parse_source("off_chain"), MetricSource::OffChain);
        assert_eq!(parse_source("database"), MetricSource::Database);
        assert_eq!(parse_source("external_api"), MetricSource::ExternalApi);
        assert_eq!(parse_source("manual"), MetricSource::Manual);
        assert_eq!(parse_source("unknown"), MetricSource::Manual);
    }

    #[test]
    fn test_business_metric_serialization() {
        let metric = BusinessMetric {
            id: Uuid::new_v4(),
            name: "test_revenue".to_string(),
            value: Decimal::new(1000, 2),
            unit: "USD".to_string(),
            category: MetricCategory::Revenue,
            tags: HashMap::from([("region".to_string(), "us-east".to_string())]),
            recorded_at: Utc::now(),
            source: MetricSource::Database,
        };
        let json = serde_json::to_string(&metric).unwrap();
        assert!(json.contains("test_revenue"));
        assert!(json.contains("revenue"));
    }

    #[test]
    fn test_metrics_summary_serialization() {
        let summary = MetricsSummary {
            total_metrics: 42,
            categories: HashMap::from([("revenue".to_string(), 10i64)]),
            latest_timestamp: Some(Utc::now()),
        };
        let json = serde_json::to_string(&summary).unwrap();
        assert!(json.contains("42"));
    }

    #[test]
    fn test_metric_category_serialization() {
        let cat = MetricCategory::Revenue;
        let json = serde_json::to_string(&cat).unwrap();
        assert!(json.contains("revenue"));
    }

    #[test]
    fn test_metric_source_default() {
        let src = MetricSource::default();
        assert_eq!(src, MetricSource::Database);
    }
}
