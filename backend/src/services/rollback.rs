/// Contract Rollback Service
/// 
/// Handles contract state rollback and transaction reversal capability
use serde::{Deserialize, Serialize};
use std::collections::HashMap;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ContractState {
    pub contract_id: String,
    pub storage: HashMap<String, Vec<u8>>,
    pub ledger_seq: u64,
    pub timestamp: i64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RollbackCheckpoint {
    pub checkpoint_id: String,
    pub states: HashMap<String, ContractState>,
    pub created_at: i64,
}

#[derive(Debug, Clone)]
pub struct RollbackManager {
    checkpoints: HashMap<String, RollbackCheckpoint>,
    active_transactions: HashMap<String, Vec<String>>,
}

impl RollbackManager {
    pub fn new() -> Self {
        Self {
            checkpoints: HashMap::new(),
            active_transactions: HashMap::new(),
        }
    }

    /// Create a checkpoint of current contract states
    pub fn create_checkpoint(
        &mut self,
        contract_ids: Vec<String>,
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

    /// Rollback contract to a specific checkpoint
    pub fn rollback_to_checkpoint(&self, checkpoint_id: &str) -> Result<RollbackCheckpoint, String> {
        self.checkpoints
            .get(checkpoint_id)
            .cloned()
            .ok_or_else(|| format!("Checkpoint {} not found", checkpoint_id))
    }

    /// Start transaction tracking for rollback capability
    pub fn begin_transaction(&mut self, tx_id: String) -> Result<(), String> {
        self.active_transactions.insert(tx_id, vec![]);
        Ok(())
    }

    /// Record a state change in the transaction
    pub fn record_state_change(&mut self, tx_id: &str, contract_id: String) -> Result<(), String> {
        if let Some(tx) = self.active_transactions.get_mut(tx_id) {
            tx.push(contract_id);
            Ok(())
        } else {
            Err(format!("Transaction {} not found", tx_id))
        }
    }

    /// Commit transaction
    pub fn commit_transaction(&mut self, tx_id: &str) -> Result<(), String> {
        self.active_transactions.remove(tx_id);
        Ok(())
    }

    /// Rollback transaction
    pub fn rollback_transaction(&mut self, tx_id: &str) -> Result<Vec<String>, String> {
        self.active_transactions
            .remove(tx_id)
            .ok_or_else(|| format!("Transaction {} not found", tx_id))
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
        
        // Validate checkpoint integrity
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
    fn test_rollback_transaction() {
        let mut manager = RollbackManager::new();
        manager.begin_transaction("tx1".to_string()).unwrap();
        manager.record_state_change("tx1", "contract1".to_string()).unwrap();
        let result = manager.rollback_transaction("tx1");
        assert!(result.is_ok());
    }
}
