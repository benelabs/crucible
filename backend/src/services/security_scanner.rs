#![allow(dead_code)]

use chrono::Utc;
use serde::{Deserialize, Serialize};
use sqlx::PgPool;
use uuid::Uuid;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SecurityFinding {
    pub id: String,
    pub severity: String,
    pub title: String,
    pub description: String,
    pub line_number: Option<u32>,
    pub recommendation: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SecurityReport {
    pub contract_name: String,
    pub findings: Vec<SecurityFinding>,
    pub risk_score: f64,
    pub scanned_at: chrono::DateTime<Utc>,
    pub passed: bool,
}

pub struct SecurityScanner {
    #[allow(dead_code)]
    db: Option<PgPool>,
}

impl Default for SecurityScanner {
    fn default() -> Self {
        Self::without_db()
    }
}

impl SecurityScanner {
    pub fn new(db: PgPool) -> Self {
        Self { db: Some(db) }
    }

    pub fn without_db() -> Self {
        Self { db: None }
    }

    /// Explicitly validates all WebAssembly section header boundaries, offsets, and payload sizes.
    pub fn validate_section_header_offsets(wasm_bytes: &[u8]) -> Result<(), String> {
        if wasm_bytes.len() < 8 || &wasm_bytes[0..4] != b"\0asm" {
            return Err("Invalid WebAssembly magic header".to_string());
        }

        let parser = wasmparser::Parser::new(0);
        for payload in parser.parse_all(wasm_bytes) {
            payload
                .map_err(|e| format!("Malformed section header offset or payload length: {}", e))?;
        }

        Ok(())
    }

    /// Disassembles and scans compiled WebAssembly contract bytecode for unsafe instructions
    /// and security vulnerabilities, validating section header boundaries and payload lengths
    /// via wasmparser to safely reject malformed or truncated binaries without panicking.
    pub fn scan_wasm_bytecode(
        &self,
        contract_name: &str,
        wasm_bytes: &[u8],
    ) -> Result<SecurityReport, String> {
        if wasm_bytes.is_empty() {
            return Err("WASM binary is empty".to_string());
        }

        if wasm_bytes.len() < 8 || &wasm_bytes[0..4] != b"\0asm" {
            return Err("Invalid WebAssembly magic header".to_string());
        }

        let mut findings = Vec::new();

        let parser = wasmparser::Parser::new(0);
        for payload in parser.parse_all(wasm_bytes) {
            let payload = match payload {
                Ok(p) => p,
                Err(err) => {
                    findings.push(SecurityFinding {
                        id: Uuid::new_v4().to_string(),
                        severity: "critical".to_string(),
                        title: "Malformed WASM section header or truncated binary".to_string(),
                        description: format!(
                            "Corrupted section boundary or payload offset detected: {}",
                            err
                        ),
                        line_number: None,
                        recommendation: "Ensure WASM binary is compiled cleanly and not truncated."
                            .to_string(),
                    });
                    break;
                }
            };

            if let wasmparser::Payload::CodeSectionEntry(body) = payload {
                let mut reader = match body.get_operators_reader() {
                    Ok(r) => r,
                    Err(e) => {
                        findings.push(SecurityFinding {
                            id: Uuid::new_v4().to_string(),
                            severity: "high".to_string(),
                            title: "Invalid code section operators reader".to_string(),
                            description: format!("Failed to read code section operators: {}", e),
                            line_number: None,
                            recommendation: "Verify code section integrity.".to_string(),
                        });
                        continue;
                    }
                };

                while !reader.eof() {
                    match reader.read() {
                        Ok(op) => {
                            if let wasmparser::Operator::MemoryGrow { .. } = op {
                                findings.push(SecurityFinding {
                                    id: Uuid::new_v4().to_string(),
                                    severity: "medium".to_string(),
                                    title: "Dynamic memory growth instruction detected".to_string(),
                                    description: "Contract executes memory.grow opcode which can lead to unpredictable heap memory expansion.".to_string(),
                                    line_number: None,
                                    recommendation: "Avoid unbounded dynamic memory growth in Soroban sandbox.".to_string(),
                                });
                            }
                        }
                        Err(e) => {
                            findings.push(SecurityFinding {
                                id: Uuid::new_v4().to_string(),
                                severity: "critical".to_string(),
                                title: "Truncated instruction sequence in function body"
                                    .to_string(),
                                description: format!("Operator read failed at offset: {}", e),
                                line_number: None,
                                recommendation: "Reject corrupted WASM bytecode.".to_string(),
                            });
                            break;
                        }
                    }
                }
            }
        }

        let risk_score: f64 = findings
            .iter()
            .map(|f| match f.severity.as_str() {
                "critical" => 50.0,
                "high" => 15.0,
                "medium" => 5.0,
                "low" => 1.0,
                _ => 0.0,
            })
            .sum::<f64>()
            .min(100.0);

        let has_critical = findings.iter().any(|f| f.severity == "critical");

        Ok(SecurityReport {
            contract_name: contract_name.to_string(),
            findings,
            risk_score,
            scanned_at: Utc::now(),
            passed: !has_critical && risk_score < 50.0,
        })
    }

    pub async fn scan(
        &self,
        contract_name: &str,
        source_code: &str,
    ) -> Result<SecurityReport, sqlx::Error> {
        let mut findings = Vec::new();

        // Check for unsafe blocks
        if source_code.contains("unsafe {") || source_code.contains("unsafe{") {
            findings.push(SecurityFinding {
                id: Uuid::new_v4().to_string(),
                severity: "high".to_string(),
                title: "Unsafe block detected".to_string(),
                description: "The contract contains unsafe Rust code which bypasses memory safety guarantees.".to_string(),
                line_number: source_code.lines().enumerate().find(|(_, l)| l.contains("unsafe")).map(|(i, _)| i as u32 + 1),
                recommendation: "Remove unsafe blocks or document why they are necessary and safe.".to_string(),
            });
        }

        // Check for missing auth on sensitive operations
        let has_transfer = source_code.contains("transfer") || source_code.contains("mint");
        let has_auth =
            source_code.contains("require_auth") || source_code.contains("require_auth_for_args");
        if has_transfer && !has_auth {
            findings.push(SecurityFinding {
                id: Uuid::new_v4().to_string(),
                severity: "medium".to_string(),
                title: "Missing authorization check".to_string(),
                description:
                    "Sensitive operations (transfer/mint) found without require_auth checks."
                        .to_string(),
                line_number: None,
                recommendation:
                    "Add require_auth() or require_auth_for_args() before sensitive operations."
                        .to_string(),
            });
        }

        // Check for potential integer overflow (absence of checked arithmetic)
        let has_arithmetic = source_code.contains(" + ") || source_code.contains(" * ");
        let has_checked = source_code.contains("checked_add")
            || source_code.contains("checked_mul")
            || source_code.contains("saturating_");
        if has_arithmetic && !has_checked {
            findings.push(SecurityFinding {
                id: Uuid::new_v4().to_string(),
                severity: "medium".to_string(),
                title: "Potential integer overflow".to_string(),
                description: "Arithmetic operations found without checked or saturating variants."
                    .to_string(),
                line_number: None,
                recommendation:
                    "Use checked_add, checked_mul, or saturating_add to prevent overflow."
                        .to_string(),
            });
        }

        // Check for hardcoded addresses
        if source_code.contains("G") && source_code.contains("AAAA") {
            findings.push(SecurityFinding {
                id: Uuid::new_v4().to_string(),
                severity: "low".to_string(),
                title: "Hardcoded address detected".to_string(),
                description: "A Stellar address appears to be hardcoded in the contract source."
                    .to_string(),
                line_number: None,
                recommendation: "Store addresses in contract storage or pass them as parameters."
                    .to_string(),
            });
        }

        let risk_score: f64 = findings
            .iter()
            .map(|f| match f.severity.as_str() {
                "critical" => 25.0,
                "high" => 15.0,
                "medium" => 5.0,
                "low" => 1.0,
                _ => 0.0,
            })
            .sum::<f64>()
            .min(100.0);

        Ok(SecurityReport {
            contract_name: contract_name.to_string(),
            findings,
            risk_score,
            scanned_at: Utc::now(),
            passed: risk_score < 50.0,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn make_valid_wasm() -> Vec<u8> {
        vec![
            0x00, 0x61, 0x73, 0x6d, 0x01, 0x00, 0x00, 0x00, // \0asm v1
            0x01, 0x04, 0x01, 0x60, 0x00, 0x00, // Type sec
            0x03, 0x02, 0x01, 0x00, // Func sec
            0x0a, 0x04, 0x01, 0x02, 0x00, 0x0b, // Code sec
        ]
    }

    #[test]
    fn test_valid_wasm_scans_cleanly() {
        let scanner = SecurityScanner::without_db();
        let wasm = make_valid_wasm();
        let report = scanner.scan_wasm_bytecode("GoodContract", &wasm).unwrap();
        assert!(report.passed);
        assert_eq!(report.findings.len(), 0);
        assert!(SecurityScanner::validate_section_header_offsets(&wasm).is_ok());
    }

    #[test]
    fn test_truncated_section_header_does_not_panic() {
        let scanner = SecurityScanner::without_db();
        let mut truncated = make_valid_wasm();
        // Corrupt section payload length to claim 1000 bytes when only 4 exist
        truncated[15] = 0xFF; // Code section header payload len = huge
        truncated.truncate(18); // Truncate file

        // Must NOT panic!
        let report = scanner
            .scan_wasm_bytecode("TruncatedContract", &truncated)
            .unwrap();
        assert!(!report.passed);
        assert!(report.findings.iter().any(|f| f.severity == "critical"));

        assert!(SecurityScanner::validate_section_header_offsets(&truncated).is_err());
    }

    #[test]
    fn test_invalid_wasm_magic_rejected() {
        let scanner = SecurityScanner::without_db();
        let invalid = vec![1, 2, 3, 4, 5, 6, 7, 8];
        assert!(scanner.scan_wasm_bytecode("Bad", &invalid).is_err());
        assert!(SecurityScanner::validate_section_header_offsets(&invalid).is_err());
    }
}
