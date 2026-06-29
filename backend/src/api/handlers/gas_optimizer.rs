/// Gas optimization API handlers.

use axum::{extract::State, response::IntoResponse, Json};
use serde::{Deserialize, Serialize};
use std::sync::Arc;

use crate::api::contracts::ApiResponse;
use crate::error::AppError;
use crate::services::gas_optimizer::{GasOptimizer, OptimizationResult};
use crate::api::handlers::profiling::AppState;

/// Request to analyze bytecode for gas optimization.
#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AnalyzeBytecodeRequest {
    pub contract_address: String,
    pub bytecode: Vec<u8>,
}

/// Request to analyze source code for gas optimization.
#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AnalyzeSourceCodeRequest {
    pub contract_address: String,
    pub source_code: String,
}

/// POST /api/v1/contracts/optimize/bytecode
pub async fn optimize_bytecode(
    _state: State<Arc<AppState>>,
    Json(payload): Json<AnalyzeBytecodeRequest>,
) -> Result<impl IntoResponse, AppError> {
    let optimizer = GasOptimizer::new();

    let result = optimizer
        .analyze_bytecode(payload.contract_address, payload.bytecode)
        .map_err(|e| AppError::BadRequest(e))?;

    Ok(Json(ApiResponse::new(result)))
}

/// POST /api/v1/contracts/optimize/source
pub async fn optimize_source_code(
    _state: State<Arc<AppState>>,
    Json(payload): Json<AnalyzeSourceCodeRequest>,
) -> Result<impl IntoResponse, AppError> {
    let optimizer = GasOptimizer::new();

    let result = optimizer
        .analyze_source_code(payload.contract_address, &payload.source_code)
        .map_err(|e| AppError::BadRequest(e))?;

    Ok(Json(ApiResponse::new(result)))
}

/// GET /api/v1/contracts/optimize/:address/report
pub async fn get_optimization_report(
    _state: State<Arc<AppState>>,
    Json(result): Json<OptimizationResult>,
) -> Result<impl IntoResponse, AppError> {
    let optimizer = GasOptimizer::new();
    let report = optimizer.generate_report(&result);

    #[derive(Serialize)]
    struct ReportResponse {
        report: String,
    }

    Ok(Json(ApiResponse::new(ReportResponse { report })))
}
