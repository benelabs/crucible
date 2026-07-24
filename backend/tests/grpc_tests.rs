use backend::services::grpc::{
    ContractMetricsRequest, ContractMetricsResponse, InternalGrpcClient, InternalGrpcService,
    PingRequest,
};

#[tokio::test]
async fn test_grpc_ping_handler() {
    let service = InternalGrpcService::new();
    let req = PingRequest {
        client_id: "test-client".to_string(),
        timestamp: 123456789,
    };

    let res = service.handle_ping(req).await.expect("Ping failed");
    assert_eq!(res.status, "OK");
}

#[tokio::test]
async fn test_grpc_contract_metrics_handler() {
    let service = InternalGrpcService::new();

    let metrics = ContractMetricsResponse {
        contract_id: "contract-123".to_string(),
        total_calls: 150,
        total_gas_used: 500000,
        avg_execution_time_ms: 12.5,
        error_count: 0,
    };

    service.update_contract_metrics(metrics.clone()).await;

    let req = ContractMetricsRequest {
        contract_id: "contract-123".to_string(),
        network: "testnet".to_string(),
    };

    let res = service
        .get_contract_metrics(req)
        .await
        .expect("Metrics fetch failed");
    assert_eq!(res.total_calls, 150);
    assert_eq!(res.total_gas_used, 500000);
}

#[test]
fn test_grpc_client_initialization() {
    let client = InternalGrpcClient::new("http://127.0.0.1:50051");
    assert_eq!(client.endpoint(), "http://127.0.0.1:50051");
}
