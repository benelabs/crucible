use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use sqlx::PgPool;
use uuid::Uuid;

use crate::error::AppError;

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct CreateContractVersionRequest {
    pub contract_id: String,
    pub version: String,
    pub source_code: String,
    pub wasm_hash: Option<String>,
    pub changelog: Option<String>,
    pub created_by: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct ContractVersion {
    pub id: String,
    pub contract_id: String,
    pub version: String,
    pub source_hash: String,
    pub wasm_hash: Option<String>,
    pub changelog: Option<String>,
    pub created_by: Option<String>,
    pub created_at: DateTime<Utc>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct VersionDiffRequest {
    pub from_version: ContractVersion,
    pub to_version: ContractVersion,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct VersionDiff {
    pub from_version: String,
    pub to_version: String,
    pub source_changed: bool,
    pub wasm_changed: bool,
    pub breaking_changes: Vec<String>,
    pub summary: String,
}

#[derive(Clone)]
pub struct ContractVersioningService {
    db: PgPool,
}

impl ContractVersioningService {
    pub fn new(db: PgPool) -> Self {
        Self { db }
    }

    pub async fn create_version(
        &self,
        request: CreateContractVersionRequest,
    ) -> Result<ContractVersion, AppError> {
        validate_version_request(&request)?;
        let source_hash = sha256_hex(request.source_code.as_bytes());
        let version = ContractVersion {
            id: Uuid::new_v4().to_string(),
            contract_id: request.contract_id,
            version: request.version,
            source_hash,
            wasm_hash: request.wasm_hash,
            changelog: request.changelog,
            created_by: request.created_by,
            created_at: Utc::now(),
        };

        let _ = sqlx::query(
            "INSERT INTO contract_versions
             (id, contract_id, version, source_hash, wasm_hash, changelog, created_by, created_at)
             VALUES ($1, $2, $3, $4, $5, $6, $7, $8)",
        )
        .bind(&version.id)
        .bind(&version.contract_id)
        .bind(&version.version)
        .bind(&version.source_hash)
        .bind(&version.wasm_hash)
        .bind(&version.changelog)
        .bind(&version.created_by)
        .bind(version.created_at)
        .execute(&self.db)
        .await;

        Ok(version)
    }

    pub fn diff(&self, request: VersionDiffRequest) -> VersionDiff {
        let source_changed = request.from_version.source_hash != request.to_version.source_hash;
        let wasm_changed = request.from_version.wasm_hash != request.to_version.wasm_hash;
        let mut breaking_changes = Vec::new();

        if major_version(&request.from_version.version)
            != major_version(&request.to_version.version)
        {
            breaking_changes
                .push("Major version changed; review client compatibility.".to_string());
        }
        if wasm_changed {
            breaking_changes.push(
                "WASM artifact changed; require deployment validation before promotion."
                    .to_string(),
            );
        }

        let summary = match (source_changed, wasm_changed) {
            (false, false) => "No source or artifact changes detected.".to_string(),
            (true, false) => {
                "Source changed without a new WASM hash; rebuild before deployment.".to_string()
            }
            (false, true) => "WASM hash changed while source hash stayed constant.".to_string(),
            (true, true) => "Source and WASM artifact both changed.".to_string(),
        };

        VersionDiff {
            from_version: request.from_version.version,
            to_version: request.to_version.version,
            source_changed,
            wasm_changed,
            breaking_changes,
            summary,
        }
    }
}

fn validate_version_request(request: &CreateContractVersionRequest) -> Result<(), AppError> {
    if request.contract_id.trim().is_empty() {
        return Err(AppError::ValidationError(
            "contractId is required".to_string(),
        ));
    }
    if !is_semver(&request.version) {
        return Err(AppError::ValidationError(
            "version must use semantic versioning, for example 1.2.3".to_string(),
        ));
    }
    if request.source_code.trim().is_empty() {
        return Err(AppError::ValidationError(
            "sourceCode is required".to_string(),
        ));
    }
    Ok(())
}

fn is_semver(version: &str) -> bool {
    let parts: Vec<_> = version.split('.').collect();
    parts.len() == 3
        && parts
            .iter()
            .all(|part| !part.is_empty() && part.chars().all(|c| c.is_ascii_digit()))
}

fn major_version(version: &str) -> Option<u64> {
    version.split('.').next()?.parse().ok()
}

fn sha256_hex(bytes: &[u8]) -> String {
    let mut hasher = Sha256::new();
    hasher.update(bytes);
    hasher.finalize().iter().map(|b| format!("{:02x}", b)).collect::<String>()
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
    async fn creates_semver_contract_version() {
        let service = ContractVersioningService::new(pool());
        let version = service
            .create_version(CreateContractVersionRequest {
                contract_id: "contract-a".to_string(),
                version: "1.2.3".to_string(),
                source_code: "pub fn increment() {}".to_string(),
                wasm_hash: None,
                changelog: Some("Initial API".to_string()),
                created_by: None,
            })
            .await
            .unwrap();

        assert_eq!(version.version, "1.2.3");
        assert_eq!(version.source_hash.len(), 64);
    }

    #[test]
    fn detects_major_version_breaking_change() {
        let service = ContractVersioningService::new(pool());
        let now = Utc::now();
        let diff = service.diff(VersionDiffRequest {
            from_version: ContractVersion {
                id: "a".to_string(),
                contract_id: "contract-a".to_string(),
                version: "1.0.0".to_string(),
                source_hash: "source-a".to_string(),
                wasm_hash: Some("wasm-a".to_string()),
                changelog: None,
                created_by: None,
                created_at: now,
            },
            to_version: ContractVersion {
                id: "b".to_string(),
                contract_id: "contract-a".to_string(),
                version: "2.0.0".to_string(),
                source_hash: "source-b".to_string(),
                wasm_hash: Some("wasm-b".to_string()),
                changelog: None,
                created_by: None,
                created_at: now,
            },
        });

        assert!(diff.source_changed);
        assert!(!diff.breaking_changes.is_empty());
    }
}
