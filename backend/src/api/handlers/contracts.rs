use axum::{extract::State, response::IntoResponse, Json};
use serde::{Deserialize, Serialize};
use std::sync::Arc;

use crate::api::contracts::ApiResponse;
use crate::api::handlers::profiling::AppState;
use crate::error::AppError;
use crate::services::compilation::CompilationService;
use crate::services::contract_deployment::{ContractDeploymentService, DeploymentRequest};
use crate::services::contract_storage_optimizer::{
    ContractStorageOptimizer, StorageOptimizationInput,
};
use crate::services::contract_test_results::{
    ContractTestResultStorageService, StoreTestRunRequest,
};
use crate::services::contract_versioning::{
    ContractVersioningService, CreateContractVersionRequest, VersionDiffRequest,
};
use crate::services::contract_upgrade::{ContractUpgradeManager, ContractUpgradeRequest};
use crate::services::dependency_analyzer::DependencyAnalyzer;

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CompileRequest {
    pub project_name: String,
    pub source_code: String,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AnalyzeRequest {
    pub cargo_toml: String,
}

/// POST /api/v1/contracts/upgrade-plan
pub async fn create_upgrade_plan(
    Json(payload): Json<ContractUpgradeRequest>,
) -> Result<impl IntoResponse, AppError> {
    let service = ContractUpgradeManager::new();
    let result = service
        .plan_upgrade(payload)
        .map_err(|err| AppError::ValidationError(err.to_string()))?;

    Ok(Json(ApiResponse::new(result)))
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct NetworkConfig {
    pub id: String,
    pub name: String,
    pub rpc_url: String,
    pub passphrase: String,
    pub status: String,
    pub ping_ms: u32,
    pub latest_ledger: u32,
    pub active_contracts_count: u32,
}

fn require_db(state: &AppState) -> Result<sqlx::PgPool, AppError> {
    state
        .db
        .clone()
        .ok_or_else(|| AppError::InternalError("Database connection not configured".to_string()))
}

/// POST /api/v1/contracts/compile
pub async fn compile_contract(
    State(state): State<Arc<AppState>>,
    Json(payload): Json<CompileRequest>,
) -> Result<impl IntoResponse, AppError> {
    // Validate payload size
    validate_payload_size(&payload, state.config.server.compile_max_size, "compile")?;
    
    // Validate source code
    if payload.source_code.trim().is_empty() {
        return Err(AppError::ValidationError(
            "source_code cannot be empty".to_string(),
        ));
    }
    
    if payload.source_code.len() > 1_000_000 {
        return Err(AppError::ValidationError(
            "source_code exceeds maximum size of 1MB".to_string(),
        ));
    }
    
    // Validate project name
    if payload.project_name.trim().is_empty() {
        return Err(AppError::ValidationError(
            "project_name cannot be empty".to_string(),
        ));
    }

    let db = require_db(&state)?;

    let service = CompilationService::new(db);
    let result = service
        .compile(&payload.project_name, &payload.source_code)
        .await
        .map_err(AppError::Database)?;

    Ok(Json(ApiResponse::new(result)))
}

fn validate_payload_size<T>(
    payload: &T,
    max_size: usize,
    endpoint: &str,
) -> Result<(), AppError> {
    let size = serde_json::to_string(payload)
        .map(|s| s.len())
        .unwrap_or(0);
    
    if size > max_size {
        return Err(AppError::ValidationError(
            format!(
                "Request payload for {} exceeds maximum size of {} bytes (got {} bytes)",
                endpoint, max_size, size
            ),
        ));
    }
    
    Ok(())
}

/// POST /api/v1/contracts/analyze-dependencies
pub async fn analyze_dependencies(
    State(state): State<Arc<AppState>>,
    Json(payload): Json<AnalyzeRequest>,
) -> Result<impl IntoResponse, AppError> {
    let db = require_db(&state)?;

    let service = DependencyAnalyzer::new(db);
    let result = service
        .analyze(&payload.cargo_toml)
        .await
        .map_err(AppError::Database)?;

    Ok(Json(ApiResponse::new(result)))
}

/// GET /api/v1/networks
pub async fn get_networks() -> Result<impl IntoResponse, AppError> {
    let networks = vec![
        NetworkConfig {
            id: "mainnet".to_string(),
            name: "Soroban Mainnet".to_string(),
            rpc_url: "https://soroban-testnet.stellar.org:443".to_string(), // placeholder/mock url
            passphrase: "Public Global Stellar Network ; October 2015".to_string(),
            status: "online".to_string(),
            ping_ms: 82,
            latest_ledger: 1045231,
            active_contracts_count: 1420,
        },
        NetworkConfig {
            id: "testnet".to_string(),
            name: "Soroban Testnet".to_string(),
            rpc_url: "https://soroban-testnet.stellar.org".to_string(),
            passphrase: "Test SDF Network ; September 2015".to_string(),
            status: "online".to_string(),
            ping_ms: 34,
            latest_ledger: 452934,
            active_contracts_count: 328,
        },
        NetworkConfig {
            id: "futurenet".to_string(),
            name: "Soroban Futurenet".to_string(),
            rpc_url: "https://rpc-futurenet.stellar.org".to_string(),
            passphrase: "Test SDF Future Network ; October 2022".to_string(),
            status: "online".to_string(),
            ping_ms: 56,
            latest_ledger: 98124,
            active_contracts_count: 94,
        },
        NetworkConfig {
            id: "sandbox".to_string(),
            name: "Local Sandbox".to_string(),
            rpc_url: "http://localhost:8000".to_string(),
            passphrase: "Standalone Network ; Standalone".to_string(),
            status: "online".to_string(),
            ping_ms: 2,
            latest_ledger: 4239,
            active_contracts_count: 15,
        },
    ];

    Ok(Json(ApiResponse::new(networks)))
}

use crate::services::compliance::ComplianceService;
use crate::services::contract_call_logger::{ContractCallLog, ContractCallLogger};

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ContractTemplate {
    pub id: String,
    pub name: String,
    pub description: String,
    pub category: String,
    pub source_code: String,
    pub compliance_score: u8,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ComplianceRequest {
    pub source_code: String,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct LogCallRequest {
    pub contract_id: String,
    pub function_name: String,
    pub arguments: serde_json::Value,
    pub caller: Option<String>,
    pub status: String,
    pub gas_used: f64,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct GetLogsQuery {
    pub contract_id: Option<String>,
    #[serde(default = "default_logs_limit")]
    pub limit: i64,
}

fn default_logs_limit() -> i64 {
    20
}

/// POST /api/v1/contracts/compliance-check
pub async fn check_compliance(
    State(state): State<Arc<AppState>>,
    Json(payload): Json<ComplianceRequest>,
) -> Result<impl IntoResponse, AppError> {
    let db = require_db(&state)?;

    let service = ComplianceService::new(db);
    let result = service.check_compliance(&payload.source_code).await?;
    Ok(Json(ApiResponse::new(result)))
}

/// POST /api/v1/contracts/logs
pub async fn log_contract_call(
    State(state): State<Arc<AppState>>,
    Json(payload): Json<LogCallRequest>,
) -> Result<impl IntoResponse, AppError> {
    let db = require_db(&state)?;

    let service = ContractCallLogger::new(db);
    let log = ContractCallLog {
        id: None,
        contract_id: payload.contract_id,
        function_name: payload.function_name,
        arguments: payload.arguments,
        caller: payload.caller,
        status: payload.status,
        gas_used: payload.gas_used,
        timestamp: Some(chrono::Utc::now()),
    };
    service.log_call(log).await?;
    Ok(Json(ApiResponse::new(
        serde_json::json!({ "success": true }),
    )))
}

/// GET /api/v1/contracts/logs
pub async fn get_contract_logs(
    State(state): State<Arc<AppState>>,
    axum::extract::Query(query): axum::extract::Query<GetLogsQuery>,
) -> Result<impl IntoResponse, AppError> {
    let db = require_db(&state)?;

    let service = ContractCallLogger::new(db);
    let result = service.get_logs(query.contract_id, query.limit).await?;
    Ok(Json(ApiResponse::new(result)))
}

/// POST /api/v1/contracts/storage/optimize
pub async fn optimize_storage(
    State(state): State<Arc<AppState>>,
    Json(payload): Json<StorageOptimizationInput>,
) -> Result<impl IntoResponse, AppError> {
    let service = ContractStorageOptimizer::new(require_db(&state)?);
    let result = service.optimize(payload).await?;
    Ok(Json(ApiResponse::new(result)))
}

/// POST /api/v1/contracts/versions
pub async fn create_contract_version(
    State(state): State<Arc<AppState>>,
    Json(payload): Json<CreateContractVersionRequest>,
) -> Result<impl IntoResponse, AppError> {
    let service = ContractVersioningService::new(require_db(&state)?);
    let result = service.create_version(payload).await?;
    Ok(Json(ApiResponse::new(result)))
}

/// POST /api/v1/contracts/versions/diff
pub async fn diff_contract_versions(
    State(state): State<Arc<AppState>>,
    Json(payload): Json<VersionDiffRequest>,
) -> Result<impl IntoResponse, AppError> {
    let service = ContractVersioningService::new(require_db(&state)?);
    let result = service.diff(payload);
    Ok(Json(ApiResponse::new(result)))
}

/// POST /api/v1/contracts/deployments
pub async fn create_contract_deployment(
    State(state): State<Arc<AppState>>,
    Json(payload): Json<DeploymentRequest>,
) -> Result<impl IntoResponse, AppError> {
    let service = ContractDeploymentService::new(require_db(&state)?);
    let result = service.create_deployment(payload).await?;
    Ok(Json(ApiResponse::new(result)))
}

/// POST /api/v1/contracts/test-results
pub async fn store_contract_test_results(
    State(state): State<Arc<AppState>>,
    Json(payload): Json<StoreTestRunRequest>,
) -> Result<impl IntoResponse, AppError> {
    let service = ContractTestResultStorageService::new(require_db(&state)?);
    let result = service.store_run(payload).await?;
    Ok(Json(ApiResponse::new(result)))
}

/// GET /api/v1/contracts/templates
pub async fn get_templates() -> Result<impl IntoResponse, AppError> {
    let templates = vec![
        ContractTemplate {
            id: "incrementer".to_string(),
            name: "Soroban Incrementer".to_string(),
            description: "A simple incrementer contract that demonstrates instance storage usage and integer arithmetic in Soroban SDK.".to_string(),
            category: "Basic".to_string(),
            compliance_score: 100,
            source_code: r#"// SPDX-License-Identifier: MIT
use soroban_sdk::{contract, contractimpl, Env, Symbol};

#[contract]
pub struct IncrementContract;

#[contractimpl]
impl IncrementContract {
    pub fn increment(env: Env) -> u32 {
        let key = Symbol::new(&env, "count");
        let mut count: u32 = env.storage().instance().get(&key).unwrap_or(0);
        count += 1;
        env.storage().instance().set(&key, &count);
        count
    }
}"#.to_string(),
        },
        ContractTemplate {
            id: "token".to_string(),
            name: "Fungible Token".to_string(),
            description: "A standard compliant ERC20-equivalent fungible token contract implemented using Soroban SDK guidelines.".to_string(),
            category: "Token".to_string(),
            compliance_score: 95,
            source_code: r#"// SPDX-License-Identifier: MIT
use soroban_sdk::{contract, contractimpl, Address, Env, String, Symbol};

#[contract]
pub struct TokenContract;

#[contractimpl]
impl TokenContract {
    pub fn initialize(env: Env, admin: Address, name: String, symbol: String) {
        env.storage().instance().set(&Symbol::new(&env, "admin"), &admin);
        env.storage().instance().set(&Symbol::new(&env, "name"), &name);
        env.storage().instance().set(&Symbol::new(&env, "symbol"), &symbol);
    }

    pub fn balance(env: Env, id: Address) -> i128 {
        env.storage().persistent().get(&id).unwrap_or(0)
    }

    pub fn transfer(env: Env, from: Address, to: Address, amount: i128) {
        from.require_auth();
        let balance_from = Self::balance(env.clone(), from.clone());
        let balance_to = Self::balance(env.clone(), to.clone());
        assert!(balance_from >= amount, "insufficient balance");
        env.storage().persistent().set(&from, &(balance_from - amount));
        env.storage().persistent().set(&to, &(balance_to + amount));
    }
}"#.to_string(),
        },
        ContractTemplate {
            id: "escrow".to_string(),
            name: "Escrow Contract".to_string(),
            description: "An escrow smart contract that holds funds until a designated arbiter approves release or triggers refund.".to_string(),
            category: "DeFi".to_string(),
            compliance_score: 90,
            source_code: r#"// SPDX-License-Identifier: MIT
use soroban_sdk::{contract, contractimpl, Address, Env, Symbol};

#[contract]
pub struct EscrowContract;

#[contractimpl]
impl EscrowContract {
    pub fn create_escrow(env: Env, sender: Address, receiver: Address, arbiter: Address, amount: i128) {
        env.storage().instance().set(&Symbol::new(&env, "sender"), &sender);
        env.storage().instance().set(&Symbol::new(&env, "receiver"), &receiver);
        env.storage().instance().set(&Symbol::new(&env, "arbiter"), &arbiter);
        env.storage().instance().set(&Symbol::new(&env, "amount"), &amount);
        env.storage().instance().set(&Symbol::new(&env, "status"), &Symbol::new(&env, "pending"));
    }

    pub fn release(env: Env, arbiter: Address) {
        arbiter.require_auth();
        let current_arbiter: Address = env.storage().instance().get(&Symbol::new(&env, "arbiter")).unwrap();
        assert!(arbiter == current_arbiter, "unauthorized arbiter");
        env.storage().instance().set(&Symbol::new(&env, "status"), &Symbol::new(&env, "released"));
    }

    pub fn refund(env: Env, arbiter: Address) {
        arbiter.require_auth();
        let current_arbiter: Address = env.storage().instance().get(&Symbol::new(&env, "arbiter")).unwrap();
        assert!(arbiter == current_arbiter, "unauthorized arbiter");
        env.storage().instance().set(&Symbol::new(&env, "status"), &Symbol::new(&env, "refunded"));
    }
}"#.to_string(),
        },
    ];

    Ok(Json(ApiResponse::new(templates)))
}
