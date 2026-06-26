//! Helpers for measuring and reporting contract execution costs.

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
