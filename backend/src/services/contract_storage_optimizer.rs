use std::collections::HashMap;
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use sqlx::PgPool;

use crate::error::AppError;

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct StorageOptimizationInput {
    pub contract_id: String,
    pub source_code: String,
    pub target_network: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct StorageRecommendation {
    pub line: u32,
    pub current_storage: String,
    pub recommended_storage: String,
    pub severity: String,
    pub reason: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct StorageOptimizationReport {
    pub contract_id: String,
    pub target_network: String,
    pub storage_entries_estimate: u32,
    pub estimated_rent_savings_percent: f64,
    pub ttl_strategy: String,
    pub recommendations: Vec<StorageRecommendation>,
    pub generated_at: DateTime<Utc>,
}

// ---------------------------------------------------------------------------
// Storage Diff Engine Types (#860)
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub enum StorageChangeType {
    Added,
    Modified,
    Deleted,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct StorageDiffEntry {
    pub key: String,
    pub old_value: Option<String>,
    pub new_value: Option<String>,
    pub change_type: StorageChangeType,
    pub visual_indicator: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct StorageDiffReport {
    pub contract_id: String,
    pub from_sequence: u64,
    pub to_sequence: u64,
    pub changes: Vec<StorageDiffEntry>,
    pub added_count: usize,
    pub modified_count: usize,
    pub deleted_count: usize,
    pub generated_at: DateTime<Utc>,
}

#[derive(Clone)]
pub struct ContractStorageOptimizer {
    db: PgPool,
}

impl ContractStorageOptimizer {
    pub fn new(db: PgPool) -> Self {
        Self { db }
    }

    /// Compare two ledger sequence storage maps and generate fine-grained key-value diffs.
    pub fn compare_storage_diff(
        contract_id: &str,
        from_sequence: u64,
        to_sequence: u64,
        before_state: &HashMap<String, String>,
        after_state: &HashMap<String, String>,
    ) -> StorageDiffReport {
        let mut changes = Vec::new();
        let mut added_count = 0;
        let mut modified_count = 0;
        let mut deleted_count = 0;

        // Check modified & deleted
        for (key, old_val) in before_state {
            match after_state.get(key) {
                Some(new_val) => {
                    if old_val != new_val {
                        modified_count += 1;
                        changes.push(StorageDiffEntry {
                            key: key.clone(),
                            old_value: Some(old_val.clone()),
                            new_value: Some(new_val.clone()),
                            change_type: StorageChangeType::Modified,
                            visual_indicator: format!("~ Modified: {} -> {}", old_val, new_val),
                        });
                    }
                }
                None => {
                    deleted_count += 1;
                    changes.push(StorageDiffEntry {
                        key: key.clone(),
                        old_value: Some(old_val.clone()),
                        new_value: None,
                        change_type: StorageChangeType::Deleted,
                        visual_indicator: format!("- Deleted: {}", old_val),
                    });
                }
            }
        }

        // Check added
        for (key, new_val) in after_state {
            if !before_state.contains_key(key) {
                added_count += 1;
                changes.push(StorageDiffEntry {
                    key: key.clone(),
                    old_value: None,
                    new_value: Some(new_val.clone()),
                    change_type: StorageChangeType::Added,
                    visual_indicator: format!("+ Added: {}", new_val),
                });
            }
        }

        changes.sort_by(|a, b| a.key.cmp(&b.key));

        StorageDiffReport {
            contract_id: contract_id.to_string(),
            from_sequence,
            to_sequence,
            changes,
            added_count,
            modified_count,
            deleted_count,
            generated_at: Utc::now(),
        }
    }

    pub async fn optimize(
        &self,
        input: StorageOptimizationInput,
    ) -> Result<StorageOptimizationReport, AppError> {
        if input.contract_id.trim().is_empty() {
            return Err(AppError::ValidationError(
                "contractId is required".to_string(),
            ));
        }
        if input.source_code.trim().is_empty() {
            return Err(AppError::ValidationError(
                "sourceCode is required".to_string(),
            ));
        }

        let recommendations = analyze_storage_lines(&input.source_code);
        let storage_entries_estimate = input
            .source_code
            .lines()
            .filter(|line| {
                line.contains("env.storage()") && (line.contains(".set(") || line.contains(".get("))
            })
            .count() as u32;
        let persistent_entries = input
            .source_code
            .lines()
            .filter(|line| line.contains("storage().persistent()"))
            .count() as f64;
        let high_impact = recommendations
            .iter()
            .filter(|rec| rec.severity == "high")
            .count() as f64;
        let estimated_rent_savings_percent = if persistent_entries == 0.0 {
            0.0
        } else {
            ((high_impact / persistent_entries) * 30.0).min(30.0)
        };

        let ttl_strategy = if input.source_code.contains("extend_ttl")
            || input.source_code.contains("bump")
        {
            "Explicit TTL management detected; keep TTL extension close to write paths.".to_string()
        } else if persistent_entries > 0.0 {
            "Add TTL extension for persistent entries that must survive ledger expiration."
                .to_string()
        } else {
            "Prefer temporary storage for short-lived workflow data and instance storage for contract configuration.".to_string()
        };

        let report = StorageOptimizationReport {
            contract_id: input.contract_id,
            target_network: input
                .target_network
                .unwrap_or_else(|| "testnet".to_string()),
            storage_entries_estimate,
            estimated_rent_savings_percent,
            ttl_strategy,
            recommendations,
            generated_at: Utc::now(),
        };

        let _ = sqlx::query(
            "INSERT INTO contract_storage_optimizations
             (contract_id, target_network, storage_entries_estimate, estimated_rent_savings_percent, ttl_strategy, recommendations, generated_at)
             VALUES ($1, $2, $3, $4, $5, $6, $7)",
        )
        .bind(&report.contract_id)
        .bind(&report.target_network)
        .bind(i64::from(report.storage_entries_estimate))
        .bind(report.estimated_rent_savings_percent)
        .bind(&report.ttl_strategy)
        .bind(serde_json::to_value(&report.recommendations)?)
        .bind(report.generated_at)
        .execute(&self.db)
        .await;

        Ok(report)
    }
}

fn analyze_storage_lines(source_code: &str) -> Vec<StorageRecommendation> {
    let mut recommendations = Vec::new();

    for (idx, line) in source_code.lines().enumerate() {
        let normalized = line.trim().to_lowercase();
        if !normalized.contains("storage().") {
            continue;
        }

        let line_no = idx as u32 + 1;
        if normalized.contains("storage().persistent()")
            && (normalized.contains("cache")
                || normalized.contains("session")
                || normalized.contains("temp")
                || normalized.contains("nonce"))
        {
            recommendations.push(StorageRecommendation {
                line: line_no,
                current_storage: "persistent".to_string(),
                recommended_storage: "temporary".to_string(),
                severity: "high".to_string(),
                reason: "Short-lived data should not pay persistent storage rent.".to_string(),
            });
        }

        if normalized.contains("storage().instance()")
            && (normalized.contains("balance")
                || normalized.contains("allowance")
                || normalized.contains("history")
                || normalized.contains("claim"))
        {
            recommendations.push(StorageRecommendation {
                line: line_no,
                current_storage: "instance".to_string(),
                recommended_storage: "persistent".to_string(),
                severity: "medium".to_string(),
                reason: "Per-account or growing state is safer outside instance storage."
                    .to_string(),
            });
        }

        if normalized.contains(".set(")
            && (normalized.contains("vec<")
                || normalized.contains("map<")
                || normalized.contains("bytesn<"))
        {
            recommendations.push(StorageRecommendation {
                line: line_no,
                current_storage: "unknown".to_string(),
                recommended_storage: "chunked".to_string(),
                severity: "high".to_string(),
                reason: "Large values should be split across bounded keys to avoid rent and size spikes.".to_string(),
            });
        }
    }

    recommendations
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
    async fn recommends_temporary_storage_for_session_state() {
        let service = ContractStorageOptimizer::new(pool());
        let report = service
            .optimize(StorageOptimizationInput {
                contract_id: "contract-a".to_string(),
                target_network: None,
                source_code: "env.storage().persistent().set(&session_key, &nonce);".to_string(),
            })
            .await
            .unwrap();

        assert_eq!(report.storage_entries_estimate, 1);
        assert_eq!(report.recommendations[0].recommended_storage, "temporary");
    }

    #[test]
    fn compares_storage_diff_accurately() {
        let mut before = HashMap::new();
        before.insert("balance:user1".to_string(), "100".to_string());
        before.insert("balance:user2".to_string(), "500".to_string());
        before.insert("admin".to_string(), "GB345".to_string());

        let mut after = HashMap::new();
        after.insert("balance:user1".to_string(), "200".to_string()); // Modified
        after.insert("admin".to_string(), "GB345".to_string());       // Unchanged
        after.insert("balance:user3".to_string(), "50".to_string());  // Added
        // user2 deleted

        let report = ContractStorageOptimizer::compare_storage_diff(
            "C123CONTRACT",
            1000,
            1005,
            &before,
            &after,
        );

        assert_eq!(report.contract_id, "C123CONTRACT");
        assert_eq!(report.from_sequence, 1000);
        assert_eq!(report.to_sequence, 1005);
        assert_eq!(report.added_count, 1);
        assert_eq!(report.modified_count, 1);
        assert_eq!(report.deleted_count, 1);
        assert_eq!(report.changes.len(), 3);

        let added = report.changes.iter().find(|c| c.change_type == StorageChangeType::Added).unwrap();
        assert_eq!(added.key, "balance:user3");
        assert_eq!(added.new_value, Some("50".to_string()));

        let modified = report.changes.iter().find(|c| c.change_type == StorageChangeType::Modified).unwrap();
        assert_eq!(modified.key, "balance:user1");
        assert_eq!(modified.old_value, Some("100".to_string()));
        assert_eq!(modified.new_value, Some("200".to_string()));

        let deleted = report.changes.iter().find(|c| c.change_type == StorageChangeType::Deleted).unwrap();
        assert_eq!(deleted.key, "balance:user2");
        assert_eq!(deleted.old_value, Some("500".to_string()));
    }
}
