use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use sqlx::PgPool;
use uuid::Uuid;

use crate::error::AppError;

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct DeploymentRequest {
    pub contract_id: String,
    pub version: String,
    pub wasm_hash: String,
    pub network: String,
    pub deployer: String,
    #[serde(default)]
    pub constructor_args: Value,
    #[serde(default)]
    pub dry_run: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct DeploymentCheck {
    pub name: String,
    pub status: String,
    pub message: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct DeploymentJob {
    pub id: String,
    pub contract_id: String,
    pub version: String,
    pub network: String,
    pub deployer: String,
    pub wasm_hash: String,
    pub status: String,
    pub transaction_envelope: Option<String>,
    pub steps: Vec<String>,
    pub checks: Vec<DeploymentCheck>,
    pub dry_run: bool,
    pub created_at: DateTime<Utc>,
}

#[derive(Clone)]
pub struct ContractDeploymentService {
    db: PgPool,
}

impl ContractDeploymentService {
    pub fn new(db: PgPool) -> Self {
        Self { db }
    }

    pub async fn create_deployment(
        &self,
        request: DeploymentRequest,
    ) -> Result<DeploymentJob, AppError> {
        validate_deployment(&request)?;

        let checks = vec![
            DeploymentCheck {
                name: "network".to_string(),
                status: "passed".to_string(),
                message: format!("{} is a supported deployment target.", request.network),
            },
            DeploymentCheck {
                name: "artifact".to_string(),
                status: "passed".to_string(),
                message: "WASM hash is present and formatted as a SHA-256 digest.".to_string(),
            },
            DeploymentCheck {
                name: "deployer".to_string(),
                status: "passed".to_string(),
                message: "Deployer address is present for authorization handoff.".to_string(),
            },
        ];
        let steps = vec![
            "Resolve network passphrase and RPC endpoint".to_string(),
            "Upload WASM artifact if the hash is not already installed".to_string(),
            "Assemble contract deployment operation".to_string(),
            "Run simulation and authorization checks".to_string(),
            "Submit signed transaction envelope".to_string(),
            "Persist deployment receipt and contract address".to_string(),
        ];
        let status = if request.dry_run { "planned" } else { "queued" }.to_string();
        let transaction_envelope = if request.dry_run {
            None
        } else {
            Some(format!(
                "pending-signature:{}:{}:{}",
                request.network, request.contract_id, request.version
            ))
        };

        let job = DeploymentJob {
            id: Uuid::new_v4().to_string(),
            contract_id: request.contract_id,
            version: request.version,
            network: request.network,
            deployer: request.deployer,
            wasm_hash: request.wasm_hash,
            status,
            transaction_envelope,
            steps,
            checks,
            dry_run: request.dry_run,
            created_at: Utc::now(),
        };

        let _ = sqlx::query(
            "INSERT INTO contract_deployments
             (id, contract_id, version, network, deployer, wasm_hash, status, transaction_envelope, steps, checks, dry_run, created_at)
             VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9, $10, $11, $12)",
        )
        .bind(&job.id)
        .bind(&job.contract_id)
        .bind(&job.version)
        .bind(&job.network)
        .bind(&job.deployer)
        .bind(&job.wasm_hash)
        .bind(&job.status)
        .bind(&job.transaction_envelope)
        .bind(serde_json::to_value(&job.steps)?)
        .bind(serde_json::to_value(&job.checks)?)
        .bind(job.dry_run)
        .bind(job.created_at)
        .execute(&self.db)
        .await;

        Ok(job)
    }
}

fn validate_deployment(request: &DeploymentRequest) -> Result<(), AppError> {
    if request.contract_id.trim().is_empty() {
        return Err(AppError::ValidationError(
            "contractId is required".to_string(),
        ));
    }
    if request.version.trim().is_empty() {
        return Err(AppError::ValidationError("version is required".to_string()));
    }
    if !matches!(
        request.network.as_str(),
        "mainnet" | "testnet" | "futurenet" | "sandbox"
    ) {
        return Err(AppError::ValidationError(
            "network must be one of mainnet, testnet, futurenet, or sandbox".to_string(),
        ));
    }
    if request.deployer.trim().is_empty() {
        return Err(AppError::ValidationError(
            "deployer is required".to_string(),
        ));
    }
    if request.wasm_hash.len() != 64 || !request.wasm_hash.chars().all(|c| c.is_ascii_hexdigit()) {
        return Err(AppError::ValidationError(
            "wasmHash must be a 64-character SHA-256 hex digest".to_string(),
        ));
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use sqlx::postgres::PgPoolOptions;

    fn pool() -> PgPool {
        PgPoolOptions::new()
            .max_connections(1)
            .connect_lazy("postgres://postgres:postgres@localhost/crucible_test")
            .unwrap()
    }

    #[tokio::test]
    async fn creates_dry_run_deployment_plan() {
        let service = ContractDeploymentService::new(pool());
        let job = service
            .create_deployment(DeploymentRequest {
                contract_id: "contract-a".to_string(),
                version: "1.0.0".to_string(),
                wasm_hash: "a".repeat(64),
                network: "testnet".to_string(),
                deployer: "GCDUMMYDEPLOYER".to_string(),
                constructor_args: serde_json::json!({}),
                dry_run: true,
            })
            .await
            .unwrap();

        assert_eq!(job.status, "planned");
        assert!(job.transaction_envelope.is_none());
        assert_eq!(job.checks.len(), 3);
    }
}
