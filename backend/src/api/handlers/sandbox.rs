use crate::{
    api::contracts::{ApiError, ApiResponse, Validate, ValidatedJson},
    services::sandbox::{
        ContractExecutionRequest, ContractExecutionResponse, ContractSandboxService, SandboxError,
    },
};
use axum::{extract::State, routing::post, Router};
use soroban_sdk::xdr::{Limits, ReadXdr, ScVal};
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
        return Err(ApiError::Validation(format!(
            "WASM payload exceeds maximum size of {} bytes (got {} bytes)",
            MAX_WASM_BASE64_SIZE, wasm_size
        )));
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

    // Validate args size and recursion depth if present
    if !request.args_xdr_base64.is_empty() {
        let args_size: usize = request.args_xdr_base64.iter().map(|s| s.len()).sum();
        const MAX_ARGS_SIZE: usize = 1 * 1024 * 1024; // 1MB

        if args_size > MAX_ARGS_SIZE {
            return Err(ApiError::Validation(format!(
                "Arguments payload exceeds maximum size of {} bytes (got {} bytes)",
                MAX_ARGS_SIZE, args_size
            )));
        }

        // Validate recursion depth for XDR argument structures to prevent stack exhaustion (Issue #1140)
        const MAX_XDR_DEPTH: u32 = 64;
        for (i, arg_b64) in request.args_xdr_base64.iter().enumerate() {
            let limits = Limits {
                depth: MAX_XDR_DEPTH,
                len: MAX_ARGS_SIZE,
            };
            if let Err(e) = ScVal::from_xdr_base64(arg_b64, limits) {
                return Err(ApiError::Validation(format!(
                    "Argument {} exceeds maximum XDR recursion depth of {} or is malformed: {}",
                    i, MAX_XDR_DEPTH, e
                )));
            }
        }
    }

    service
        .execute(request)
        .await
        .map(ApiResponse::new)
        .map_err(map_sandbox_error)
}

fn map_sandbox_error(error: SandboxError) -> ApiError {
    if error.is_capacity_rejection() {
        return ApiError::RateLimited;
    }

    match error {
        SandboxError::InvalidRequest(message) | SandboxError::Rejected(message) => {
            ApiError::Validation(message)
        }
        SandboxError::TimedOut(message) => ApiError::Internal(message.to_string()),
        SandboxError::FuelExhausted(limit) => ApiError::Validation(format!(
            "execution exceeded {limit} CPU instruction fuel limit"
        )),
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

    #[tokio::test]
    async fn route_accepts_valid_xdr_arguments() {
        use soroban_sdk::xdr::WriteXdr;
        let service = Arc::new(ContractSandboxService::with_executor(
            SandboxLimits::default(),
            Arc::new(OkExecutor),
        ));
        let app = routes(service);
        let arg_val = ScVal::U32(42);
        let arg_b64 = arg_val.to_xdr_base64(Limits::none()).unwrap();

        let payload = json!({
            "wasm_base64": "AGFzbQEAAAA=",
            "function": "hello",
            "args_xdr_base64": [arg_b64]
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
    async fn route_rejects_excessively_nested_xdr_arguments() {
        use soroban_sdk::xdr::{ScVec, WriteXdr};
        let service = Arc::new(ContractSandboxService::with_executor(
            SandboxLimits::default(),
            Arc::new(OkExecutor),
        ));
        let app = routes(service);

        // Build deeply nested ScVal (nested 70 levels deep, exceeding limit of 64)
        let mut nested = ScVal::U32(1);
        for _ in 0..70 {
            nested = ScVal::Vec(Some(ScVec(vec![nested].try_into().unwrap())));
        }
        let deeply_nested_b64 = nested.to_xdr_base64(Limits::none()).unwrap();

        let payload = json!({
            "wasm_base64": "AGFzbQEAAAA=",
            "function": "hello",
            "args_xdr_base64": [deeply_nested_b64]
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

    #[tokio::test]
    async fn route_rejects_malformed_xdr_arguments() {
        let service = Arc::new(ContractSandboxService::with_executor(
            SandboxLimits::default(),
            Arc::new(OkExecutor),
        ));
        let app = routes(service);

        let payload = json!({
            "wasm_base64": "AGFzbQEAAAA=",
            "function": "hello",
            "args_xdr_base64": ["not_valid_xdr_base64!"]
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

    #[tokio::test]
    async fn route_returns_too_many_requests_when_capacity_exceeded() {
        struct RejectingCapacityExecutor;
        impl ContractExecutor for RejectingCapacityExecutor {
            fn execute(&self, _execution: PreparedExecution) -> Result<RawExecution, SandboxError> {
                Err(SandboxError::Rejected("sandbox execution thread pool capacity exceeded; too many concurrent invocations".to_string()))
            }
        }

        let service = Arc::new(ContractSandboxService::with_executor(
            SandboxLimits::default(),
            Arc::new(RejectingCapacityExecutor),
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

        assert_eq!(response.status(), StatusCode::TOO_MANY_REQUESTS);
    }

    #[tokio::test]
    async fn route_maps_fuel_exhausted_service_response() {
        struct FuelExhaustedMock;
        impl ContractExecutor for FuelExhaustedMock {
            fn execute(&self, execution: PreparedExecution) -> Result<RawExecution, SandboxError> {
                Err(SandboxError::FuelExhausted(
                    execution.limits.max_cpu_instructions,
                ))
            }
        }

        let service = Arc::new(ContractSandboxService::with_executor(
            SandboxLimits::default(),
            Arc::new(FuelExhaustedMock),
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
        let body = axum::body::to_bytes(response.into_body(), usize::MAX)
            .await
            .unwrap();
        let json: serde_json::Value = serde_json::from_slice(&body).unwrap();
        assert_eq!(json["data"]["status"], "fuel_exhausted");
        assert_eq!(json["data"]["cpu_instructions"], 25_000_000);
    }

    #[test]
    fn test_map_sandbox_error_fuel_exhausted() {
        let err = SandboxError::FuelExhausted(50_000);
        let api_err = map_sandbox_error(err);
        match api_err {
            crate::api::contracts::ApiError::Validation(msg) => {
                assert!(msg.contains("50000"));
                assert!(msg.contains("fuel limit"));
            }
            _ => panic!("expected Validation error"),
        }
    }
}
