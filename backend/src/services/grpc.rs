use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::sync::Arc;
use tokio::sync::RwLock;

/// High-performance gRPC Internal Communication Layer structures
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct PingRequest {
    pub client_id: String,
    pub timestamp: i64,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct PingResponse {
    pub status: String,
    pub server_time: i64,
    pub version: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct ContractMetricsRequest {
    pub contract_id: String,
    pub network: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct ContractMetricsResponse {
    pub contract_id: String,
    pub total_calls: i64,
    pub total_gas_used: i64,
    pub avg_execution_time_ms: f64,
    pub error_count: i64,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct ContractEventNotification {
    pub contract_id: String,
    pub topic: String,
    pub payload: Vec<u8>,
    pub sequence: i64,
    pub timestamp: i64,
}

/// Internal gRPC Service Handler
#[derive(Clone, Default)]
pub struct InternalGrpcService {
    metrics_cache: Arc<RwLock<HashMap<String, ContractMetricsResponse>>>,
}

impl InternalGrpcService {
    pub fn new() -> Self {
        Self {
            metrics_cache: Arc::new(RwLock::new(HashMap::new())),
        }
    }

    /// Process high-speed ping diagnostic request
    pub async fn handle_ping(&self, req: PingRequest) -> Result<PingResponse, String> {
        let now = chrono::Utc::now().timestamp_millis();
        Ok(PingResponse {
            status: "OK".to_string(),
            server_time: now,
            version: env!("CARGO_PKG_VERSION").to_string(),
        })
    }

    /// Get telemetry metrics for a target contract over gRPC
    pub async fn get_contract_metrics(
        &self,
        req: ContractMetricsRequest,
    ) -> Result<ContractMetricsResponse, String> {
        let cache = self.metrics_cache.read().await;
        if let Some(metrics) = cache.get(&req.contract_id) {
            return Ok(metrics.clone());
        }

        // Default metrics if not cached
        Ok(ContractMetricsResponse {
            contract_id: req.contract_id,
            total_calls: 0,
            total_gas_used: 0,
            avg_execution_time_ms: 0.0,
            error_count: 0,
        })
    }

    /// Update internal contract metrics state
    pub async fn update_contract_metrics(&self, metrics: ContractMetricsResponse) {
        let mut cache = self.metrics_cache.write().await;
        cache.insert(metrics.contract_id.clone(), metrics);
    }
}

/// gRPC Client Helper for inter-service communication
pub struct InternalGrpcClient {
    endpoint: String,
}

impl InternalGrpcClient {
    pub fn new(endpoint: impl Into<String>) -> Self {
        Self {
            endpoint: endpoint.into(),
        }
    }

    pub fn endpoint(&self) -> &str {
        &self.endpoint
    }
}
