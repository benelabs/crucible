#![allow(dead_code)]

use chrono::Utc;
use serde::{Deserialize, Serialize};
use sqlx::PgPool;
use std::collections::HashMap;
use std::sync::Arc;
use tokio::sync::RwLock;
use uuid::Uuid;

#[derive(Debug, thiserror::Error)]
pub enum RegistryError {
    #[error("Database error: {0}")]
    Database(#[from] sqlx::Error),
    #[error("Unauthorized: caller '{caller}' does not hold ownership or deployer rights for contract '{contract_id}' (registered deployer: '{owner}')")]
    Unauthorized {
        caller: String,
        contract_id: String,
        owner: String,
    },
    #[error("Contract not found: {0}")]
    NotFound(String),
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CallerAuthContext {
    pub caller_address: String,
    pub organization_id: Option<String>,
    pub is_admin: bool,
    pub signature: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ContractEntry {
    pub id: String,
    pub name: String,
    pub address: String,
    pub network: String,
    pub abi_json: String,
    pub wasm_hash: String,
    pub created_at: chrono::DateTime<Utc>,
    pub updated_at: chrono::DateTime<Utc>,
    pub tags: Vec<String>,
    #[serde(default)]
    pub deployer_address: String,
    #[serde(default)]
    pub organization_id: Option<String>,
}

#[derive(Clone)]
pub struct ContractRegistry {
    #[allow(dead_code)]
    db: PgPool,
    entries: Arc<RwLock<HashMap<String, ContractEntry>>>,
}

impl ContractRegistry {
    pub fn new(db: PgPool) -> Self {
        Self {
            db,
            entries: Arc::new(RwLock::new(HashMap::new())),
        }
    }

    /// Verifies that caller holds administrative privileges, is the original deployer, or shares organization.
    pub fn check_authorization(
        &self,
        auth: &CallerAuthContext,
        entry: &ContractEntry,
    ) -> Result<(), RegistryError> {
        if auth.is_admin {
            return Ok(());
        }

        if !entry.deployer_address.is_empty() && auth.caller_address == entry.deployer_address {
            return Ok(());
        }

        if let (Some(caller_org), Some(contract_org)) =
            (&auth.organization_id, &entry.organization_id)
        {
            if caller_org == contract_org && !caller_org.is_empty() {
                return Ok(());
            }
        }

        Err(RegistryError::Unauthorized {
            caller: auth.caller_address.clone(),
            contract_id: entry.id.clone(),
            owner: entry.deployer_address.clone(),
        })
    }

    /// Register a contract without auth context (backward-compatible)
    pub async fn register(
        &self,
        name: &str,
        address: &str,
        network: &str,
        abi_json: &str,
        wasm_hash: &str,
        tags: Vec<String>,
    ) -> Result<ContractEntry, sqlx::Error> {
        let now = Utc::now();
        let entry = ContractEntry {
            id: Uuid::new_v4().to_string(),
            name: name.to_string(),
            address: address.to_string(),
            network: network.to_string(),
            abi_json: abi_json.to_string(),
            wasm_hash: wasm_hash.to_string(),
            created_at: now,
            updated_at: now,
            tags,
            deployer_address: address.to_string(),
            organization_id: None,
        };

        let mut store = self.entries.write().await;
        store.insert(entry.id.clone(), entry.clone());
        Ok(entry)
    }

    /// Register a contract with explicit caller authorization context
    pub async fn register_with_auth(
        &self,
        auth: &CallerAuthContext,
        name: &str,
        address: &str,
        network: &str,
        abi_json: &str,
        wasm_hash: &str,
        tags: Vec<String>,
    ) -> Result<ContractEntry, RegistryError> {
        let mut store = self.entries.write().await;

        if let Some(existing) = store.values().find(|e| e.address == address) {
            self.check_authorization(auth, existing)?;
        }

        let now = Utc::now();
        let entry = ContractEntry {
            id: Uuid::new_v4().to_string(),
            name: name.to_string(),
            address: address.to_string(),
            network: network.to_string(),
            abi_json: abi_json.to_string(),
            wasm_hash: wasm_hash.to_string(),
            created_at: now,
            updated_at: now,
            tags,
            deployer_address: auth.caller_address.clone(),
            organization_id: auth.organization_id.clone(),
        };

        store.insert(entry.id.clone(), entry.clone());
        Ok(entry)
    }

    /// Update contract metadata with ownership authorization check
    pub async fn update_metadata(
        &self,
        auth: &CallerAuthContext,
        id: &str,
        new_name: Option<&str>,
        new_abi_json: Option<&str>,
        new_tags: Option<Vec<String>>,
    ) -> Result<ContractEntry, RegistryError> {
        let mut store = self.entries.write().await;
        let entry = store
            .get_mut(id)
            .ok_or_else(|| RegistryError::NotFound(id.to_string()))?;

        self.check_authorization(auth, entry)?;

        if let Some(name) = new_name {
            entry.name = name.to_string();
        }
        if let Some(abi) = new_abi_json {
            entry.abi_json = abi.to_string();
        }
        if let Some(tags) = new_tags {
            entry.tags = tags;
        }
        entry.updated_at = Utc::now();

        Ok(entry.clone())
    }

    /// Deregister a contract with ownership authorization check
    pub async fn deregister_with_auth(
        &self,
        auth: &CallerAuthContext,
        id: &str,
    ) -> Result<bool, RegistryError> {
        let mut store = self.entries.write().await;
        let entry = store
            .get(id)
            .ok_or_else(|| RegistryError::NotFound(id.to_string()))?;

        self.check_authorization(auth, entry)?;

        store.remove(id);
        Ok(true)
    }

    pub async fn get(&self, id: &str) -> Result<Option<ContractEntry>, sqlx::Error> {
        let store = self.entries.read().await;
        Ok(store.get(id).cloned())
    }

    pub async fn list(&self, network: Option<&str>) -> Result<Vec<ContractEntry>, sqlx::Error> {
        let store = self.entries.read().await;
        Ok(store
            .values()
            .filter(|e| network.is_none_or(|n| e.network == n))
            .cloned()
            .collect())
    }

    pub async fn deregister(&self, id: &str) -> Result<bool, sqlx::Error> {
        let mut store = self.entries.write().await;
        Ok(store.remove(id).is_some())
    }

    pub async fn search(&self, query: &str) -> Result<Vec<ContractEntry>, sqlx::Error> {
        let store = self.entries.read().await;
        let q = query.to_lowercase();
        Ok(store
            .values()
            .filter(|e| e.name.to_lowercase().contains(&q) || e.address.to_lowercase().contains(&q))
            .cloned()
            .collect())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn test_contract_registration_and_update_by_deployer() {
        let pool = sqlx::PgPool::connect_lazy("postgres://localhost/test").unwrap();
        let registry = ContractRegistry::new(pool);
        let deployer_auth = CallerAuthContext {
            caller_address: "GDEPLOYER1111111111111111111111111111111111111111111111".to_string(),
            organization_id: Some("org_alpha".to_string()),
            is_admin: false,
            signature: None,
        };

        let entry = registry
            .register_with_auth(
                &deployer_auth,
                "TokenA",
                "CAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAWH4",
                "testnet",
                "{}",
                "hash123",
                vec!["defi".to_string()],
            )
            .await
            .unwrap();

        assert_eq!(entry.deployer_address, deployer_auth.caller_address);

        // Update by deployer succeeds
        let updated = registry
            .update_metadata(
                &deployer_auth,
                &entry.id,
                Some("TokenA_v2"),
                None,
                Some(vec!["defi".to_string(), "token".to_string()]),
            )
            .await
            .unwrap();

        assert_eq!(updated.name, "TokenA_v2");
        assert_eq!(updated.tags.len(), 2);
    }

    #[tokio::test]
    async fn test_contract_metadata_overwrite_rejected_for_unauthorized_caller() {
        let pool = sqlx::PgPool::connect_lazy("postgres://localhost/test").unwrap();
        let registry = ContractRegistry::new(pool);
        let deployer_auth = CallerAuthContext {
            caller_address: "GDEPLOYER1111111111111111111111111111111111111111111111".to_string(),
            organization_id: Some("org_alpha".to_string()),
            is_admin: false,
            signature: None,
        };

        let entry = registry
            .register_with_auth(
                &deployer_auth,
                "VaultContract",
                "CBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBB",
                "testnet",
                "{}",
                "hash_vault",
                vec![],
            )
            .await
            .unwrap();

        // Malicious unauthorized caller attempts to overwrite metadata
        let attacker_auth = CallerAuthContext {
            caller_address: "GATTACKER9999999999999999999999999999999999999999999999".to_string(),
            organization_id: Some("org_attacker".to_string()),
            is_admin: false,
            signature: None,
        };

        let err = registry
            .update_metadata(
                &attacker_auth,
                &entry.id,
                Some("HijackedContract"),
                None,
                None,
            )
            .await;

        assert!(err.is_err());
        match err.unwrap_err() {
            RegistryError::Unauthorized { caller, owner, .. } => {
                assert_eq!(caller, attacker_auth.caller_address);
                assert_eq!(owner, deployer_auth.caller_address);
            }
            other => panic!("Expected Unauthorized, got {:?}", other),
        }

        // Attempting to deregister as attacker also fails
        let dereg_err = registry
            .deregister_with_auth(&attacker_auth, &entry.id)
            .await;
        assert!(dereg_err.is_err());

        // Admin can update
        let admin_auth = CallerAuthContext {
            caller_address: "GADMIN000000000000000000000000000000000000000000000000".to_string(),
            organization_id: None,
            is_admin: true,
            signature: None,
        };
        let admin_update = registry
            .update_metadata(
                &admin_auth,
                &entry.id,
                Some("Vault_AdminUpdated"),
                None,
                None,
            )
            .await;
        assert!(admin_update.is_ok());
    }
}
