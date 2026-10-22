//! # CI Report Utility
//!
//! Standalone CLI binary that outputs a summary report for CI integration.
//!
//! ## Usage
//!
//! ```bash
//! cargo run --bin ci-report
//! ```

use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::io::Write;
use std::path::Path;

#[derive(Debug, Serialize, Deserialize)]
pub struct CiReport {
    pub project: String,
    pub status: String,
    pub ci_integration: bool,
    pub report_type: String,
    pub timestamp: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub regressions_detected: Option<bool>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub failures_detected: Option<bool>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub details: Option<Value>,
}

/// Evaluates whether a report JSON payload or status contains benchmark regressions or failures.
pub fn evaluate_report_status(report_val: &Value) -> bool {
    // 1. Check top-level status string
    if let Some(status_str) = report_val.get("status").and_then(|s| s.as_str()) {
        let s = status_str.to_lowercase();
        if s == "failed" || s == "failure" || s == "error" || s == "regression" {
            return false;
        }
    }

    // 2. Check if benchmark regressions array is present and non-empty
    if let Some(regressions) = report_val.get("regressions").and_then(|r| r.as_array()) {
        if !regressions.is_empty() {
            return false;
        }
    }

    // 3. Check regressions_detected or failures_detected flags
    if let Some(reg_detected) = report_val.get("regressions_detected").and_then(|b| b.as_bool()) {
        if reg_detected {
            return false;
        }
    }
    if let Some(fail_detected) = report_val.get("failures_detected").and_then(|b| b.as_bool()) {
        if fail_detected {
            return false;
        }
    }

    // 4. Check nested overall / operations / test failures
    if let Some(failed_tests) = report_val.get("failed_tests").and_then(|n| n.as_u64()) {
        if failed_tests > 0 {
            return false;
        }
    }

    true
}

fn main() -> anyhow::Result<()> {
    let args: Vec<String> = std::env::args().collect();
    let mut project_name = "crucible".to_string();
    let mut report_payload: Option<Value> = None;

    if args.len() > 1 {
        let first_arg = &args[1];
        // Check if argument is a JSON string or file path
        if first_arg.trim_start().starts_with('{') {
            if let Ok(v) = serde_json::from_str::<Value>(first_arg) {
                if let Some(p) = v.get("project").and_then(|p| p.as_str()) {
                    project_name = p.to_string();
                }
                report_payload = Some(v);
            }
        } else if Path::new(first_arg).exists() {
            if let Ok(content) = std::fs::read_to_string(first_arg) {
                if let Ok(v) = serde_json::from_str::<Value>(&content) {
                    if let Some(p) = v.get("project").and_then(|p| p.as_str()) {
                        project_name = p.to_string();
                    }
                    report_payload = Some(v);
                }
            }
        } else {
            project_name = first_arg.clone();
            // If second argument is provided (e.g. status or JSON file)
            if args.len() > 2 {
                let second_arg = &args[2];
                if second_arg.trim_start().starts_with('{') {
                    if let Ok(v) = serde_json::from_str::<Value>(second_arg) {
                        report_payload = Some(v);
                    }
                } else if Path::new(second_arg).exists() {
                    if let Ok(content) = std::fs::read_to_string(second_arg) {
                        if let Ok(v) = serde_json::from_str::<Value>(&content) {
                            report_payload = Some(v);
                        }
                    }
                } else if second_arg == "failed" || second_arg == "regression" {
                    report_payload = Some(serde_json::json!({
                        "status": second_arg
                    }));
                }
            }
        }
    }

    // Determine health and build final output report
    let passed = if let Some(ref payload) = report_payload {
        evaluate_report_status(payload)
    } else {
        true
    };

    let status = if passed { "success" } else { "failed" };

    let mut report = serde_json::json!({
        "project": project_name,
        "status": status,
        "ci_integration": true,
        "report_type": "cli_output",
        "timestamp": chrono::Utc::now().to_rfc3339(),
        "regressions_detected": !passed,
        "failures_detected": !passed
    });

    if let Some(payload) = report_payload {
        if let Some(obj) = report.as_object_mut() {
            obj.insert("details".to_string(), payload);
        }
    }

    let json_output = serde_json::to_string_pretty(&report)?;
    writeln!(std::io::stdout(), "{}", json_output)?;

    if !passed {
        std::process::exit(1);
    }

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_ci_report_json_format() {
        let report_data = serde_json::json!({
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
    }

    #[test]
    fn test_ci_report_evaluation_success() {
        let payload = serde_json::json!({
            "status": "success",
            "regressions": [],
            "failed_tests": 0
        });
        assert!(evaluate_report_status(&payload));
    }

    #[test]
    fn test_ci_report_evaluation_benchmark_regressions() {
        let payload = serde_json::json!({
            "status": "warning",
            "regressions": [
                {
                    "metric": "p95_duration_us",
                    "baseline": 100.0,
                    "current": 150.0
                }
            ]
        });
        assert!(!evaluate_report_status(&payload), "Regressions must be detected and return false");
    }

    #[test]
    fn test_ci_report_evaluation_failed_status() {
        let payload = serde_json::json!({
            "status": "failed",
            "failed_tests": 3
        });
        assert!(!evaluate_report_status(&payload), "Failed status must return false");
    }
}
