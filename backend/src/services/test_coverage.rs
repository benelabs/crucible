//! Contract test coverage reporting service.
//!
//! Stores per-commit coverage snapshots in PostgreSQL and caches the latest
//! report for each project in Redis (1-hour TTL) to avoid redundant DB queries.

use chrono::{DateTime, Utc};
use redis::AsyncCommands;
use serde::{Deserialize, Serialize};
use sqlx::PgPool;
use thiserror::Error;
use tracing::{debug, info, instrument};
use uuid::Uuid;

use crate::error::AppError;

// ── Error ─────────────────────────────────────────────────────────────────────

#[derive(Debug, Error)]
pub enum CoverageError {
    #[error("database error: {0}")]
    Database(#[from] sqlx::Error),
    #[error("redis error: {0}")]
    Redis(#[from] redis::RedisError),
    #[error("serialization error: {0}")]
    Serialization(#[from] serde_json::Error),
    #[error("project not found: {0}")]
    NotFound(String),
    #[error("validation error: {0}")]
    Validation(String),
}

impl From<CoverageError> for AppError {
    fn from(e: CoverageError) -> Self {
        match e {
            CoverageError::NotFound(msg) => AppError::NotFound(msg),
            CoverageError::Validation(msg) => AppError::ValidationError(msg),
            CoverageError::Database(e) => AppError::Database(e),
            CoverageError::Redis(e) => AppError::Redis(e),
            CoverageError::Serialization(e) => AppError::Serialization(e),
        }
    }
}

// ── Domain types ──────────────────────────────────────────────────────────────

/// A persisted coverage snapshot for one build.
#[derive(Debug, Clone, Serialize, Deserialize, sqlx::FromRow, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct TestCoverage {
    pub id: Uuid,
    pub project_name: String,
    pub branch: String,
    pub commit_sha: String,
    /// Overall line-coverage percentage (0.0 – 100.0).
    pub coverage_percent: f64,
    pub total_lines: i32,
    pub covered_lines: i32,
    pub created_at: DateTime<Utc>,
}

/// Payload for submitting a new coverage report.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct NewTestCoverage {
    pub project_name: String,
    pub branch: String,
    pub commit_sha: String,
    pub coverage_percent: f64,
    pub total_lines: i32,
    pub covered_lines: i32,
}

// ── Service ───────────────────────────────────────────────────────────────────

#[derive(Clone)]
pub struct TestCoverageService {
    db: PgPool,
    redis: redis::Client,
}

impl TestCoverageService {
    pub fn new(db: PgPool, redis: redis::Client) -> Self {
        Self { db, redis }
    }

    /// Persist a new coverage snapshot and update the Redis cache.
    #[instrument(skip(self, data), fields(project = %data.project_name, branch = %data.branch))]
    pub async fn submit_coverage(
        &self,
        data: NewTestCoverage,
    ) -> Result<TestCoverage, CoverageError> {
        validate_coverage(&data)?;

        info!("Persisting coverage report");

        let record = sqlx::query_as::<_, TestCoverage>(
            "INSERT INTO test_coverage \
             (id, project_name, branch, commit_sha, coverage_percent, total_lines, covered_lines, created_at) \
             VALUES ($1, $2, $3, $4, $5, $6, $7, $8) \
             RETURNING *",
        )
        .bind(Uuid::new_v4())
        .bind(&data.project_name)
        .bind(&data.branch)
        .bind(&data.commit_sha)
        .bind(data.coverage_percent)
        .bind(data.total_lines)
        .bind(data.covered_lines)
        .bind(Utc::now())
        .fetch_one(&self.db)
        .await?;

        self.cache_set(&data.project_name, &record).await?;
        Ok(record)
    }

    /// Return the most-recent coverage snapshot for a project (cache-first).
    #[instrument(skip(self), fields(project = %project_name))]
    pub async fn get_latest_coverage(
        &self,
        project_name: &str,
    ) -> Result<TestCoverage, CoverageError> {
        // 1. Try Redis cache.
        if let Some(cached) = self.cache_get(project_name).await? {
            debug!("Coverage cache hit");
            return Ok(cached);
        }

        // 2. Fall back to DB.
        debug!("Coverage cache miss – querying database");
        let record = sqlx::query_as::<_, TestCoverage>(
            "SELECT * FROM test_coverage \
             WHERE project_name = $1 \
             ORDER BY created_at DESC LIMIT 1",
        )
        .bind(project_name)
        .fetch_optional(&self.db)
        .await?
        .ok_or_else(|| CoverageError::NotFound(project_name.to_string()))?;

        self.cache_set(project_name, &record).await?;
        Ok(record)
    }

    // ── helpers ───────────────────────────────────────────────────────────────

    fn cache_key(project_name: &str) -> String {
        format!("coverage:latest:{project_name}")
    }

    async fn cache_get(&self, project_name: &str) -> Result<Option<TestCoverage>, CoverageError> {
        let mut conn = self.redis.get_multiplexed_async_connection().await?;
        let raw: Option<String> = conn.get(Self::cache_key(project_name)).await?;
        raw.map(|s| serde_json::from_str(&s).map_err(CoverageError::from))
            .transpose()
    }

    async fn cache_set(
        &self,
        project_name: &str,
        record: &TestCoverage,
    ) -> Result<(), CoverageError> {
        let serialized = serde_json::to_string(record)?;
        let mut conn = self.redis.get_multiplexed_async_connection().await?;
        let _: () = conn
            .set_ex(Self::cache_key(project_name), serialized, 3600)
            .await?;
        debug!("Updated coverage cache for {project_name}");
        Ok(())
    }
}

// ── Validation ────────────────────────────────────────────────────────────────

fn validate_coverage(data: &NewTestCoverage) -> Result<(), CoverageError> {
    if data.project_name.trim().is_empty() {
        return Err(CoverageError::Validation(
            "projectName is required".to_string(),
        ));
    }
    if data.branch.trim().is_empty() {
        return Err(CoverageError::Validation("branch is required".to_string()));
    }
    if data.commit_sha.trim().is_empty() {
        return Err(CoverageError::Validation(
            "commitSha is required".to_string(),
        ));
    }
    if !(0.0..=100.0).contains(&data.coverage_percent) {
        return Err(CoverageError::Validation(
            "coveragePercent must be between 0 and 100".to_string(),
        ));
    }
    if data.total_lines < 0 {
        return Err(CoverageError::Validation(
            "totalLines must be non-negative".to_string(),
        ));
    }
    if data.covered_lines < 0 {
        return Err(CoverageError::Validation(
            "coveredLines must be non-negative".to_string(),
        ));
    }
    if data.covered_lines > data.total_lines {
        return Err(CoverageError::Validation(
            "coveredLines cannot exceed totalLines".to_string(),
        ));
    }
    Ok(())
}

// ── Unit tests ────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    fn valid_coverage() -> NewTestCoverage {
        NewTestCoverage {
            project_name: "my-contract".to_string(),
            branch: "main".to_string(),
            commit_sha: "abc123".to_string(),
            coverage_percent: 87.5,
            total_lines: 200,
            covered_lines: 175,
        }
    }

    // ── validate_coverage ─────────────────────────────────────────────────────

    #[test]
    fn validation_passes_for_valid_input() {
        assert!(validate_coverage(&valid_coverage()).is_ok());
    }

    #[test]
    fn validation_rejects_empty_project_name() {
        let data = NewTestCoverage {
            project_name: "  ".to_string(),
            ..valid_coverage()
        };
        let err = validate_coverage(&data).unwrap_err();
        assert!(err.to_string().contains("projectName"));
    }

    #[test]
    fn validation_rejects_empty_branch() {
        let data = NewTestCoverage {
            branch: String::new(),
            ..valid_coverage()
        };
        let err = validate_coverage(&data).unwrap_err();
        assert!(err.to_string().contains("branch"));
    }

    #[test]
    fn validation_rejects_empty_commit_sha() {
        let data = NewTestCoverage {
            commit_sha: String::new(),
            ..valid_coverage()
        };
        let err = validate_coverage(&data).unwrap_err();
        assert!(err.to_string().contains("commitSha"));
    }

    #[test]
    fn validation_rejects_coverage_above_100() {
        let data = NewTestCoverage {
            coverage_percent: 100.1,
            ..valid_coverage()
        };
        let err = validate_coverage(&data).unwrap_err();
        assert!(err.to_string().contains("coveragePercent"));
    }

    #[test]
    fn validation_rejects_negative_coverage() {
        let data = NewTestCoverage {
            coverage_percent: -1.0,
            ..valid_coverage()
        };
        let err = validate_coverage(&data).unwrap_err();
        assert!(err.to_string().contains("coveragePercent"));
    }

    #[test]
    fn validation_rejects_negative_total_lines() {
        let data = NewTestCoverage {
            total_lines: -1,
            ..valid_coverage()
        };
        let err = validate_coverage(&data).unwrap_err();
        assert!(err.to_string().contains("totalLines"));
    }

    #[test]
    fn validation_rejects_covered_exceeding_total() {
        let data = NewTestCoverage {
            total_lines: 100,
            covered_lines: 101,
            ..valid_coverage()
        };
        let err = validate_coverage(&data).unwrap_err();
        assert!(err.to_string().contains("coveredLines"));
    }

    #[test]
    fn validation_accepts_zero_coverage() {
        let data = NewTestCoverage {
            coverage_percent: 0.0,
            total_lines: 100,
            covered_lines: 0,
            ..valid_coverage()
        };
        assert!(validate_coverage(&data).is_ok());
    }

    #[test]
    fn validation_accepts_full_coverage() {
        let data = NewTestCoverage {
            coverage_percent: 100.0,
            total_lines: 50,
            covered_lines: 50,
            ..valid_coverage()
        };
        assert!(validate_coverage(&data).is_ok());
    }

    // ── Error conversion ──────────────────────────────────────────────────────

    #[test]
    fn coverage_error_not_found_converts_to_app_error() {
        let err: AppError = CoverageError::NotFound("proj".to_string()).into();
        assert!(matches!(err, AppError::NotFound(_)));
    }

    #[test]
    fn coverage_error_validation_converts_to_app_error() {
        let err: AppError = CoverageError::Validation("bad input".to_string()).into();
        assert!(matches!(err, AppError::ValidationError(_)));
    }

    // ── cache_key ─────────────────────────────────────────────────────────────

    #[test]
    fn cache_key_is_deterministic() {
        assert_eq!(
            TestCoverageService::cache_key("my-project"),
            "coverage:latest:my-project"
        );
    }
}
