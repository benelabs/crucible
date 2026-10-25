//! HTTP handlers for the contract test coverage reporting API.
//!
//! Routes:
//!   POST /api/v1/coverage                      – submit a new coverage snapshot
//!   POST /api/v1/coverage/lcov                 – ingest an uploaded LCOV report (sanitizing SF: paths)
//!   GET  /api/v1/coverage/:project             – retrieve the latest snapshot for a project
//!   GET  /api/v1/coverage/:contract_id/history – retrieve progression history (composite index backed)
//!   GET  /api/v1/coverage/progression/:contract_id – alternative path for progression history

use axum::{
    extract::{Path, Query, State},
    http::StatusCode,
    response::IntoResponse,
    Json,
};
use serde::{Deserialize, Serialize};
use std::sync::Arc;

use crate::error::AppError;
use crate::services::test_coverage::{NewTestCoverage, TestCoverageService};

/// Shared state for coverage routes.
#[derive(Clone)]
pub struct CoverageState {
    pub service: TestCoverageService,
}

/// Query parameters for coverage historical queries.
#[derive(Debug, Clone, Default, Deserialize)]
pub struct CoverageHistoryQuery {
    pub limit: Option<i64>,
}

/// Request payload for uploading an LCOV format coverage report.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct LcovUploadRequest {
    pub project_name: String,
    pub branch: String,
    pub commit_sha: String,
    pub lcov_content: String,
    pub workspace_root: Option<String>,
}

/// Parsed file-level coverage information from an LCOV report.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct ParsedLcovFile {
    pub source_file: String,
    pub lines_found: i32,
    pub lines_hit: i32,
}

/// Aggregated result of parsing an LCOV coverage report.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct ParsedLcovReport {
    pub files: Vec<ParsedLcovFile>,
    pub total_lines: i32,
    pub covered_lines: i32,
    pub coverage_percent: f64,
}

/// Canonicalizes and sanitizes parsed source file paths from LCOV `SF:` lines
/// against the repository/workspace root, preventing directory traversal escapes (`../`).
pub fn sanitize_source_path(
    raw_path: &str,
    workspace_root: &std::path::Path,
) -> Result<std::path::PathBuf, AppError> {
    let raw_trimmed = raw_path.trim();
    if raw_trimmed.is_empty() {
        return Err(AppError::ValidationError(
            "Source file path in LCOV report cannot be empty".to_string(),
        ));
    }

    if raw_trimmed.contains('\0') {
        return Err(AppError::ValidationError(
            "Source file path contains illegal null bytes".to_string(),
        ));
    }

    let p = std::path::Path::new(raw_trimmed);

    // Normalize path components and strictly forbid parent dir escapes (`..`)
    let mut normalized = std::path::PathBuf::new();
    for component in p.components() {
        match component {
            std::path::Component::ParentDir => {
                if !normalized.pop() {
                    return Err(AppError::ValidationError(format!(
                        "Path traversal detected in LCOV source path: '{raw_trimmed}' escapes workspace root"
                    )));
                }
            }
            std::path::Component::Normal(c) => {
                normalized.push(c);
            }
            std::path::Component::CurDir => {}
            std::path::Component::RootDir | std::path::Component::Prefix(_) => {
                // Absolute paths must strictly reside inside the workspace root
                if !p.starts_with(workspace_root) {
                    return Err(AppError::ValidationError(format!(
                        "Absolute path in LCOV '{raw_trimmed}' is outside workspace root '{}'",
                        workspace_root.display()
                    )));
                }
                if let Ok(rel) = p.strip_prefix(workspace_root) {
                    normalized = rel.to_path_buf();
                } else {
                    return Err(AppError::ValidationError(format!(
                        "Path '{raw_trimmed}' does not reside within workspace root"
                    )));
                }
                break;
            }
        }
    }

    let target_path = workspace_root.join(&normalized);

    // Enforce safe path prefix validation
    if !target_path.starts_with(workspace_root) {
        return Err(AppError::ValidationError(format!(
            "Path traversal validation failed: '{raw_trimmed}' resolves outside workspace root"
        )));
    }

    Ok(target_path)
}

/// Parse an LCOV format coverage string, validating and sanitizing all SF: lines.
pub fn parse_lcov_report(
    lcov_content: &str,
    workspace_root: &std::path::Path,
) -> Result<ParsedLcovReport, AppError> {
    let mut files = Vec::new();
    let mut current_file: Option<String> = None;
    let mut current_lf: i32 = 0;
    let mut current_lh: i32 = 0;

    for line in lcov_content.lines() {
        let line = line.trim();
        if line.is_empty() {
            continue;
        }

        if let Some(rest) = line.strip_prefix("SF:") {
            // Sanitize source file path using safe prefix validation
            let sanitized = sanitize_source_path(rest, workspace_root)?;
            current_file = Some(sanitized.to_string_lossy().replace('\\', "/"));
            current_lf = 0;
            current_lh = 0;
        } else if let Some(rest) = line.strip_prefix("LF:") {
            if let Ok(val) = rest.trim().parse::<i32>() {
                current_lf = val;
            }
        } else if let Some(rest) = line.strip_prefix("LH:") {
            if let Ok(val) = rest.trim().parse::<i32>() {
                current_lh = val;
            }
        } else if line == "end_of_record" {
            if let Some(source_file) = current_file.take() {
                files.push(ParsedLcovFile {
                    source_file,
                    lines_found: current_lf,
                    lines_hit: current_lh,
                });
            }
        }
    }

    let total_lines: i32 = files.iter().map(|f| f.lines_found).sum();
    let covered_lines: i32 = files.iter().map(|f| f.lines_hit).sum();
    let coverage_percent = if total_lines > 0 {
        ((covered_lines as f64) / (total_lines as f64)) * 100.0
    } else {
        0.0
    };

    Ok(ParsedLcovReport {
        files,
        total_lines,
        covered_lines,
        coverage_percent,
    })
}

/// POST /api/v1/coverage
///
/// Accepts a JSON coverage payload and persists a new snapshot.
/// Returns `201 Created` with the stored record on success.
pub async fn submit_coverage(
    State(state): State<Arc<CoverageState>>,
    Json(payload): Json<NewTestCoverage>,
) -> Result<impl IntoResponse, AppError> {
    let record = state
        .service
        .submit_coverage(payload)
        .await
        .map_err(AppError::from)?;
    Ok((StatusCode::CREATED, Json(record)))
}

/// POST /api/v1/coverage/lcov
///
/// Ingests an uploaded LCOV coverage report, sanitizing source file paths against
/// the workspace root and persisting the resulting test coverage snapshot.
pub async fn submit_lcov_coverage(
    State(state): State<Arc<CoverageState>>,
    Json(payload): Json<LcovUploadRequest>,
) -> Result<impl IntoResponse, AppError> {
    let workspace_root_path = payload
        .workspace_root
        .as_deref()
        .map(std::path::PathBuf::from)
        .unwrap_or_else(|| {
            std::env::current_dir().unwrap_or_else(|_| std::path::PathBuf::from("/workspace"))
        });

    let parsed = parse_lcov_report(&payload.lcov_content, &workspace_root_path)?;

    let new_coverage = NewTestCoverage {
        contract_id: None,
        project_name: payload.project_name,
        branch: payload.branch,
        commit_sha: payload.commit_sha,
        coverage_percent: parsed.coverage_percent,
        total_lines: parsed.total_lines,
        covered_lines: parsed.covered_lines,
    };

    let record = state
        .service
        .submit_coverage(new_coverage)
        .await
        .map_err(AppError::from)?;

    Ok((
        StatusCode::CREATED,
        Json(serde_json::json!({
            "record": record,
            "parsedReport": parsed
        })),
    ))
}

/// GET /api/v1/coverage/:project
///
/// Returns the most-recent coverage snapshot for `project`.
/// Returns `404 Not Found` when no report has been submitted yet.
pub async fn get_latest_coverage(
    State(state): State<Arc<CoverageState>>,
    Path(project): Path<String>,
) -> Result<impl IntoResponse, AppError> {
    let record = state
        .service
        .get_latest_coverage(&project)
        .await
        .map_err(AppError::from)?;
    Ok(Json(record))
}

/// GET /api/v1/coverage/:contract_id/history
/// GET /api/v1/coverage/progression/:contract_id
///
/// Returns historical coverage records sorted by `created_at DESC`.
/// Supports `?limit=N` (defaults to 20, clamped 1..=100).
pub async fn get_coverage_progression(
    State(state): State<Arc<CoverageState>>,
    Path(contract_id): Path<String>,
    Query(query): Query<CoverageHistoryQuery>,
) -> Result<impl IntoResponse, AppError> {
    let limit = query.limit.unwrap_or(20);
    let records = state
        .service
        .get_coverage_progression(&contract_id, limit)
        .await
        .map_err(AppError::from)?;
    Ok(Json(records))
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::Path;

    #[test]
    fn query_deserializes_default() {
        let q: CoverageHistoryQuery = serde_json::from_str("{}").unwrap();
        assert_eq!(q.limit, None);
    }

    #[test]
    fn query_deserializes_limit() {
        let q: CoverageHistoryQuery = serde_json::from_str(r#"{"limit":50}"#).unwrap();
        assert_eq!(q.limit, Some(50));
    }

    #[test]
    fn test_sanitize_source_path_valid_relative() {
        let root = Path::new("/workspace/project");
        let result = sanitize_source_path("src/lib.rs", root).unwrap();
        assert_eq!(result, root.join("src/lib.rs"));
    }

    #[test]
    fn test_sanitize_source_path_rejects_parent_traversal() {
        let root = Path::new("/workspace/project");
        let result = sanitize_source_path("../../etc/passwd", root);
        assert!(matches!(result, Err(AppError::ValidationError(_))));
        let err_msg = result.unwrap_err().to_string();
        assert!(err_msg.contains("Path traversal detected"));
    }

    #[test]
    fn test_sanitize_source_path_rejects_embedded_traversal() {
        let root = Path::new("/workspace/project");
        let result = sanitize_source_path("src/deep/../../../../escape.rs", root);
        assert!(matches!(result, Err(AppError::ValidationError(_))));
    }

    #[test]
    fn test_sanitize_source_path_rejects_null_bytes() {
        let root = Path::new("/workspace/project");
        let result = sanitize_source_path("src/lib.rs\0malicious", root);
        assert!(matches!(result, Err(AppError::ValidationError(_))));
        assert!(result.unwrap_err().to_string().contains("null bytes"));
    }

    #[test]
    fn test_sanitize_source_path_rejects_absolute_path_outside_root() {
        let root = Path::new("/workspace/project");
        let result = sanitize_source_path("/etc/shadow", root);
        assert!(matches!(result, Err(AppError::ValidationError(_))));
        assert!(result
            .unwrap_err()
            .to_string()
            .contains("outside workspace root"));
    }

    #[test]
    fn test_sanitize_source_path_accepts_absolute_path_within_root() {
        let root = Path::new("/workspace/project");
        let result = sanitize_source_path("/workspace/project/src/models.rs", root).unwrap();
        assert_eq!(result, root.join("src/models.rs"));
    }

    #[test]
    fn test_parse_lcov_report_valid() {
        let root = Path::new("/workspace/project");
        let lcov = r#"
TN:test_suite
SF:src/lib.rs
DA:1,5
DA:2,0
LF:2
LH:1
end_of_record
SF:src/utils.rs
DA:1,10
DA:2,10
LF:2
LH:2
end_of_record
"#;
        let report = parse_lcov_report(lcov, root).unwrap();
        assert_eq!(report.files.len(), 2);
        assert_eq!(report.total_lines, 4);
        assert_eq!(report.covered_lines, 3);
        assert!((report.coverage_percent - 75.0).abs() < 1e-6);
    }

    #[test]
    fn test_parse_lcov_report_rejects_traversal() {
        let root = Path::new("/workspace/project");
        let lcov = r#"
TN:malicious
SF:../../../../etc/passwd
DA:1,1
LF:1
LH:1
end_of_record
"#;
        let result = parse_lcov_report(lcov, root);
        assert!(matches!(result, Err(AppError::ValidationError(_))));
    }
}
