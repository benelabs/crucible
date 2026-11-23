/// Contract Rollback Service
///
/// Handles contract state rollback and transaction reversal capability across
/// all Soroban contract storage tiers: instance, temporary, and persistent.
use serde::{Deserialize, Serialize};
use std::collections::HashMap;

/// Storage tier / keyspace for Soroban smart contracts.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum StorageTier {
    /// Instance storage tied directly to contract instance lifetime.
    Instance,
    /// Temporary storage with short-lived TTL, discarded if expired.
    Temporary,
    /// Persistent storage maintained independently across ledger boundaries.
    Persistent,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct ContractState {
    pub contract_id: String,
    /// Instance storage entries (retained as 'storage' for backward compatibility).
    pub storage: HashMap<String, Vec<u8>>,
    /// Temporary storage keyspace restored during deployment rollbacks.
    #[serde(default)]
    pub temporary_storage: HashMap<String, Vec<u8>>,
    /// Persistent storage keyspace restored during deployment rollbacks.
    #[serde(default)]
    pub persistent_storage: HashMap<String, Vec<u8>>,
    /// WASM contract bytecode snapshot.
    #[serde(default)]
    pub wasm_bytecode: Option<Vec<u8>>,
    pub ledger_seq: u64,
    pub timestamp: i64,
}

impl ContractState {
    /// Create new basic contract state
    pub fn new(contract_id: String, ledger_seq: u64, timestamp: i64) -> Self {
        Self {
            contract_id,
            storage: HashMap::new(),
            temporary_storage: HashMap::new(),
            persistent_storage: HashMap::new(),
            wasm_bytecode: None,
            ledger_seq,
            timestamp,
        }
    }

    /// Create contract state with instance storage
    pub fn with_storage(
        contract_id: String,
        storage: HashMap<String, Vec<u8>>,
        ledger_seq: u64,
        timestamp: i64,
    ) -> Self {
        Self {
            contract_id,
            storage,
            temporary_storage: HashMap::new(),
            persistent_storage: HashMap::new(),
            wasm_bytecode: None,
            ledger_seq,
            timestamp,
        }
    }

    /// Create complete contract state snapshot with all storage tiers and bytecode
    pub fn with_all_tiers(
        contract_id: String,
        instance_storage: HashMap<String, Vec<u8>>,
        temporary_storage: HashMap<String, Vec<u8>>,
        persistent_storage: HashMap<String, Vec<u8>>,
        wasm_bytecode: Option<Vec<u8>>,
        ledger_seq: u64,
        timestamp: i64,
    ) -> Self {
        Self {
            contract_id,
            storage: instance_storage,
            temporary_storage,
            persistent_storage,
            wasm_bytecode,
            ledger_seq,
            timestamp,
        }
    }

    /// Access reference to specific storage tier
    pub fn get_storage_tier(&self, tier: StorageTier) -> &HashMap<String, Vec<u8>> {
        match tier {
            StorageTier::Instance => &self.storage,
            StorageTier::Temporary => &self.temporary_storage,
            StorageTier::Persistent => &self.persistent_storage,
        }
    }

    /// Access mutable reference to specific storage tier
    pub fn get_storage_tier_mut(&mut self, tier: StorageTier) -> &mut HashMap<String, Vec<u8>> {
        match tier {
            StorageTier::Instance => &mut self.storage,
            StorageTier::Temporary => &mut self.temporary_storage,
            StorageTier::Persistent => &mut self.persistent_storage,
        }
    }

    /// Restore all storage tiers and bytecode from another state snapshot
    pub fn restore_from(&mut self, source: &ContractState) {
        self.storage = source.storage.clone();
        self.temporary_storage = source.temporary_storage.clone();
        self.persistent_storage = source.persistent_storage.clone();
        self.wasm_bytecode = source.wasm_bytecode.clone();
        self.ledger_seq = source.ledger_seq;
        self.timestamp = source.timestamp;
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RollbackCheckpoint {
    pub checkpoint_id: String,
    pub states: HashMap<String, ContractState>,
    pub created_at: i64,
}

impl RollbackCheckpoint {
    pub fn get_contract_state(&self, contract_id: &str) -> Option<&ContractState> {
        self.states.get(contract_id)
    }

    pub fn contains_contract(&self, contract_id: &str) -> bool {
        self.states.contains_key(contract_id)
    }
}

/// Tracks modification to a specific key in a storage tier during a transaction/migration.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct StorageEntryModification {
    pub contract_id: String,
    pub tier: StorageTier,
    pub key: String,
    pub previous_value: Option<Vec<u8>>,
}

#[derive(Debug, Clone, Default)]
pub struct RollbackManager {
    checkpoints: HashMap<String, RollbackCheckpoint>,
    active_transactions: HashMap<String, Vec<String>>,
    transaction_modifications: HashMap<String, Vec<StorageEntryModification>>,
    live_states: HashMap<String, ContractState>,
}

impl RollbackManager {
    pub fn new() -> Self {
        Self::default()
    }

    /// Register or update a live contract state
    pub fn register_live_state(&mut self, state: ContractState) {
        self.live_states.insert(state.contract_id.clone(), state);
    }

    /// Retrieve live contract state
    pub fn get_live_state(&self, contract_id: &str) -> Option<&ContractState> {
        self.live_states.get(contract_id)
    }

    /// Retrieve mutable live contract state
    pub fn get_live_state_mut(&mut self, contract_id: &str) -> Option<&mut ContractState> {
        self.live_states.get_mut(contract_id)
    }

    /// Snapshot live contract states across all storage tiers (instance, temporary, persistent)
    pub fn snapshot_contracts(&mut self, contract_ids: &[String]) -> Result<String, String> {
        let mut states = HashMap::new();
        for id in contract_ids {
            if let Some(state) = self.live_states.get(id) {
                states.insert(id.clone(), state.clone());
            } else {
                return Err(format!(
                    "Contract {} not found in live states for snapshot",
                    id
                ));
            }
        }
        self.create_checkpoint(contract_ids.to_vec(), states)
    }

    /// Create a checkpoint of contract states (capturing all storage tiers)
    pub fn create_checkpoint(
        &mut self,
        _contract_ids: Vec<String>,
        states: HashMap<String, ContractState>,
    ) -> Result<String, String> {
        let checkpoint_id = format!("cp_{}", chrono::Utc::now().timestamp_millis());

        let checkpoint = RollbackCheckpoint {
            checkpoint_id: checkpoint_id.clone(),
            states,
            created_at: chrono::Utc::now().timestamp(),
        };

        self.checkpoints.insert(checkpoint_id.clone(), checkpoint);
        Ok(checkpoint_id)
    }

    /// Rollback contracts to a specific checkpoint, restoring all storage tiers
    /// (instance, temporary, and persistent storage) as well as WASM bytecode.
    pub fn rollback_to_checkpoint(
        &mut self,
        checkpoint_id: &str,
    ) -> Result<RollbackCheckpoint, String> {
        let checkpoint = self
            .checkpoints
            .get(checkpoint_id)
            .cloned()
            .ok_or_else(|| format!("Checkpoint {} not found", checkpoint_id))?;

        // Restore each contract in live_states to its exact checkpointed state
        for (contract_id, saved_state) in &checkpoint.states {
            if let Some(live) = self.live_states.get_mut(contract_id) {
                live.restore_from(saved_state);
            } else {
                self.live_states
                    .insert(contract_id.clone(), saved_state.clone());
            }
        }

        Ok(checkpoint)
    }

    /// Rollback an individual contract to a specific checkpoint
    pub fn rollback_contract(
        &mut self,
        checkpoint_id: &str,
        contract_id: &str,
    ) -> Result<ContractState, String> {
        let checkpoint = self
            .checkpoints
            .get(checkpoint_id)
            .ok_or_else(|| format!("Checkpoint {} not found", checkpoint_id))?;

        let saved_state = checkpoint.states.get(contract_id).cloned().ok_or_else(|| {
            format!(
                "Contract {} not found in checkpoint {}",
                contract_id, checkpoint_id
            )
        })?;

        if let Some(live) = self.live_states.get_mut(contract_id) {
            live.restore_from(&saved_state);
        } else {
            self.live_states
                .insert(contract_id.to_string(), saved_state.clone());
        }

        Ok(saved_state)
    }

    /// Start transaction tracking for rollback capability
    pub fn begin_transaction(&mut self, tx_id: String) -> Result<(), String> {
        self.active_transactions.insert(tx_id.clone(), vec![]);
        self.transaction_modifications.insert(tx_id, vec![]);
        Ok(())
    }

    /// Record a state change in the transaction (contract level)
    pub fn record_state_change(&mut self, tx_id: &str, contract_id: String) -> Result<(), String> {
        if let Some(tx) = self.active_transactions.get_mut(tx_id) {
            tx.push(contract_id);
            Ok(())
        } else {
            Err(format!("Transaction {} not found", tx_id))
        }
    }

    /// Record modification to a specific storage tier entry within a transaction
    pub fn record_storage_modification(
        &mut self,
        tx_id: &str,
        contract_id: String,
        tier: StorageTier,
        key: String,
        previous_value: Option<Vec<u8>>,
    ) -> Result<(), String> {
        if let Some(mods) = self.transaction_modifications.get_mut(tx_id) {
            mods.push(StorageEntryModification {
                contract_id: contract_id.clone(),
                tier,
                key,
                previous_value,
            });
            if let Some(tx) = self.active_transactions.get_mut(tx_id) {
                if !tx.contains(&contract_id) {
                    tx.push(contract_id);
                }
            }
            Ok(())
        } else {
            Err(format!("Transaction {} not found", tx_id))
        }
    }

    /// Commit transaction, discarding tracking history
    pub fn commit_transaction(&mut self, tx_id: &str) -> Result<(), String> {
        self.active_transactions.remove(tx_id);
        self.transaction_modifications.remove(tx_id);
        Ok(())
    }

    /// Rollback transaction: reverts modified entries across all storage tiers
    /// (instance, temporary, and persistent) to their previous values.
    pub fn rollback_transaction(&mut self, tx_id: &str) -> Result<Vec<String>, String> {
        let contracts = self
            .active_transactions
            .remove(tx_id)
            .ok_or_else(|| format!("Transaction {} not found", tx_id))?;

        if let Some(mods) = self.transaction_modifications.remove(tx_id) {
            // Apply modifications in reverse order
            for m in mods.into_iter().rev() {
                if let Some(live) = self.live_states.get_mut(&m.contract_id) {
                    let map = live.get_storage_tier_mut(m.tier);
                    match m.previous_value {
                        Some(val) => {
                            map.insert(m.key, val);
                        }
                        None => {
                            map.remove(&m.key);
                        }
                    }
                }
            }
        }

        Ok(contracts)
    }

    /// Get list of all checkpoints
    pub fn list_checkpoints(&self) -> Vec<String> {
        self.checkpoints.keys().cloned().collect()
    }

    /// Validate rollback safety
    pub fn validate_rollback(&self, checkpoint_id: &str) -> Result<bool, String> {
        if !self.checkpoints.contains_key(checkpoint_id) {
            return Err(format!("Checkpoint {} not found", checkpoint_id));
        }

        // Validate checkpoint integrity and presence of states
        if let Some(checkpoint) = self.checkpoints.get(checkpoint_id) {
            Ok(!checkpoint.states.is_empty())
        } else {
            Ok(false)
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_create_checkpoint() {
        let mut manager = RollbackManager::new();
        let states = HashMap::new();
        let result = manager.create_checkpoint(vec!["contract1".to_string()], states);
        assert!(result.is_ok());
    }

    #[test]
    fn test_begin_transaction() {
        let mut manager = RollbackManager::new();
        let result = manager.begin_transaction("tx1".to_string());
        assert!(result.is_ok());
    }

    #[test]
    fn test_record_state_change() {
        let mut manager = RollbackManager::new();
        manager.begin_transaction("tx1".to_string()).unwrap();
        let result = manager.record_state_change("tx1", "contract1".to_string());
        assert!(result.is_ok());
    }

    #[test]
    fn test_commit_transaction() {
        let mut manager = RollbackManager::new();
        manager.begin_transaction("tx1".to_string()).unwrap();
        let result = manager.commit_transaction("tx1");
        assert!(result.is_ok());
    }

    #[test]
    fn test_rollback_transaction_basic() {
        let mut manager = RollbackManager::new();
        manager.begin_transaction("tx1".to_string()).unwrap();
        manager
            .record_state_change("tx1", "contract1".to_string())
            .unwrap();
        let result = manager.rollback_transaction("tx1");
        assert!(result.is_ok());
    }

    #[test]
    fn test_temporary_storage_rollback_restoration() {
        let mut manager = RollbackManager::new();

        let mut instance = HashMap::new();
        instance.insert("admin".to_string(), vec![1, 2, 3]);

        let mut temporary = HashMap::new();
        temporary.insert("temp_session_key".to_string(), vec![0xAA, 0xBB]);
        temporary.insert("nonce".to_string(), vec![42]);

        let mut persistent = HashMap::new();
        persistent.insert("user_balance".to_string(), vec![0x10, 0x27]);

        let contract_id = "CA3D5KRYM6CB7OWQ6TWYRR3Z4T7ZQVNO44PPINP5U2HGACGLWAVE5E52".to_string();
        let initial_state = ContractState::with_all_tiers(
            contract_id.clone(),
            instance,
            temporary.clone(),
            persistent.clone(),
            Some(vec![0x00, 0x61, 0x73, 0x6d, 0x01, 0x00, 0x00, 0x00]),
            1000,
            1700000000,
        );

        manager.register_live_state(initial_state);

        // Snapshot prior to failed migration
        let checkpoint_id = manager
            .snapshot_contracts(std::slice::from_ref(&contract_id))
            .expect("snapshot succeeds");

        // Simulate failed migration mutating temporary storage and persistent storage
        {
            let live = manager.get_live_state_mut(&contract_id).unwrap();
            live.temporary_storage
                .insert("temp_session_key".to_string(), vec![0xFF, 0xFF]);
            live.temporary_storage
                .insert("corrupted_temp_key".to_string(), vec![99]);
            live.persistent_storage
                .insert("user_balance".to_string(), vec![0]);
            live.storage.insert("admin".to_string(), vec![0]);
        }

        // Verify corrupted state before rollback
        let corrupted = manager.get_live_state(&contract_id).unwrap();
        assert_eq!(
            corrupted.temporary_storage.get("temp_session_key").unwrap(),
            &vec![0xFF, 0xFF]
        );
        assert!(corrupted
            .temporary_storage
            .contains_key("corrupted_temp_key"));

        // Execute rollback to checkpoint
        let restored_cp = manager
            .rollback_to_checkpoint(&checkpoint_id)
            .expect("rollback succeeds");
        assert!(restored_cp.contains_contract(&contract_id));

        // Verify that temporary, persistent, instance storage and wasm bytecode are 100% restored
        let restored = manager.get_live_state(&contract_id).unwrap();
        assert_eq!(
            restored.temporary_storage.get("temp_session_key").unwrap(),
            &vec![0xAA, 0xBB],
            "Temporary storage key must be restored"
        );
        assert_eq!(
            restored.temporary_storage.get("nonce").unwrap(),
            &vec![42],
            "Temporary storage nonce must be preserved"
        );
        assert!(
            !restored
                .temporary_storage
                .contains_key("corrupted_temp_key"),
            "Mutated temporary keys added by failed migration must not exist"
        );
        assert_eq!(
            restored.persistent_storage.get("user_balance").unwrap(),
            &vec![0x10, 0x27],
            "Persistent storage balance must be restored"
        );
        assert_eq!(
            restored.storage.get("admin").unwrap(),
            &vec![1, 2, 3],
            "Instance storage must be restored"
        );
        assert_eq!(
            restored.wasm_bytecode.as_ref().unwrap(),
            &vec![0x00, 0x61, 0x73, 0x6d, 0x01, 0x00, 0x00, 0x00]
        );
    }

    #[test]
    fn test_fine_grained_transaction_rollback_across_tiers() {
        let mut manager = RollbackManager::new();
        let contract_id = "contract_beta".to_string();

        let mut temporary = HashMap::new();
        temporary.insert("auth_token".to_string(), vec![1, 1, 1]);

        let state = ContractState::with_all_tiers(
            contract_id.clone(),
            HashMap::new(),
            temporary,
            HashMap::new(),
            None,
            500,
            1700000000,
        );
        manager.register_live_state(state);

        // Begin transaction
        manager
            .begin_transaction("migration_tx".to_string())
            .unwrap();

        // Mutate temporary storage in transaction
        manager
            .record_storage_modification(
                "migration_tx",
                contract_id.clone(),
                StorageTier::Temporary,
                "auth_token".to_string(),
                Some(vec![1, 1, 1]),
            )
            .unwrap();

        // Update live state to new value
        manager
            .get_live_state_mut(&contract_id)
            .unwrap()
            .temporary_storage
            .insert("auth_token".to_string(), vec![2, 2, 2]);

        // Add a brand new temporary entry
        manager
            .record_storage_modification(
                "migration_tx",
                contract_id.clone(),
                StorageTier::Temporary,
                "transient_flag".to_string(),
                None, // was absent
            )
            .unwrap();
        manager
            .get_live_state_mut(&contract_id)
            .unwrap()
            .temporary_storage
            .insert("transient_flag".to_string(), vec![1]);

        assert_eq!(
            manager
                .get_live_state(&contract_id)
                .unwrap()
                .temporary_storage
                .get("auth_token")
                .unwrap(),
            &vec![2, 2, 2]
        );

        // Rollback transaction
        let affected = manager.rollback_transaction("migration_tx").unwrap();
        assert!(affected.contains(&contract_id));

        // Verify live state reverted
        let live = manager.get_live_state(&contract_id).unwrap();
        assert_eq!(
            live.temporary_storage.get("auth_token").unwrap(),
            &vec![1, 1, 1],
            "Previous value in temporary storage restored"
        );
        assert!(
            !live.temporary_storage.contains_key("transient_flag"),
            "Newly inserted key removed on rollback"
        );
    }
}
