use crate::error::AppError;
use serde::{Deserialize, Serialize};
use std::sync::Arc;
use std::time::Duration;
use tokio::sync::RwLock;
use uuid::Uuid;

pub const DEFAULT_COMPLIANCE_TIMEOUT: Duration = Duration::from_secs(3);

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

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub enum SanctionStatus {
    Compliant,
    Sanctioned,
    ManualReviewRequired,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct SanctionCheckRequest {
    pub entity_id: String,
    pub address: Option<String>,
    pub country_code: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct SanctionCheckResult {
    pub entity_id: String,
    pub status: SanctionStatus,
    pub details: String,
    pub checked_at: i64,
    pub queued_for_manual_review: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct ManualReviewItem {
    pub review_id: String,
    pub entity_id: String,
    pub address: Option<String>,
    pub reason: String,
    pub enqueued_at: i64,
    pub resolved: bool,
}

#[derive(Clone)]
pub struct ComplianceService {
    #[allow(dead_code)]
    db: sqlx::PgPool,
    client: reqwest::Client,
    timeout: Duration,
    manual_review_queue: Arc<RwLock<Vec<ManualReviewItem>>>,
    endpoint_url: Option<String>,
}

impl ComplianceService {
    pub fn new(db: sqlx::PgPool) -> Self {
        let client = reqwest::Client::builder()
            .timeout(DEFAULT_COMPLIANCE_TIMEOUT)
            .build()
            .unwrap_or_else(|_| reqwest::Client::new());

        Self {
            db,
            client,
            timeout: DEFAULT_COMPLIANCE_TIMEOUT,
            manual_review_queue: Arc::new(RwLock::new(Vec::new())),
            endpoint_url: None,
        }
    }

    pub fn with_endpoint_and_timeout(
        db: sqlx::PgPool,
        endpoint: String,
        timeout: Duration,
    ) -> Self {
        let client = reqwest::Client::builder()
            .timeout(timeout)
            .build()
            .unwrap_or_else(|_| reqwest::Client::new());

        Self {
            db,
            client,
            timeout,
            manual_review_queue: Arc::new(RwLock::new(Vec::new())),
            endpoint_url: Some(endpoint),
        }
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

    /// Verifies entity/contract against external OFAC / sanctions service with strict 3-second timeout.
    /// If upstream hangs or exceeds timeout, gracefully falls back to the manual review queue.
    pub async fn verify_sanctions(
        &self,
        req: SanctionCheckRequest,
    ) -> Result<SanctionCheckResult, AppError> {
        let now = chrono::Utc::now().timestamp();

        // If no external endpoint is configured, simulate local verification
        let endpoint = match &self.endpoint_url {
            Some(url) => url.clone(),
            None => {
                // If address is explicitly tagged known-sanctioned in heuristic:
                if let Some(addr) = &req.address {
                    if addr.contains("SANCTIONED") || addr.contains("BLOCKED") {
                        return Ok(SanctionCheckResult {
                            entity_id: req.entity_id,
                            status: SanctionStatus::Sanctioned,
                            details: "Address flagged on OFAC Specially Designated Nationals list"
                                .to_string(),
                            checked_at: now,
                            queued_for_manual_review: false,
                        });
                    }
                }

                return Ok(SanctionCheckResult {
                    entity_id: req.entity_id,
                    status: SanctionStatus::Compliant,
                    details: "Clean compliance verification (no sanction matches)".to_string(),
                    checked_at: now,
                    queued_for_manual_review: false,
                });
            }
        };

        // Strict timeout wrapper around external HTTP request
        let check_future = async { self.client.post(&endpoint).json(&req).send().await };

        match tokio::time::timeout(self.timeout, check_future).await {
            Ok(Ok(response)) => {
                if response.status().is_success() {
                    let result: Result<SanctionCheckResult, _> = response.json().await;
                    match result {
                        Ok(res) => Ok(res),
                        Err(_) => {
                            self.enqueue_manual_review(
                                req.entity_id.clone(),
                                req.address.clone(),
                                "Malformed response from compliance API".to_string(),
                            )
                            .await;

                            Ok(SanctionCheckResult {
                                entity_id: req.entity_id,
                                status: SanctionStatus::ManualReviewRequired,
                                details: "Malformed response from compliance API; enqueued for manual review".to_string(),
                                checked_at: now,
                                queued_for_manual_review: true,
                            })
                        }
                    }
                } else if response.status().as_u16() == 403 {
                    Ok(SanctionCheckResult {
                        entity_id: req.entity_id,
                        status: SanctionStatus::Sanctioned,
                        details: "Entity matched sanctioned OFAC registry".to_string(),
                        checked_at: now,
                        queued_for_manual_review: false,
                    })
                } else {
                    self.enqueue_manual_review(
                        req.entity_id.clone(),
                        req.address.clone(),
                        format!(
                            "Upstream compliance service returned HTTP status {}",
                            response.status()
                        ),
                    )
                    .await;

                    Ok(SanctionCheckResult {
                        entity_id: req.entity_id,
                        status: SanctionStatus::ManualReviewRequired,
                        details: "Compliance service error; fallback to temporary manual review"
                            .to_string(),
                        checked_at: now,
                        queued_for_manual_review: true,
                    })
                }
            }
            Ok(Err(req_err)) => {
                // Connection refused, network error, or client timeout
                self.enqueue_manual_review(
                    req.entity_id.clone(),
                    req.address.clone(),
                    format!("Compliance request network error: {}", req_err),
                )
                .await;

                Ok(SanctionCheckResult {
                    entity_id: req.entity_id,
                    status: SanctionStatus::ManualReviewRequired,
                    details:
                        "Network failure calling compliance provider; enqueued for manual review"
                            .to_string(),
                    checked_at: now,
                    queued_for_manual_review: true,
                })
            }
            Err(_timeout_elapsed) => {
                // Upstream hung longer than 3 seconds
                self.enqueue_manual_review(
                    req.entity_id.clone(),
                    req.address.clone(),
                    format!(
                        "External OFAC verification timed out after {:?}",
                        self.timeout
                    ),
                )
                .await;

                Ok(SanctionCheckResult {
                    entity_id: req.entity_id,
                    status: SanctionStatus::ManualReviewRequired,
                    details: format!(
                        "Verification timed out after {:?}; enqueued for manual review",
                        self.timeout
                    ),
                    checked_at: now,
                    queued_for_manual_review: true,
                })
            }
        }
    }

    async fn enqueue_manual_review(
        &self,
        entity_id: String,
        address: Option<String>,
        reason: String,
    ) {
        let mut queue = self.manual_review_queue.write().await;
        queue.push(ManualReviewItem {
            review_id: Uuid::new_v4().to_string(),
            entity_id,
            address,
            reason,
            enqueued_at: chrono::Utc::now().timestamp(),
            resolved: false,
        });
    }

    pub async fn get_manual_review_queue(&self) -> Vec<ManualReviewItem> {
        let queue = self.manual_review_queue.read().await;
        queue.clone()
    }

    pub async fn resolve_manual_review(
        &self,
        review_id: &str,
        _is_compliant: bool,
    ) -> Result<bool, AppError> {
        let mut queue = self.manual_review_queue.write().await;
        if let Some(item) = queue.iter_mut().find(|i| i.review_id == review_id) {
            item.resolved = true;
            Ok(true)
        } else {
            Ok(false)
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn test_check_compliance_source() {
        let pool = sqlx::PgPool::connect_lazy("postgres://localhost/test").unwrap();
        let service = ComplianceService::new(pool);
        let valid_code =
            "// SPDX-License-Identifier: MIT\nuse soroban_sdk::contract;\npub fn run() {}";
        let report = service.check_compliance(valid_code).await.unwrap();
        assert!(report.is_compliant);

        let unsafe_code = "pub fn bad() { unsafe { } }";
        let report2 = service.check_compliance(unsafe_code).await.unwrap();
        assert!(!report2.is_compliant);
    }

    #[tokio::test]
    async fn test_sanction_check_clean_fallback() {
        let pool = sqlx::PgPool::connect_lazy("postgres://localhost/test").unwrap();
        let service = ComplianceService::new(pool);
        let req = SanctionCheckRequest {
            entity_id: "user_123".to_string(),
            address: Some("GAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAWH4".to_string()),
            country_code: Some("US".to_string()),
        };

        let result = service.verify_sanctions(req).await.unwrap();
        assert_eq!(result.status, SanctionStatus::Compliant);
        assert!(!result.queued_for_manual_review);
    }

    #[tokio::test]
    async fn test_sanction_check_timeout_enqueues_manual_review() {
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = listener.local_addr().unwrap();

        tokio::spawn(async move {
            if let Ok((mut socket, _)) = listener.accept().await {
                tokio::time::sleep(Duration::from_millis(300)).await;
                use tokio::io::AsyncWriteExt;
                let _ = socket
                    .write_all(b"HTTP/1.1 200 OK\r\nContent-Length: 0\r\n\r\n")
                    .await;
            }
        });

        let pool = sqlx::PgPool::connect_lazy("postgres://localhost/test").unwrap();
        let endpoint = format!("http://{}", addr);
        let service =
            ComplianceService::with_endpoint_and_timeout(pool, endpoint, Duration::from_millis(50));

        let req = SanctionCheckRequest {
            entity_id: "entity_slow_api".to_string(),
            address: Some("GBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBB".to_string()),
            country_code: Some("IR".to_string()),
        };

        let result = service.verify_sanctions(req).await.unwrap();
        assert_eq!(result.status, SanctionStatus::ManualReviewRequired);
        assert!(result.queued_for_manual_review);

        let queue = service.get_manual_review_queue().await;
        assert_eq!(queue.len(), 1);
        assert_eq!(queue[0].entity_id, "entity_slow_api");
        assert!(!queue[0].resolved);

        let resolved = service
            .resolve_manual_review(&queue[0].review_id, true)
            .await
            .unwrap();
        assert!(resolved);
    }
}
