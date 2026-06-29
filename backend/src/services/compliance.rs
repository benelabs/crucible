use crate::error::AppError;
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct ComplianceIssue {
    pub severity: String, // "info" | "warning" | "error"
    pub message: String,
    pub rule_id: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct ComplianceReport {
    pub is_compliant: bool,
    pub issues: Vec<ComplianceIssue>,
}

#[derive(Clone)]
pub struct ComplianceService {
    #[allow(dead_code)]
    db: sqlx::PgPool,
}

impl ComplianceService {
    pub fn new(db: sqlx::PgPool) -> Self {
        Self { db }
    }

    pub async fn check_compliance(&self, source_code: &str) -> Result<ComplianceReport, AppError> {
        let mut issues = Vec::new();

        // Rule 1: No unsafe code
        if source_code.contains("unsafe ")
            || source_code.contains("unsafe{")
            || source_code.contains("unsafe {")
        {
            issues.push(ComplianceIssue {
                severity: "error".to_string(),
                message: "Unsafe code block or modifier detected. Soroban smart contracts must run inside a safe sandbox.".to_string(),
                rule_id: "NO_UNSAFE".to_string(),
            });
        }

        // Rule 2: License headers
        if !source_code.contains("SPDX-License-Identifier") {
            issues.push(ComplianceIssue {
                severity: "warning".to_string(),
                message:
                    "Missing SPDX-License-Identifier licensing header in smart contract source."
                        .to_string(),
                rule_id: "MISSING_LICENSE".to_string(),
            });
        }

        // Rule 3: File or network operations (which will fail in Wasm)
        if source_code.contains("std::fs")
            || source_code.contains("std::net")
            || source_code.contains("std::thread")
        {
            issues.push(ComplianceIssue {
                severity: "error".to_string(),
                message: "Standard library filesystem/networking/threading usage detected, which is incompatible with the Wasm runtime.".to_string(),
                rule_id: "FORBIDDEN_STD_LIBS".to_string(),
            });
        }

        // Rule 4: SDK version or module check
        if !source_code.contains("soroban_sdk") && !source_code.contains("soroban-sdk") {
            issues.push(ComplianceIssue {
                severity: "warning".to_string(),
                message: "No references to soroban_sdk or soroban-sdk crate detected. Confirm SDK requirements.".to_string(),
                rule_id: "MISSING_SDK_REFERENCE".to_string(),
            });
        }

        let is_compliant = !issues.iter().any(|i| i.severity == "error");

        Ok(ComplianceReport {
            is_compliant,
            issues,
        })
    }
}
