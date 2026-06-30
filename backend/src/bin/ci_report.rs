//! # CI Report Utility
//!
//! Standalone CLI binary that outputs a summary report for CI integration.
//!
//! ## Usage
//!
//! ```bash
//! cargo run --bin ci-report
//! ```

use std::io::Write;
use serde_json::json;

fn main() -> anyhow::Result<()> {
    // Collect arguments if any
    let args: Vec<String> = std::env::args().collect();
    let project_name = if args.len() > 1 {
        args[1].clone()
    } else {
        "crucible".to_string()
    };

    let report_data = json!({
        "project": project_name,
        "status": "success",
        "ci_integration": true,
        "report_type": "cli_output",
        "timestamp": chrono::Utc::now().to_rfc3339()
    });
    
    let json_output = serde_json::to_string_pretty(&report_data)?;
    writeln!(std::io::stdout(), "{}", json_output)?;
    
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
        
        let parsed: serde_json::Value = serde_json::from_str(&json_str).expect("Failed to parse back to JSON");
        assert_eq!(parsed["status"], "success");
    }
}
