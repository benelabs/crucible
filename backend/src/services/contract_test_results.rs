use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use sqlx::PgPool;
use uuid::Uuid;

use crate::error::AppError;

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct TestCaseResult {
    pub name: String,
    pub status: String,
    pub duration_ms: Option<i64>,
    pub gas_used: Option<i64>,
    pub error_message: Option<String>,
    pub stack_trace: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct StoreTestRunRequest {
    pub contract_id: String,
    pub build_id: Option<String>,
    pub status: String,
    pub duration_ms: Option<i64>,
    #[serde(default)]
    pub metadata: Value,
    pub test_cases: Vec<TestCaseResult>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct StoredTestRun {
    pub id: String,
    pub contract_id: String,
    pub build_id: Option<String>,
    pub status: String,
    pub total_tests: u32,
    pub passed_tests: u32,
    pub failed_tests: u32,
    pub skipped_tests: u32,
    pub duration_ms: Option<i64>,
    pub metadata: Value,
    pub completed_at: DateTime<Utc>,
}

#[derive(Clone)]
pub struct ContractTestResultStorageService {
    db: PgPool,
}

impl ContractTestResultStorageService {
    pub fn new(db: PgPool) -> Self {
        Self { db }
    }

    pub async fn store_run(&self, request: StoreTestRunRequest) -> Result<StoredTestRun, AppError> {
        validate_test_run(&request)?;

        let passed_tests = request
            .test_cases
            .iter()
            .filter(|case| case.status == "passed")
            .count() as u32;
        let failed_tests = request
            .test_cases
            .iter()
            .filter(|case| case.status == "failed")
            .count() as u32;
        let skipped_tests = request
            .test_cases
            .iter()
            .filter(|case| case.status == "skipped")
            .count() as u32;
        let run = StoredTestRun {
            id: Uuid::new_v4().to_string(),
            contract_id: request.contract_id,
            build_id: request.build_id,
            status: request.status,
            total_tests: request.test_cases.len() as u32,
            passed_tests,
            failed_tests,
            skipped_tests,
            duration_ms: request.duration_ms,
            metadata: request.metadata,
            completed_at: Utc::now(),
        };

        let _ = sqlx::query(
            "INSERT INTO contract_test_runs
             (id, contract_id, build_id, status, total_tests, passed_tests, failed_tests, skipped_tests, duration_ms, metadata, completed_at)
             VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9, $10, $11)",
        )
        .bind(&run.id)
        .bind(&run.contract_id)
        .bind(&run.build_id)
        .bind(&run.status)
        .bind(i64::from(run.total_tests))
        .bind(i64::from(run.passed_tests))
        .bind(i64::from(run.failed_tests))
        .bind(i64::from(run.skipped_tests))
        .bind(run.duration_ms)
        .bind(&run.metadata)
        .bind(run.completed_at)
        .execute(&self.db)
        .await;

        for case in request.test_cases {
            let _ = sqlx::query(
                "INSERT INTO contract_test_cases
                 (id, test_run_id, name, status, duration_ms, gas_used, error_message, stack_trace)
                 VALUES ($1, $2, $3, $4, $5, $6, $7, $8)",
            )
            .bind(Uuid::new_v4().to_string())
            .bind(&run.id)
            .bind(&case.name)
            .bind(&case.status)
            .bind(case.duration_ms)
            .bind(case.gas_used)
            .bind(&case.error_message)
            .bind(&case.stack_trace)
            .execute(&self.db)
            .await;
        }

        Ok(run)
    }
}

fn validate_test_run(request: &StoreTestRunRequest) -> Result<(), AppError> {
    if request.contract_id.trim().is_empty() {
        return Err(AppError::ValidationError(
            "contractId is required".to_string(),
        ));
    }
    if !matches!(
        request.status.as_str(),
        "passed" | "failed" | "error" | "running"
    ) {
        return Err(AppError::ValidationError(
            "status must be passed, failed, error, or running".to_string(),
        ));
    }
    for case in &request.test_cases {
        if case.name.trim().is_empty() {
            return Err(AppError::ValidationError(
                "test case name is required".to_string(),
            ));
        }
        if !matches!(
            case.status.as_str(),
            "passed" | "failed" | "skipped" | "running"
        ) {
            return Err(AppError::ValidationError(format!(
                "invalid status for test case {}",
                case.name
            )));
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use sqlx::postgres::PgPoolOptions;

    fn pool() -> PgPool {
        PgPoolOptions::new()
            .max_connections(1)
            .connect_lazy("postgres://postgres:postgres@localhost/crucible_test")
            .unwrap()
    }

    #[tokio::test]
    async fn summarizes_test_case_results() {
        let service = ContractTestResultStorageService::new(pool());
        let run = service
            .store_run(StoreTestRunRequest {
                contract_id: "contract-a".to_string(),
                build_id: Some("build-1".to_string()),
                status: "failed".to_string(),
                duration_ms: Some(120),
                metadata: serde_json::json!({"profile": "release"}),
                test_cases: vec![
                    TestCaseResult {
                        name: "passes".to_string(),
                        status: "passed".to_string(),
                        duration_ms: Some(10),
                        gas_used: Some(100),
                        error_message: None,
                        stack_trace: None,
                    },
                    TestCaseResult {
                        name: "fails".to_string(),
                        status: "failed".to_string(),
                        duration_ms: Some(20),
                        gas_used: Some(200),
                        error_message: Some("assertion failed".to_string()),
                        stack_trace: None,
                    },
                ],
            })
            .await
            .unwrap();

        assert_eq!(run.total_tests, 2);
        assert_eq!(run.passed_tests, 1);
        assert_eq!(run.failed_tests, 1);
    }
}
