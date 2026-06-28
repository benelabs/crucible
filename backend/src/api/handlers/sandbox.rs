use crate::{
    api::contracts::{ApiError, ApiResponse, Validate, ValidatedJson},
    services::sandbox::{
        ContractExecutionRequest, ContractExecutionResponse, ContractSandboxService, SandboxError,
    },
};
use axum::{extract::State, routing::post, Router};
use std::sync::Arc;
use tracing::instrument;

impl Validate for ContractExecutionRequest {
    fn validate(&self) -> Result<(), String> {
        if self.wasm_base64.trim().is_empty() {
            return Err("wasm_base64 is required".to_string());
        }
        if self.function.trim().is_empty() {
            return Err("function is required".to_string());
        }
        Ok(())
    }
}

/// Routes for bounded contract execution.
pub fn routes(service: Arc<ContractSandboxService>) -> Router {
    Router::new()
        .route("/execute", post(execute_contract))
        .with_state(service)
}

/// Executes a Soroban contract WASM module in a bounded sandbox.
#[instrument(skip_all, fields(http.method = "POST", http.route = "/api/v1/sandbox/execute"))]
pub async fn execute_contract(
    State(service): State<Arc<ContractSandboxService>>,
    ValidatedJson(request): ValidatedJson<ContractExecutionRequest>,
) -> Result<ApiResponse<ContractExecutionResponse>, ApiError> {
    // Validate WASM size (base64 encoded wasm should be reasonable)
    let wasm_size = request.wasm_base64.len();
    const MAX_WASM_BASE64_SIZE: usize = 5 * 1024 * 1024; // 5MB base64
    
    if wasm_size > MAX_WASM_BASE64_SIZE {
        return Err(ApiError::Validation(
            format!(
                "WASM payload exceeds maximum size of {} bytes (got {} bytes)",
                MAX_WASM_BASE64_SIZE, wasm_size
            ),
        ));
    }
    
    // Validate function name
    if request.function.trim().is_empty() {
        return Err(ApiError::Validation(
            "function name cannot be empty".to_string(),
        ));
    }
    
    // Validate function name length
    if request.function.len() > 256 {
        return Err(ApiError::Validation(
            "function name exceeds maximum length of 256 characters".to_string(),
        ));
    }
    
    // Validate args size if present
    if let Some(ref args) = request.args_xdr_base64 {
        let args_size = args.len();
        const MAX_ARGS_SIZE: usize = 1 * 1024 * 1024; // 1MB
        
        if args_size > MAX_ARGS_SIZE {
            return Err(ApiError::Validation(
                format!(
                    "Arguments payload exceeds maximum size of {} bytes (got {} bytes)",
                    MAX_ARGS_SIZE, args_size
                ),
            ));
        }
    }
    
    service
        .execute(request)
        .await
        .map(ApiResponse::new)
        .map_err(map_sandbox_error)
}

fn map_sandbox_error(error: SandboxError) -> ApiError {
    match error {
        SandboxError::InvalidRequest(message) => ApiError::Validation(message),
        SandboxError::Rejected(message) => ApiError::Validation(message),
        SandboxError::TimedOut(message) => ApiError::Internal(message.to_string()),
        SandboxError::Worker(message) => ApiError::Internal(message),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::services::sandbox::{
        ContractExecutor, ExecutionStatus, PreparedExecution, RawExecution, SandboxLimits,
    };
    use axum::{
        body::Body,
        http::{Request, StatusCode},
    };
    use serde_json::json;
    use tower::ServiceExt;

    #[derive(Clone)]
    struct OkExecutor;

    impl ContractExecutor for OkExecutor {
        fn execute(&self, _execution: PreparedExecution) -> Result<RawExecution, SandboxError> {
            Ok(RawExecution {
                status: ExecutionStatus::Succeeded,
                contract_id: Some("contract".to_string()),
                result_xdr: None,
                diagnostics: Vec::new(),
                cpu_instructions: 7,
                memory_bytes: 11,
            })
        }
    }

    #[tokio::test]
    async fn route_executes_contract_request() {
        let service = Arc::new(ContractSandboxService::with_executor(
            SandboxLimits::default(),
            Arc::new(OkExecutor),
        ));
        let app = routes(service);
        let payload = json!({
            "wasm_base64": "AGFzbQEAAAA=",
            "function": "hello",
            "args_xdr_base64": []
        });

        let response = app
            .oneshot(
                Request::builder()
                    .method("POST")
                    .uri("/execute")
                    .header("content-type", "application/json")
                    .body(Body::from(serde_json::to_vec(&payload).unwrap()))
                    .unwrap(),
            )
            .await
            .unwrap();

        assert_eq!(response.status(), StatusCode::OK);
    }

    #[tokio::test]
    async fn route_rejects_invalid_payload() {
        let service = Arc::new(ContractSandboxService::default());
        let app = routes(service);
        let payload = json!({
            "wasm_base64": "",
            "function": "hello"
        });

        let response = app
            .oneshot(
                Request::builder()
                    .method("POST")
                    .uri("/execute")
                    .header("content-type", "application/json")
                    .body(Body::from(serde_json::to_vec(&payload).unwrap()))
                    .unwrap(),
            )
            .await
            .unwrap();

        assert_eq!(response.status(), StatusCode::BAD_REQUEST);
    }
}
