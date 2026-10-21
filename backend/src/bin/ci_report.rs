//! # CI Report Utility
//!
//! Standalone CLI binary that outputs a summary report for CI integration.
//!
//! ## Usage
//!
//! ```bash
//! cargo run --bin ci-report
//! ```

use serde_json::{json, Value};
use std::io::Write;

/// Inspects the report payload for benchmark regressions, test failures, or error statuses.
/// Returns `true` if any regression or failure is detected.
pub fn has_regressions_or_failures(report: &Value) -> bool {
    // Check top-level status
    if let Some(status) = report.get("status").and_then(|s| s.as_str()) {
        let status_lower = status.to_lowercase();
        if status_lower == "failure"
            || status_lower == "failed"
            || status_lower == "regression"
            || status_lower == "error"
        {
            return true;
        }
    }

    // Check benchmark regressions
    if let Some(regressions) = report.get("benchmark_regressions") {
        if regressions.as_bool().unwrap_or(false) {
            return true;
        }
        if let Some(arr) = regressions.as_array() {
            if !arr.is_empty() {
                return true;
            }
        }
        if let Some(n) = regressions.as_u64() {
            if n > 0 {
                return true;
            }
        }
    }

    if let Some(reg) = report.get("has_regressions").and_then(|v| v.as_bool()) {
        if reg {
            return true;
        }
    }

    // Check test failures
    if let Some(failures) = report.get("test_failures") {
        if failures.as_bool().unwrap_or(false) {
            return true;
        }
        if let Some(arr) = failures.as_array() {
            if !arr.is_empty() {
                return true;
            }
        }
        if let Some(n) = failures.as_u64() {
            if n > 0 {
                return true;
            }
        }
    }

    if let Some(failures) = report.get("failures") {
        if failures.as_bool().unwrap_or(false) {
            return true;
        }
        if let Some(arr) = failures.as_array() {
            if !arr.is_empty() {
                return true;
            }
        }
        if let Some(n) = failures.as_u64() {
            if n > 0 {
                return true;
            }
        }
    }

    false
}

fn main() -> anyhow::Result<()> {
    // Collect arguments if any
    let args: Vec<String> = std::env::args().collect();
    let project_name = if args.len() > 1 && !args[1].starts_with('-') {
        args[1].clone()
    } else {
        "crucible".to_string()
    };

    let report_data = if let Some(file_idx) = args.iter().position(|a| a == "--report-file" || a == "--file") {
        if let Some(path) = args.get(file_idx + 1) {
            let content = std::fs::read_to_string(path)?;
            serde_json::from_str::<Value>(&content)?
        } else {
            anyhow::bail!("Missing path for report file flag");
        }
    } else {
        let has_regression = args.iter().any(|a| a == "--regression" || a == "--benchmark-regression")
            || std::env::var("BENCHMARK_REGRESSION").map(|v| v == "1" || v.eq_ignore_ascii_case("true")).unwrap_or(false);
        let has_failure = args.iter().any(|a| a == "--failure" || a == "--test-failure")
            || std::env::var("CI_FAILURES").map(|v| v == "1" || v.eq_ignore_ascii_case("true")).unwrap_or(false);
        let status = if has_regression || has_failure { "failure" } else { "success" };

        json!({
            "project": project_name,
            "status": status,
            "ci_integration": true,
            "report_type": "cli_output",
            "benchmark_regressions": has_regression,
            "test_failures": has_failure,
            "timestamp": chrono::Utc::now().to_rfc3339()
        })
    };

    let json_output = serde_json::to_string_pretty(&report_data)?;
    writeln!(std::io::stdout(), "{}", json_output)?;

    if has_regressions_or_failures(&report_data) {
        eprintln!("CI Report detected benchmark regressions or test failures. Exiting with code 1.");
        std::process::exit(1);
    }

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_ci_report_json_format() {
        let report_data = json!({
            "project": "test-project",
            "status": "success",
            "ci_integration": true,
            "report_type": "cli_output"
        });

        let json_str = serde_json::to_string(&report_data).expect("Failed to serialize");
        assert!(json_str.contains("test-project"));
        assert!(json_str.contains("success"));
        assert!(json_str.contains("cli_output"));

        let parsed: serde_json::Value =
            serde_json::from_str(&json_str).expect("Failed to parse back to JSON");
        assert_eq!(parsed["status"], "success");
        assert!(!has_regressions_or_failures(&parsed));
    }

    #[test]
    fn test_detects_benchmark_regressions_flag() {
        let report_data = json!({
            "project": "crucible",
            "status": "success",
            "benchmark_regressions": true
        });
        assert!(has_regressions_or_failures(&report_data));

        let report_array = json!({
            "project": "crucible",
            "status": "success",
            "benchmark_regressions": ["sort_benchmark regressed by 15%"]
        });
        assert!(has_regressions_or_failures(&report_array));
    }

    #[test]
    fn test_detects_test_failures() {
        let report_data = json!({
            "project": "crucible",
            "status": "success",
            "test_failures": 2
        });
        assert!(has_regressions_or_failures(&report_data));
    }

    #[test]
    fn test_detects_failure_status() {
        let report_data = json!({
            "project": "crucible",
            "status": "failure"
        });
        assert!(has_regressions_or_failures(&report_data));

        let report_error = json!({
            "project": "crucible",
            "status": "error"
        });
        assert!(has_regressions_or_failures(&report_error));
    }

    #[test]
    fn test_clean_report_passes() {
        let report_data = json!({
            "project": "crucible",
            "status": "success",
            "benchmark_regressions": false,
            "test_failures": 0
        });
        assert!(!has_regressions_or_failures(&report_data));
    }
}
