use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::sync::Arc;
use std::time::Duration;
use tokio::sync::RwLock;
use tonic::transport::Server;

/// gRPC Server configuration with keepalive and limits
#[derive(Debug, Clone)]
pub struct GrpcServerConfig {
    /// HTTP/2 keepalive ping interval (default: 30 seconds)
    pub http2_keepalive_interval: Duration,
    /// HTTP/2 keepalive timeout (default: 10 seconds)
    pub http2_keepalive_timeout: Duration,
    /// Maximum connection age before forced closure (default: 5 minutes)
    pub max_connection_age: Duration,
    /// Maximum inbound message size in bytes (default: 10MB)
    pub max_decoding_message_size: usize,
}

impl Default for GrpcServerConfig {
    fn default() -> Self {
        Self {
            http2_keepalive_interval: Duration::from_secs(30),
            http2_keepalive_timeout: Duration::from_secs(10),
            max_connection_age: Duration::from_secs(300), // 5 minutes
            max_decoding_message_size: 10 * 1024 * 1024,  // 10 MB
        }
    }
}

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
    grpc_config: Option<GrpcServerConfig>,
}

impl InternalGrpcService {
    pub fn new() -> Self {
        Self {
            metrics_cache: Arc::new(RwLock::new(HashMap::new())),
            grpc_config: Some(GrpcServerConfig::default()),
        }
    }

    pub fn with_config(config: GrpcServerConfig) -> Self {
        Self {
            metrics_cache: Arc::new(RwLock::new(HashMap::new())),
            grpc_config: Some(config),
        }
    }

    /// Get the gRPC server configuration
    pub fn config(&self) -> GrpcServerConfig {
        self.grpc_config.clone().unwrap_or_default()
    }

    /// Build a Tonic Server with proper keepalive configuration
    pub fn build_server_with_config(&self) -> Server {
        let config = self.config();

        Server::builder()
            // Configure HTTP/2 keepalive to detect dead connections
            .http2_keepalive_interval(Some(config.http2_keepalive_interval))
            .http2_keepalive_timeout(Some(config.http2_keepalive_timeout))
            // Force close connections after max_connection_age
            .max_connection_age(config.max_connection_age)
    }

    /// Process high-speed ping diagnostic request
    pub async fn handle_ping(&self, _req: PingRequest) -> Result<PingResponse, String> {
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_grpc_server_config_defaults() {
        let config = GrpcServerConfig::default();
        assert_eq!(config.http2_keepalive_interval.as_secs(), 30);
        assert_eq!(config.http2_keepalive_timeout.as_secs(), 10);
        assert_eq!(config.max_connection_age.as_secs(), 300);
        assert_eq!(config.max_decoding_message_size, 10 * 1024 * 1024);
    }

    #[test]
    fn test_grpc_server_config_custom() {
        let config = GrpcServerConfig {
            http2_keepalive_interval: Duration::from_secs(60),
            http2_keepalive_timeout: Duration::from_secs(20),
            max_connection_age: Duration::from_secs(600),
            max_decoding_message_size: 20 * 1024 * 1024,
        };

        assert_eq!(config.http2_keepalive_interval.as_secs(), 60);
        assert_eq!(config.http2_keepalive_timeout.as_secs(), 20);
        assert_eq!(config.max_connection_age.as_secs(), 600);
        assert_eq!(config.max_decoding_message_size, 20 * 1024 * 1024);
    }

    #[test]
    fn test_internal_grpc_service_default_config() {
        let service = InternalGrpcService::new();
        let config = service.config();
        assert_eq!(config.http2_keepalive_interval.as_secs(), 30);
        assert_eq!(config.max_decoding_message_size, 10 * 1024 * 1024);
    }

    #[test]
    fn test_internal_grpc_service_custom_config() {
        let custom_config = GrpcServerConfig {
            http2_keepalive_interval: Duration::from_secs(45),
            http2_keepalive_timeout: Duration::from_secs(15),
            max_connection_age: Duration::from_secs(400),
            max_decoding_message_size: 5 * 1024 * 1024,
        };

        let service = InternalGrpcService::with_config(custom_config.clone());
        let config = service.config();
        assert_eq!(config.http2_keepalive_interval.as_secs(), 45);
        assert_eq!(config.max_decoding_message_size, 5 * 1024 * 1024);
    }

    #[tokio::test]
    async fn test_internal_grpc_service_ping() {
        let service = InternalGrpcService::new();
        let req = PingRequest {
            client_id: "test-client".to_string(),
            timestamp: 1234567890,
        };

        let resp = service.handle_ping(req).await.unwrap();
        assert_eq!(resp.status, "OK");
        assert!(resp.server_time > 0);
    }

    #[tokio::test]
    async fn test_contract_metrics_caching() {
        let service = InternalGrpcService::new();
        let metrics = ContractMetricsResponse {
            contract_id: "CAB123".to_string(),
            total_calls: 42,
            total_gas_used: 5000,
            avg_execution_time_ms: 125.5,
            error_count: 2,
        };

        service.update_contract_metrics(metrics.clone()).await;

        let req = ContractMetricsRequest {
            contract_id: "CAB123".to_string(),
            network: "testnet".to_string(),
        };

        let result = service.get_contract_metrics(req).await.unwrap();
        assert_eq!(result.contract_id, "CAB123");
        assert_eq!(result.total_calls, 42);
    }
}
