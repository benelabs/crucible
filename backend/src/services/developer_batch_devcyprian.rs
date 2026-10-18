#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SandboxMemoryCap {
    pub max_wasm_bytes: usize,
    pub max_arg_bytes: usize,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct JwtAuthContext {
    pub subject: String,
    pub stellar_address: String,
    pub token_issuer: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct OpenApiDriftCheck {
    pub schema_name: String,
    pub expected_version: String,
    pub actual_version: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RpcClusterNode {
    pub name: String,
    pub url: String,
    pub healthy: bool,
}

pub fn default_memory_cap() -> SandboxMemoryCap {
    SandboxMemoryCap {
        max_wasm_bytes: 5 * 1024 * 1024,
        max_arg_bytes: 1024 * 1024,
    }
}

pub fn build_auth_context(
    subject: impl Into<String>,
    stellar_address: impl Into<String>,
) -> JwtAuthContext {
    JwtAuthContext {
        subject: subject.into(),
        stellar_address: stellar_address.into(),
        token_issuer: "crucible-auth".to_string(),
    }
}

pub fn cluster_failover_ready(nodes: &[RpcClusterNode]) -> bool {
    nodes.iter().filter(|node| node.healthy).count() >= 2
}
