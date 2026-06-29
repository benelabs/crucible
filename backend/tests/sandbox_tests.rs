use axum::{
    body::Body,
    http::{Request, StatusCode},
};
use backend::{
    api::handlers::sandbox,
    services::sandbox::{
        ContractExecutionRequest, ContractExecutor, ContractSandboxService, ExecutionStatus,
        PreparedExecution, RawExecution, SandboxError, SandboxLimits,
    },
};
use serde_json::json;
use std::sync::Arc;
use tower::ServiceExt;

#[derive(Clone)]
struct IntegrationExecutor;

impl ContractExecutor for IntegrationExecutor {
    fn execute(&self, execution: PreparedExecution) -> Result<RawExecution, SandboxError> {
        Ok(RawExecution {
            status: ExecutionStatus::Succeeded,
            contract_id: Some("sandboxed-contract".to_string()),
            result_xdr: Some(execution.function.into_bytes()),
            diagnostics: Vec::new(),
            cpu_instructions: 100,
            memory_bytes: 200,
        })
    }
}

#[tokio::test]
async fn sandbox_execute_endpoint_returns_standard_api_response() {
    let service = Arc::new(ContractSandboxService::with_executor(
        SandboxLimits::default(),
        Arc::new(IntegrationExecutor),
    ));
    let app = sandbox::routes(service);
    let payload = json!({
        "wasm_base64": "AGFzbQEAAAA=",
        "function": "invoke",
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

    assert_eq!(json["status"], "success");
    assert_eq!(json["data"]["status"], "succeeded");
    assert_eq!(json["data"]["contract_id"], "sandboxed-contract");
    assert_eq!(json["data"]["cpu_instructions"], 100);
}

#[tokio::test]
async fn sandbox_service_blocks_oversized_arg_lists_before_execution() {
    let service = ContractSandboxService::default();
    let mut args = Vec::new();
    for _ in 0..=SandboxLimits::default().max_args {
        args.push("AAAAAA==".to_string());
    }

    let request = ContractExecutionRequest {
        wasm_base64: "AGFzbQEAAAA=".to_string(),
        function: "invoke".to_string(),
        args_xdr_base64: args,
        ledger_sequence: None,
        ledger_timestamp: None,
        budget: Default::default(),
    };

    let error = service.execute(request).await.unwrap_err();

    assert!(matches!(error, SandboxError::InvalidRequest(_)));
}
