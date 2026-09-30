//! Helpers for measuring and reporting contract execution costs.
//! SDK 26: soroban_env_host is not a public dependency; FeeEstimate is defined locally.

/// Fee breakdown returned by the Soroban host (SDK 26 compatible).
/// Mirrors the fields previously exposed by `soroban_env_host::FeeEstimate`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FeeEstimate {
    pub total: i64,
    pub instructions: i64,
    pub disk_read_entries: i64,
    pub write_entries: i64,
    pub disk_read_bytes: i64,
    pub write_bytes: i64,
    pub contract_events: i64,
    pub persistent_entry_rent: i64,
    pub temporary_entry_rent: i64,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CostReport {
    instructions: u64,
    memory: u64,
    fee_stroops: Option<i128>,
    #[cfg(feature = "std")]
    report_cache: std::sync::OnceLock<String>,
}

use std::fmt;

impl std::fmt::Display for CostReport {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let instructions_str = format_with_commas(self.instructions);
        let memory_str = format_with_commas(self.memory);
        let fee_str = format!("{} str", self.fee_stroops());
        let source = if self.uses_sdk_fee_estimate() {
            "SDK"
        } else {
            "heuristic"
        };
        writeln!(f, "+---------------------+-----------+")?;
        writeln!(f, "| Metric              | Value     |")?;
        writeln!(f, "+---------------------+-----------+")?;
        writeln!(f, "| Instructions        | {:>9} |", instructions_str)?;
        writeln!(f, "| Memory (bytes)      | {:>9} |", memory_str)?;
        writeln!(f, "| Estimated fee       | {:>9} |", fee_str)?;
        writeln!(f, "| Fee source          | {:>9} |", source)?;
        write!(f, "+---------------------+-----------+")
    }
}

impl CostReport {
    pub fn new(instructions: u64, memory: u64) -> Self {
        Self { instructions, memory }
    }
    pub fn instructions(&self) -> u64 { self.instructions }
    pub fn memory_bytes(&self) -> u64 { self.memory }
    pub fn fee_stroops(&self) -> i64 { (self.instructions / 100) as i64 }
    pub fn report(&self) -> String {
        format!("Instructions: {}\nMemory: {} bytes\nFee: {} stroops",
            format_with_commas(self.instructions),
            format_with_commas(self.memory),
            self.fee_stroops())
    }
}

fn format_with_commas(n: u64) -> String {
    let s = n.to_string();
    let chars: Vec<char> = s.chars().collect();
    let len = chars.len();
    let mut result = String::new();
    for (i, &c) in chars.iter().enumerate() {
        result.push(c);
        if (len - i - 1) > 0 && (len - i - 1) % 3 == 0 { result.push(','); }
    }
    result
}

#[cfg(feature = "snapshots")]
use serde::{Deserialize, Serialize};

#[cfg(feature = "snapshots")]
#[derive(Serialize, Deserialize)]
struct CostSnapshot {
    name: String,
    instructions: u64,
    memory_bytes: u64,
    fee_stroops: i64,
}

#[cfg(feature = "snapshots")]
impl CostReport {
    pub fn assert_snapshot(&self, name: &str) {
        self.assert_snapshot_with_tolerance(name, 0.05);
    }
    pub fn assert_snapshot_with_tolerance(&self, name: &str, tolerance: f64) {
        use std::fs;
        use std::path::PathBuf;
        let manifest_dir = std::env::var("CARGO_MANIFEST_DIR").unwrap_or_else(|_| ".".to_string());
        let snap_dir = PathBuf::from(&manifest_dir).join("test_snapshots").join("cost");
        let snap_path = snap_dir.join(format!("{}.json", name));
        let update = std::env::var("CRUCIBLE_UPDATE_SNAPSHOTS").map(|v| v == "1").unwrap_or(false);
        if !snap_path.exists() || update {
            fs::create_dir_all(&snap_dir).unwrap();
            let snap = CostSnapshot { name: name.to_string(), instructions: self.instructions, memory_bytes: self.memory, fee_stroops: self.fee_stroops() };
            let json = serde_json::to_string_pretty(&snap).unwrap();
            fs::write(&snap_path, json).unwrap();
            return;
        }
        let contents = fs::read_to_string(&snap_path).unwrap();
        let saved: CostSnapshot = serde_json::from_str(&contents).unwrap();
        check_tolerance("instructions", saved.instructions, self.instructions, tolerance, name);
        check_tolerance("memory_bytes", saved.memory_bytes, self.memory, tolerance, name);
    }
}

#[cfg(feature = "snapshots")]
fn check_tolerance(metric: &str, saved: u64, current: u64, tolerance: f64, name: &str) {
    if saved == 0 { return; }
    let ratio = current as f64 / saved as f64;
    if ratio > 1.0 + tolerance {
        panic!("cost regression in '{}': {} {} -> {} ({:.1}% > {:.1}%)", name, metric, saved, current, (ratio-1.0)*100.0, tolerance*100.0);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn test_new() {
        let r = CostReport::new(1000, 500);
        assert_eq!(r.instructions(), 1000);
    }
    #[test]
    fn test_commas() {
        assert_eq!(format_with_commas(1234), "1,234");
    }
}
