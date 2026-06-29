//! HTTP handlers for the contract test coverage reporting API.
//!
//! Routes:
//!   POST /api/v1/coverage          – submit a new coverage snapshot
//!   GET  /api/v1/coverage/:project – retrieve the latest snapshot for a project

use axum::{
    extract::{Path, State},
    http::StatusCode,
    response::IntoResponse,
    Json,
};
use std::sync::Arc;

use crate::services::test_coverage::{NewTestCoverage, TestCoverageService};
use crate::error::AppError;

/// Shared state for coverage routes.
#[derive(Clone)]
pub struct CoverageState {
    pub service: TestCoverageService,
}

/// POST /api/v1/coverage
///
/// Accepts a JSON coverage payload and persists a new snapshot.
/// Returns `201 Created` with the stored record on success.
pub async fn submit_coverage(
    State(state): State<Arc<CoverageState>>,
    Json(payload): Json<NewTestCoverage>,
) -> Result<impl IntoResponse, AppError> {
    let record = state
        .service
        .submit_coverage(payload)
        .await
        .map_err(AppError::from)?;
    Ok((StatusCode::CREATED, Json(record)))
}

/// GET /api/v1/coverage/:project
///
/// Returns the most-recent coverage snapshot for `project`.
/// Returns `404 Not Found` when no report has been submitted yet.
pub async fn get_latest_coverage(
    State(state): State<Arc<CoverageState>>,
    Path(project): Path<String>,
) -> Result<impl IntoResponse, AppError> {
    let record = state
        .service
        .get_latest_coverage(&project)
        .await
        .map_err(AppError::from)?;
    Ok(Json(record))
}
