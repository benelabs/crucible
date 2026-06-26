//! Helpers for measuring and reporting contract execution costs.

/// A report of the compute costs for a contract invocation.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CostReport {
    instructions: u64,
    memory: u64,
}

impl CostReport {
    /// Creates a new cost report.
    pub fn new(instructions: u64, memory: u64) -> Self {
        Self {
            instructions,
            memory,
        }
    }

    /// Returns the number of CPU instructions consumed.
    pub fn instructions(&self) -> u64 {
        self.instructions
    }

    /// Returns the peak memory usage in bytes.
    pub fn memory_bytes(&self) -> u64 {
        self.memory
    }

    /// Returns the estimated network fee in stroops.
    ///
    /// This is a simplified estimation based on instructions.
    /// Heuristic: 100 instructions = 1 stroop (calibrate as needed).
    pub fn fee_stroops(&self) -> i64 {
        (self.instructions / 100) as i64
    }

    /// Returns a human-readable formatted table report of the costs.
    ///
    /// The output is a formatted table with comma-separated numbers for readability.
    /// Example:
    /// ```text
    /// ┌─────────────────────┬───────────┐
    /// │ Metric              │ Value     │
    /// ├─────────────────────┼───────────┤
    /// │ Instructions        │ 1,234,567 │
    /// │ Memory (bytes)      │ 45,678    │
    /// │ Estimated fee       │ 123 str   │
    /// └─────────────────────┴───────────┘
    /// ```
    pub fn report(&self) -> String {
        let instructions_str = format_with_commas(self.instructions);
        let memory_str = format_with_commas(self.memory);
        let fee_str = format!("{} str", self.fee_stroops());

        // Create formatted table with box-drawing characters
        let mut output = String::new();
        output.push_str("┌─────────────────────┬───────────┐\n");
        output.push_str("│ Metric              │ Value     │\n");
        output.push_str("├─────────────────────┼───────────┤\n");
        output.push_str(&format!(
            "│ Instructions        │ {:>9} │\n",
            instructions_str
        ));
        output.push_str(&format!("│ Memory (bytes)      │ {:>9} │\n", memory_str));
        output.push_str(&format!("│ Estimated fee       │ {:>9} │\n", fee_str));
        output.push_str("└─────────────────────┴───────────┘");

        output
    }
}

/// Format a number with comma separators for readability.
fn format_with_commas(n: u64) -> String {
    let s = n.to_string();
    let mut result = String::new();
    let chars: Vec<char> = s.chars().collect();
    let len = chars.len();

    for (i, &c) in chars.iter().enumerate() {
        result.push(c);
        let remaining = len - i - 1;
        if remaining > 0 && remaining.is_multiple_of(3) {
            result.push(',');
        }
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
    /// Assert that this cost report matches a stored snapshot within a default 5% tolerance.
    pub fn assert_snapshot(&self, name: &str) {
        self.assert_snapshot_with_tolerance(name, 0.05);
    }

    /// Assert that this cost report matches a stored snapshot within a custom tolerance.
    pub fn assert_snapshot_with_tolerance(&self, name: &str, tolerance: f64) {
        use std::fs;
        use std::path::PathBuf;

        let manifest_dir = std::env::var("CARGO_MANIFEST_DIR")
            .unwrap_or_else(|_| ".".to_string());
        let snap_dir = PathBuf::from(&manifest_dir)
            .join("test_snapshots")
            .join("cost");
        let snap_path = snap_dir.join(format!("{}.json", name));

        let update = std::env::var("CRUCIBLE_UPDATE_SNAPSHOTS")
            .map(|v| v == "1" || v.eq_ignore_ascii_case("true"))
            .unwrap_or(false);

        if !snap_path.exists() || update {
            fs::create_dir_all(&snap_dir)
                .unwrap_or_else(|e| panic!("failed to create snapshot dir: {}", e));

            let snapshot = CostSnapshot {
                name: name.to_string(),
                instructions: self.instructions,
                memory_bytes: self.memory,
                fee_stroops: self.fee_stroops(),
            };

            let json = serde_json::to_string_pretty(&snapshot)
                .unwrap_or_else(|e| panic!("failed to serialize snapshot '{}': {}", name, e));

            fs::write(&snap_path, json)
                .unwrap_or_else(|e| panic!("failed to write snapshot '{}': {}", name, e));

            if update {
                eprintln!("[crucible] updated snapshot '{}'", name);
            } else {
                eprintln!("[crucible] wrote new snapshot '{}'", name);
            }
            return;
        }

        let contents = fs::read_to_string(&snap_path)
            .unwrap_or_else(|e| panic!("failed to read snapshot '{}': {}", name, e));

        let saved: CostSnapshot = serde_json::from_str(&contents)
            .unwrap_or_else(|e| panic!("failed to parse snapshot '{}': {}", name, e));

        check_within_tolerance("instructions", saved.instructions, self.instructions, tolerance, name);
        check_within_tolerance("memory_bytes", saved.memory_bytes, self.memory, tolerance, name);
    }
}

#[cfg(feature = "snapshots")]
fn check_within_tolerance(metric: &str, saved: u64, current: u64, tolerance: f64, name: &str) {
    if saved == 0 {
        return;
    }
    let ratio = current as f64 / saved as f64;
    if ratio > 1.0 + tolerance {
        panic!(
            "cost regression in snapshot '{}': {} increased from {} to {} ({:.1}% > {:.1}% tolerance)",
            name, metric, saved, current, (ratio - 1.0) * 100.0, tolerance * 100.0,
        );
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_cost_report_creation() {
        let report = CostReport::new(1_000_000, 50_000);
        assert_eq!(report.instructions(), 1_000_000);
        assert_eq!(report.memory_bytes(), 50_000);
    }

    #[test]
    fn test_fee_stroops_calculation() {
        let report = CostReport::new(10_000, 0);
        assert_eq!(report.fee_stroops(), 100); // 10_000 / 100 = 100
    }

    #[test]
    fn test_report_returns_non_empty_string() {
        let report = CostReport::new(1_234_567, 45_678);
        let report_str = report.report();
        assert!(!report_str.is_empty());
        // Check that expected labels are present
        assert!(report_str.contains("Instructions"));
        assert!(report_str.contains("Memory (bytes)"));
        assert!(report_str.contains("Estimated fee"));
    }

    #[test]
    fn test_format_with_commas() {
        assert_eq!(format_with_commas(0), "0");
        assert_eq!(format_with_commas(123), "123");
        assert_eq!(format_with_commas(1_234), "1,234");
        assert_eq!(format_with_commas(1_234_567), "1,234,567");
        assert_eq!(format_with_commas(1_000_000_000), "1,000,000,000");
    }

    #[test]
    fn test_report_formatting_contains_table_elements() {
        let report = CostReport::new(1_234_567, 45_678);
        let report_str = report.report();
        // Check for box-drawing characters
        assert!(report_str.contains("┌"));
        assert!(report_str.contains("┐"));
        assert!(report_str.contains("└"));
        assert!(report_str.contains("┘"));
        assert!(report_str.contains("├"));
        assert!(report_str.contains("┤"));
        assert!(report_str.contains("┼"));
    }
        #[test]
    fn test_snapshot_serialization_roundtrip() {
        #[cfg(feature = "snapshots")]
        {
            let snap = super::CostSnapshot {
                name: "test".to_string(),
                instructions: 1000,
                memory_bytes: 2000,
                fee_stroops: 10,
            };
            let json = serde_json::to_string(&snap).unwrap();
            let parsed: super::CostSnapshot = serde_json::from_str(&json).unwrap();
            assert_eq!(parsed.instructions, 1000);
            assert_eq!(parsed.memory_bytes, 2000);
        }
    }
}
