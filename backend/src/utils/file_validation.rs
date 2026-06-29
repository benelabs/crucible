//! File upload validation utilities.
//!
//! Validates uploaded files for size, MIME type, and file name safety before
//! they are persisted or processed. Uses magic-byte sniffing to verify that
//! the declared content type matches the actual file content.

use std::collections::HashSet;
use tracing::{debug, instrument, warn};

use crate::utils::errors::FileError;

// ---------------------------------------------------------------------------
// Configuration
// ---------------------------------------------------------------------------

/// Default maximum file size: 10 MiB.
pub const DEFAULT_MAX_SIZE: u64 = 10 * 1024 * 1024;

/// Allowed MIME types for contract-related uploads.
pub const ALLOWED_MIME_TYPES: &[&str] = &[
    "application/wasm",
    "application/octet-stream",
    "application/json",
    "text/plain",
    "text/x-rust",
];

/// Configuration for file upload validation.
#[derive(Debug, Clone)]
pub struct ValidationConfig {
    /// Maximum allowed file size in bytes.
    pub max_size: u64,
    /// Set of permitted MIME types.
    pub allowed_mime_types: HashSet<String>,
}

impl Default for ValidationConfig {
    fn default() -> Self {
        Self {
            max_size: DEFAULT_MAX_SIZE,
            allowed_mime_types: ALLOWED_MIME_TYPES
                .iter()
                .map(|s| s.to_string())
                .collect(),
        }
    }
}

impl ValidationConfig {
    /// Create a config with a custom size limit and the default MIME allow-list.
    pub fn with_max_size(max_size: u64) -> Self {
        Self { max_size, ..Default::default() }
    }

    /// Add an extra allowed MIME type.
    pub fn allow_mime(mut self, mime: impl Into<String>) -> Self {
        self.allowed_mime_types.insert(mime.into());
        self
    }
}

// ---------------------------------------------------------------------------
// Validation result
// ---------------------------------------------------------------------------

/// Metadata produced after a successful validation pass.
#[derive(Debug, Clone)]
pub struct ValidatedFile {
    /// Original file name (sanitised).
    pub file_name: String,
    /// Detected MIME type.
    pub mime_type: String,
    /// File size in bytes.
    pub size: u64,
    /// Raw file bytes.
    pub bytes: Vec<u8>,
}

// ---------------------------------------------------------------------------
// Core validator
// ---------------------------------------------------------------------------

/// Validates a raw file upload against the provided configuration.
///
/// # Errors
///
/// Returns [`FileError`] if any validation step fails.
#[instrument(skip(bytes, config), fields(file_name = %file_name, size = bytes.len()))]
pub fn validate_upload(
    file_name: &str,
    declared_mime: &str,
    bytes: Vec<u8>,
    config: &ValidationConfig,
) -> Result<ValidatedFile, FileError> {
    let size = bytes.len() as u64;

    // 1. Size check
    if size > config.max_size {
        warn!(size, limit = config.max_size, "File exceeds size limit");
        return Err(FileError::TooLarge { size, limit: config.max_size });
    }

    // 2. File name safety
    let safe_name = sanitize_file_name(file_name)?;

    // 3. MIME type check (declared)
    let normalized_mime = declared_mime.split(';').next().unwrap_or("").trim().to_lowercase();
    if !config.allowed_mime_types.contains(&normalized_mime) {
        warn!(mime = %normalized_mime, "Unsupported MIME type");
        return Err(FileError::UnsupportedMimeType(normalized_mime));
    }

    // 4. Magic-byte verification
    let detected = detect_mime(&bytes);
    if let Some(detected_mime) = detected {
        if detected_mime != normalized_mime
            && !is_compatible_mime(&normalized_mime, detected_mime)
        {
            warn!(
                declared = %normalized_mime,
                detected = %detected_mime,
                "MIME type mismatch between declared and detected"
            );
            return Err(FileError::MalformedContent(format!(
                "declared MIME '{normalized_mime}' does not match detected '{detected_mime}'"
            )));
        }
    }

    debug!(file_name = %safe_name, size, mime = %normalized_mime, "File validation passed");

    Ok(ValidatedFile {
        file_name: safe_name,
        mime_type: normalized_mime,
        size,
        bytes,
    })
}

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

/// Sanitise a file name: reject path traversal, null bytes, and empty names.
fn sanitize_file_name(name: &str) -> Result<String, FileError> {
    if name.is_empty() {
        return Err(FileError::InvalidFileName("file name is empty".into()));
    }
    if name.contains('\0') {
        return Err(FileError::InvalidFileName("file name contains null byte".into()));
    }
    // Strip any directory component — keep only the final segment.
    let base = name
        .replace('\\', "/")
        .split('/')
        .filter(|s| !s.is_empty() && *s != "..")
        .last()
        .unwrap_or("")
        .to_string();

    if base.is_empty() || base == ".." {
        return Err(FileError::InvalidFileName(format!("unsafe file name: {name}")));
    }
    Ok(base)
}

/// Detect MIME type from magic bytes.
fn detect_mime(bytes: &[u8]) -> Option<&'static str> {
    // WebAssembly magic: \0asm
    if bytes.starts_with(&[0x00, 0x61, 0x73, 0x6d]) {
        return Some("application/wasm");
    }

    let data = if bytes.starts_with(&[0xEF, 0xBB, 0xBF]) {
        &bytes[3..]
    } else {
        bytes
    };

    if data
        .iter()
        .position(|b| !b.is_ascii_whitespace())
        .map(|i| matches!(data[i], b'{' | b'['))
        .unwrap_or(false)
    {
        return Some("application/json");
    }

    if is_valid_text(bytes) {
        return Some("text/plain");
    }

    Some("application/octet-stream")
}

fn is_valid_text(bytes: &[u8]) -> bool {
    match std::str::from_utf8(bytes) {
        Ok(text) => text.chars().all(|c| {
            c == '\n' || c == '\r' || c == '\t' || !c.is_control()
        }),
        Err(_) => false,
    }
}

/// Returns true when the declared and detected MIME types are compatible
/// (e.g. `application/octet-stream` is a valid fallback for any binary).
fn is_compatible_mime(declared: &str, detected: &str) -> bool {
    declared == "application/octet-stream"
        || detected == "application/octet-stream"
        || (declared.starts_with("text/") && detected.starts_with("text/"))
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    fn cfg() -> ValidationConfig {
        ValidationConfig::default()
    }

    #[test]
    fn valid_wasm_upload() {
        let bytes = vec![0x00, 0x61, 0x73, 0x6d, 0x01, 0x00, 0x00, 0x00];
        let result = validate_upload("contract.wasm", "application/wasm", bytes, &cfg());
        assert!(result.is_ok());
        let f = result.unwrap();
        assert_eq!(f.file_name, "contract.wasm");
        assert_eq!(f.mime_type, "application/wasm");
    }

    #[test]
    fn valid_json_upload() {
        let bytes = br#"{"name":"test"}"#.to_vec();
        let result = validate_upload("meta.json", "application/json", bytes, &cfg());
        assert!(result.is_ok());
    }

    #[test]
    fn rejects_file_too_large() {
        let bytes = vec![0u8; 11 * 1024 * 1024];
        let result = validate_upload("big.wasm", "application/wasm", bytes, &cfg());
        assert!(matches!(result, Err(FileError::TooLarge { .. })));
    }

    #[test]
    fn rejects_unsupported_mime() {
        let bytes = b"data".to_vec();
        let result = validate_upload("file.exe", "application/exe", bytes, &cfg());
        assert!(matches!(result, Err(FileError::UnsupportedMimeType(_))));
    }

    #[test]
    fn rejects_path_traversal() {
        let bytes = b"data".to_vec();
        let result = validate_upload("../../etc/passwd", "text/plain", bytes, &cfg());
        // Should either sanitise or reject
        match result {
            Ok(f) => assert!(!f.file_name.contains("..")),
            Err(FileError::InvalidFileName(_)) => {}
            Err(e) => panic!("unexpected error: {e}"),
        }
    }

    #[test]
    fn rejects_empty_file_name() {
        let bytes = b"data".to_vec();
        let result = validate_upload("", "text/plain", bytes, &cfg());
        assert!(matches!(result, Err(FileError::InvalidFileName(_))));
    }

    #[test]
    fn strips_directory_prefix() {
        let bytes = b"hello".to_vec();
        let result = validate_upload("uploads/contract.txt", "text/plain", bytes, &cfg());
        assert!(result.is_ok());
        assert_eq!(result.unwrap().file_name, "contract.txt");
    }

    #[test]
    fn rejects_mime_mismatch() {
        // Declare JSON but send WASM magic bytes
        let bytes = vec![0x00, 0x61, 0x73, 0x6d, 0x01, 0x00, 0x00, 0x00];
        let result = validate_upload("contract.json", "application/json", bytes, &cfg());
        assert!(matches!(result, Err(FileError::MalformedContent(_))));
    }

    #[test]
    fn octet_stream_is_compatible_with_any() {
        let bytes = vec![0x00, 0x61, 0x73, 0x6d, 0x01, 0x00, 0x00, 0x00];
        let result = validate_upload("contract.wasm", "application/octet-stream", bytes, &cfg());
        assert!(result.is_ok());
    }

    #[test]
    fn custom_config_allows_extra_mime() {
        let cfg = ValidationConfig::default().allow_mime("image/png");
        let bytes = b"PNG data".to_vec();
        // Will fail magic check but pass MIME allow-list check
        let result = validate_upload("logo.png", "image/png", bytes, &cfg);
        // text/plain detected for ASCII bytes — compatible via text/* rule? No.
        // Just verify the MIME allow-list step passes (error is MalformedContent, not UnsupportedMimeType)
        assert!(!matches!(result, Err(FileError::UnsupportedMimeType(_))));
    }

    #[test]
    fn sanitize_null_byte_rejected() {
        let result = sanitize_file_name("file\0name.txt");
        assert!(matches!(result, Err(FileError::InvalidFileName(_))));
    }

    #[test]
    fn detect_mime_wasm() {
        let bytes = vec![0x00, 0x61, 0x73, 0x6d];
        assert_eq!(detect_mime(&bytes), Some("application/wasm"));
    }

    #[test]
    fn detect_mime_json() {
        let bytes = b"{\"key\":\"val\"}".to_vec();
        assert_eq!(detect_mime(&bytes), Some("application/json"));
    }
}
