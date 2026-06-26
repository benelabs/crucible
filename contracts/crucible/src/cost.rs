//! Helpers for measuring and reporting contract execution costs.

/// A report of the compute costs for a contract invocation.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CostReport {
    instructions: u64,
    memory: u64,
}

impl CostReport {
    pub fn new(instructions: u64, memory: u64) -> Self {
        Self { instructions, memory }
    }

    pub fn instructions(&self) -> u64 { self.instructions }

    pub fn memory_bytes(&self) -> u64 { self.memory }

    pub fn fee_stroops(&self) -> i64 { (self.instructions / 100) as i64 }

    pub fn report(&self) -> String {
        let instructions_str = format_with_commas(self.instructions);
        let memory_str = format_with_commas(self.memory);
        let fee_str = format!("{} str", self.fee_stroops());
        let mut output = String::new();
        output.push_str("+---------------------+-----------+\n");
        output.push_str("| Metric              | Value     |\n");
        output.push_str("+---------------------+-----------+\n");
        output.push_str(&format!("| Instructions        | {:>9} |\n", instructions_str));
        output.push_str(&format!("| Memory (bytes)      | {:>9} |\n", memory_str));
        output.push_str(&format!("| Estimated fee       | {:>9} |\n", fee_str));
        output.push_str("+---------------------+-----------+");
        output
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
        let update = std::env::var("CRUCIBLE_UPDATE_SNAPSHOTS").map(|v| v == "1" || v.eq_ignore_ascii_case("true")).unwrap_or(false);
        if !snap_path.exists() || update {
            fs::create_dir_all(&snap_dir).unwrap_or_else(|e| panic!("failed to create snapshot dir: {}", e));
            let snapshot = CostSnapshot { name: name.to_string(), instructions: self.instructions, memory_bytes: self.memory, fee_stroops: self.fee_stroops() };
            let json = serde_json::to_string_pretty(&snapshot).unwrap_or_else(|e| panic!("failed to serialize: {}", e));
            fs::write(&snap_path, json).unwrap_or_else(|e| panic!("failed to write: {}", e));
            return;
        }
        let contents = fs::read_to_string(&snap_path).unwrap_or_else(|e| panic!("failed to read: {}", e));
        let saved: CostSnapshot = serde_json::from_str(&contents).unwrap_or_else(|e| panic!("failed to parse: {}", e));
        check_within_tolerance("instructions", saved.instructions, self.instructions, tolerance, name);
        check_within_tolerance("memory_bytes", saved.memory_bytes, self.memory, tolerance, name);
    }
}

#[cfg(feature = "snapshots")]
fn check_within_tolerance(metric: &str, saved: u64, current: u64, tolerance: f64, name: &str) {
    if saved == 0 { return; }
    let ratio = current as f64 / saved as f64;
    if ratio > 1.0 + tolerance {
        panic!("cost regression in snapshot '{}': {} increased from {} to {} ({:.1}% > {:.1}% tolerance)", name, metric, saved, current, (ratio - 1.0) * 100.0, tolerance * 100.0);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn test_cost_report_creation() {
        let r = CostReport::new(1_000_000, 50_000);
        assert_eq!(r.instructions(), 1_000_000);
        assert_eq!(r.memory_bytes(), 50_000);
    }
    #[test]
    fn test_format_with_commas() {
        assert_eq!(format_with_commas(0), "0");
        assert_eq!(format_with_commas(1234), "1,234");
        assert_eq!(format_with_commas(1_234_567), "1,234,567");
    }
    #[test]
    fn test_snapshot_serialization_roundtrip() {
        #[cfg(feature = "snapshots")]
        {
            let snap = super::CostSnapshot { name: "t".into(), instructions: 1000, memory_bytes: 2000, fee_stroops: 10 };
            let json = serde_json::to_string(&snap).unwrap();
            let parsed: super::CostSnapshot = serde_json::from_str(&json).unwrap();
            assert_eq!(parsed.instructions, 1000);
        }
    }
}
