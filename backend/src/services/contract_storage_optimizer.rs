use base64::{engine::general_purpose::STANDARD, Engine as _};
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use sqlx::PgPool;
use std::collections::HashMap;
use stellar_xdr::curr::{Limited, Limits, ReadXdr, ScVal, WriteXdr};

use crate::error::AppError;

/// Dynamic active network ledger configuration parameters for storage rent calculation (#1214).
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct DynamicStorageRentConfig {
    /// Active ledger sequence from getLatestLedger.
    pub sequence: u64,
    /// Protocol version.
    pub protocol_version: u32,
    /// Fee per write entry in stroops / gas.
    pub fee_write_1kb: f64,
    /// Rent fee per persistent entry per ledger cycle.
    pub rent_fee_per_entry: f64,
    /// Rent fee per byte per ledger cycle.
    pub rent_fee_per_byte: f64,
    /// Minimum persistent rent TTL in ledgers.
    pub min_persistent_rent_ledgers: u32,
    /// Dynamic rent scaling factor (percentage scale).
    pub rent_rate_multiplier: f64,
}

impl Default for DynamicStorageRentConfig {
    fn default() -> Self {
        Self {
            sequence: 1_000_000,
            protocol_version: 21,
            fee_write_1kb: 10_000.0,
            rent_fee_per_entry: 0.0001,
            rent_fee_per_byte: 0.000005,
            min_persistent_rent_ledgers: 100_000,
            rent_rate_multiplier: 30.0,
        }
    }
}

impl DynamicStorageRentConfig {
    /// Creates a network-specific default configuration for known networks.
    pub fn for_network(network: &str) -> Self {
        match network.to_ascii_lowercase().as_str() {
            "mainnet" => Self {
                sequence: 55_000_000,
                protocol_version: 21,
                fee_write_1kb: 12_500.0,
                rent_fee_per_entry: 0.00015,
                rent_fee_per_byte: 0.000006,
                min_persistent_rent_ledgers: 120_000,
                rent_rate_multiplier: 35.0,
            },
            "testnet" => Self {
                sequence: 2_500_000,
                protocol_version: 21,
                fee_write_1kb: 10_000.0,
                rent_fee_per_entry: 0.0001,
                rent_fee_per_byte: 0.000005,
                min_persistent_rent_ledgers: 100_000,
                rent_rate_multiplier: 30.0,
            },
            _ => Self::default(),
        }
    }

    /// Computes estimated storage rent savings percent based on high-impact persistent entries.
    pub fn compute_savings_percent(&self, persistent_entries: f64, high_impact: f64) -> f64 {
        if persistent_entries <= 0.0 {
            0.0
        } else {
            let ratio = high_impact / persistent_entries;
            (ratio * self.rent_rate_multiplier).min(self.rent_rate_multiplier)
        }
    }

    /// Parses getLatestLedger JSON-RPC response body into dynamic ledger rent parameters.
    pub fn parse_rpc_latest_ledger(json_val: &serde_json::Value) -> Result<Self, AppError> {
        let result = json_val.get("result").unwrap_or(json_val);
        let sequence = result
            .get("sequence")
            .and_then(|s| s.as_u64())
            .unwrap_or(1_000_000);
        let protocol_version = result
            .get("protocolVersion")
            .and_then(|p| p.as_u64())
            .map(|p| p as u32)
            .unwrap_or(21);

        let multiplier = if sequence > 10_000_000 {
            35.0
        } else if sequence > 1_000_000 {
            30.0
        } else {
            25.0
        };

        Ok(Self {
            sequence,
            protocol_version,
            fee_write_1kb: 10_000.0,
            rent_fee_per_entry: 0.0001,
            rent_fee_per_byte: 0.000005,
            min_persistent_rent_ledgers: 100_000,
            rent_rate_multiplier: multiplier,
        })
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, Default)]
#[serde(rename_all = "camelCase")]
pub struct StorageOptimizationInput {
    pub contract_id: String,
    pub source_code: String,
    pub target_network: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub rpc_url: Option<String>,
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
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub active_ledger_sequence: Option<u64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub rent_config: Option<DynamicStorageRentConfig>,
}

// ---------------------------------------------------------------------------
// Storage Diff Engine Types (#860) & Canonical XDR Keying (#1215)
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
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub canonical_key: Option<String>,
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

/// Serializes a Soroban DataKey (`ScVal`) to a deterministic, canonical base64-encoded XDR string.
///
/// Prevents collision and truncation bugs that occur when complex enum/tuple DataKeys
/// are converted to plain string keys using Debug or Display formatting.
pub fn serialize_datakey_to_canonical_xdr(key: &ScVal) -> Result<String, AppError> {
    let bytes = key
        .to_xdr(Limits::none())
        .map_err(|e| AppError::InternalError(format!("failed to serialize DataKey to XDR: {e}")))?;
    Ok(STANDARD.encode(bytes))
}

/// Deserializes a canonical base64-encoded XDR string back into a Soroban `ScVal` DataKey.
pub fn deserialize_datakey_from_canonical_xdr(xdr_base64: &str) -> Result<ScVal, AppError> {
    let bytes = STANDARD
        .decode(xdr_base64.trim())
        .map_err(|e| AppError::BadRequest(format!("invalid base64 DataKey: {e}")))?;
    let mut limited = Limited::new(std::io::Cursor::new(bytes), Limits::none());
    ScVal::read_xdr_to_end(&mut limited)
        .map_err(|e| AppError::BadRequest(format!("invalid XDR DataKey: {e}")))
}

#[derive(Clone)]
pub struct ContractStorageOptimizer {
    db: PgPool,
}

impl ContractStorageOptimizer {
    pub fn new(db: PgPool) -> Self {
        Self { db }
    }

    /// Canonicalizes a storage key string. If the key is a valid XDR-encoded base64 representation,
    /// it ensures canonical encoding; otherwise, the original key is preserved.
    pub fn canonicalize_storage_key(raw_key: &str) -> String {
        if let Ok(scval) = deserialize_datakey_from_canonical_xdr(raw_key) {
            if let Ok(canonical) = serialize_datakey_to_canonical_xdr(&scval) {
                return canonical;
            }
        }
        raw_key.to_string()
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
            let canonical_key = Self::canonicalize_storage_key(key);
            match after_state.get(key) {
                Some(new_val) => {
                    if old_val != new_val {
                        modified_count += 1;
                        changes.push(StorageDiffEntry {
                            key: key.clone(),
                            canonical_key: Some(canonical_key),
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
                        canonical_key: Some(canonical_key),
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
                let canonical_key = Self::canonicalize_storage_key(key);
                added_count += 1;
                changes.push(StorageDiffEntry {
                    key: key.clone(),
                    canonical_key: Some(canonical_key),
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

    /// Fetches active network ledger configuration parameters from a Soroban JSON-RPC endpoint.
    pub async fn fetch_network_ledger_config(
        rpc_url: &str,
    ) -> Result<DynamicStorageRentConfig, AppError> {
        let client = reqwest::Client::builder()
            .timeout(std::time::Duration::from_secs(5))
            .build()
            .map_err(|e| AppError::InternalError(format!("failed to build HTTP client: {e}")))?;

        let body = serde_json::json!({
            "jsonrpc": "2.0",
            "id": 1,
            "method": "getLatestLedger",
            "params": {}
        });

        let resp = client
            .post(rpc_url)
            .json(&body)
            .send()
            .await
            .map_err(|e| AppError::InternalError(format!("failed to call Soroban RPC: {e}")))?;

        let val: serde_json::Value = resp
            .json()
            .await
            .map_err(|e| AppError::InternalError(format!("failed to parse Soroban RPC JSON: {e}")))?;

        DynamicStorageRentConfig::parse_rpc_latest_ledger(&val)
    }

    /// Compare two ledger sequence storage maps indexed directly by Soroban `ScVal` DataKeys,
    /// serializing each DataKey to canonical base64-encoded XDR to prevent enum/variant key collisions.
    pub fn compare_datakey_storage_diff(
        contract_id: &str,
        from_sequence: u64,
        to_sequence: u64,
        before_state: &HashMap<ScVal, String>,
        after_state: &HashMap<ScVal, String>,
    ) -> Result<StorageDiffReport, AppError> {
        let mut before_canonical = HashMap::with_capacity(before_state.len());
        for (k, v) in before_state {
            let canonical_xdr = serialize_datakey_to_canonical_xdr(k)?;
            before_canonical.insert(canonical_xdr, v.clone());
        }

        let mut after_canonical = HashMap::with_capacity(after_state.len());
        for (k, v) in after_state {
            let canonical_xdr = serialize_datakey_to_canonical_xdr(k)?;
            after_canonical.insert(canonical_xdr, v.clone());
        }

        Ok(Self::compare_storage_diff(
            contract_id,
            from_sequence,
            to_sequence,
            &before_canonical,
            &after_canonical,
        ))
    }

    pub async fn optimize_with_config(
        &self,
        input: StorageOptimizationInput,
        rent_config: DynamicStorageRentConfig,
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

        let estimated_rent_savings_percent =
            rent_config.compute_savings_percent(persistent_entries, high_impact);

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

        let target_network = input
            .target_network
            .unwrap_or_else(|| "testnet".to_string());

        let report = StorageOptimizationReport {
            contract_id: input.contract_id,
            target_network,
            storage_entries_estimate,
            estimated_rent_savings_percent,
            ttl_strategy,
            recommendations,
            generated_at: Utc::now(),
            active_ledger_sequence: Some(rent_config.sequence),
            rent_config: Some(rent_config),
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
        .bind(serde_json::to_value(&report.recommendations).map_err(|e| AppError::InternalError(e.to_string()))?)
        .bind(report.generated_at)
        .execute(&self.db)
        .await;

        Ok(report)
    }

    pub async fn optimize(
        &self,
        input: StorageOptimizationInput,
    ) -> Result<StorageOptimizationReport, AppError> {
        let target_network = input.target_network.as_deref().unwrap_or("testnet");

        let rent_config = if let Some(ref url) = input.rpc_url {
            Self::fetch_network_ledger_config(url)
                .await
                .unwrap_or_else(|_| DynamicStorageRentConfig::for_network(target_network))
        } else {
            DynamicStorageRentConfig::for_network(target_network)
        };

        self.optimize_with_config(input, rent_config).await
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
                rpc_url: None,
            })
            .await
            .unwrap();

        assert_eq!(report.storage_entries_estimate, 1);
        assert_eq!(report.recommendations[0].recommended_storage, "temporary");
        assert!(report.active_ledger_sequence.is_some());
    }

    #[test]
    fn dynamic_storage_rent_config_scales_savings() {
        let testnet_config = DynamicStorageRentConfig::for_network("testnet");
        let mainnet_config = DynamicStorageRentConfig::for_network("mainnet");

        assert_eq!(testnet_config.rent_rate_multiplier, 30.0);
        assert_eq!(mainnet_config.rent_rate_multiplier, 35.0);

        // 1 high-impact out of 2 persistent entries = 50%
        let testnet_savings = testnet_config.compute_savings_percent(2.0, 1.0);
        assert_eq!(testnet_savings, 15.0);

        let mainnet_savings = mainnet_config.compute_savings_percent(2.0, 1.0);
        assert_eq!(mainnet_savings, 17.5);
    }

    #[test]
    fn parse_rpc_latest_ledger_response() {
        let rpc_response = serde_json::json!({
            "jsonrpc": "2.0",
            "id": 1,
            "result": {
                "id": "673f4e1f7b0e...",
                "protocolVersion": 21,
                "sequence": 15_420_000
            }
        });

        let config = DynamicStorageRentConfig::parse_rpc_latest_ledger(&rpc_response).unwrap();
        assert_eq!(config.sequence, 15_420_000);
        assert_eq!(config.protocol_version, 21);
        assert_eq!(config.rent_rate_multiplier, 35.0);
    }

    #[test]
    fn compares_storage_diff_accurately() {
        let mut before = HashMap::new();
        before.insert("balance:user1".to_string(), "100".to_string());
        before.insert("balance:user2".to_string(), "500".to_string());
        before.insert("admin".to_string(), "GB345".to_string());

        let mut after = HashMap::new();
        after.insert("balance:user1".to_string(), "200".to_string()); // Modified
        after.insert("admin".to_string(), "GB345".to_string()); // Unchanged
        after.insert("balance:user3".to_string(), "50".to_string()); // Added
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

        let added = report
            .changes
            .iter()
            .find(|c| c.change_type == StorageChangeType::Added)
            .unwrap();
        assert_eq!(added.key, "balance:user3");
        assert_eq!(added.new_value, Some("50".to_string()));

        let modified = report
            .changes
            .iter()
            .find(|c| c.change_type == StorageChangeType::Modified)
            .unwrap();
        assert_eq!(modified.key, "balance:user1");
        assert_eq!(modified.old_value, Some("100".to_string()));
        assert_eq!(modified.new_value, Some("200".to_string()));

        let deleted = report
            .changes
            .iter()
            .find(|c| c.change_type == StorageChangeType::Deleted)
            .unwrap();
        assert_eq!(deleted.key, "balance:user2");
        assert_eq!(deleted.old_value, Some("500".to_string()));
    }

    #[test]
    fn test_canonical_xdr_key_serialization_prevents_collision() {
        // Two distinct DataKeys: ScVal::U32(1) vs ScVal::I32(1)
        let key_u32 = ScVal::U32(1);
        let key_i32 = ScVal::I32(1);

        // In Debug format, both might print as simple numbers or collide in naive stringification
        let xdr_u32 = serialize_datakey_to_canonical_xdr(&key_u32).unwrap();
        let xdr_i32 = serialize_datakey_to_canonical_xdr(&key_i32).unwrap();

        assert_ne!(
            xdr_u32, xdr_i32,
            "Canonical XDR keys must be distinct for distinct ScVal variants"
        );

        // Verify round-trip deserialization
        let round_u32 = deserialize_datakey_from_canonical_xdr(&xdr_u32).unwrap();
        assert_eq!(round_u32, key_u32);

        let round_i32 = deserialize_datakey_from_canonical_xdr(&xdr_i32).unwrap();
        assert_eq!(round_i32, key_i32);
    }

    #[test]
    fn test_compare_datakey_storage_diff_with_xdr_keys() {
        let key1 = ScVal::U32(100);
        let key2 = ScVal::U32(200);
        let key3 = ScVal::U32(300);

        let mut before = HashMap::new();
        before.insert(key1.clone(), "val1".to_string());
        before.insert(key2.clone(), "old2".to_string());

        let mut after = HashMap::new();
        after.insert(key1.clone(), "val1".to_string()); // unchanged
        after.insert(key2.clone(), "new2".to_string()); // modified
        after.insert(key3.clone(), "val3".to_string()); // added

        let report = ContractStorageOptimizer::compare_datakey_storage_diff(
            "CDATAKEYCONTRACT",
            500,
            505,
            &before,
            &after,
        )
        .unwrap();

        assert_eq!(report.added_count, 1);
        assert_eq!(report.modified_count, 1);
        assert_eq!(report.deleted_count, 0);
        assert_eq!(report.changes.len(), 2);

        let mod_change = report
            .changes
            .iter()
            .find(|c| c.change_type == StorageChangeType::Modified)
            .unwrap();
        let expected_key2_xdr = serialize_datakey_to_canonical_xdr(&key2).unwrap();
        assert_eq!(mod_change.key, expected_key2_xdr);
        assert_eq!(mod_change.old_value, Some("old2".to_string()));
        assert_eq!(mod_change.new_value, Some("new2".to_string()));
    }
}
