use axum::{
    async_trait,
    extract::{FromRequest, Json, Request},
    http::StatusCode,
    response::{IntoResponse, Response},
};
use serde::{Deserialize, Serialize};
use thiserror::Error;
use tracing::{error, warn};

/// Standard API result type returned by handlers.
pub type ApiResult<T> = Result<ApiResponse<T>, ApiError>;

// ---------------------------------------------------------------------------
// Error Handling Contract
// ---------------------------------------------------------------------------

/// Standardized API error response.
#[derive(Debug, Serialize, Deserialize)]
pub struct ApiErrorResponse {
    pub code: String,
    pub message: String,
    pub timestamp: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub details: Option<serde_json::Value>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub request_id: Option<String>,
}

/// Errors that can occur in the API.
#[derive(Debug, Error)]
#[allow(dead_code)]
pub enum ApiError {
    #[error("Validation failed: {0}")]
    Validation(String),

    #[error("Database error: {0}")]
    Database(#[from] sqlx::Error),

    #[error("Redis error: {0}")]
    Redis(#[from] redis::RedisError),

    #[error("Internal server error: {0}")]
    Internal(String),

    #[error("Not found: {0}")]
    NotFound(String),

    #[error("Unauthorized")]
    Unauthorized,

    #[error("Forbidden")]
    Forbidden,

    #[error("Conflict: {0}")]
    Conflict(String),

    #[error("Too many requests")]
    RateLimited,

    #[error("Length required: {0}")]
    LengthRequired(String),
}

impl ApiError {
    /// Returns the HTTP status code mapped to this API error.
    pub fn status_code(&self) -> StatusCode {
        match self {
            Self::Validation(_) => StatusCode::BAD_REQUEST,
            Self::Database(_) => StatusCode::INTERNAL_SERVER_ERROR,
            Self::Redis(_) => StatusCode::INTERNAL_SERVER_ERROR,
            Self::Internal(_) => StatusCode::INTERNAL_SERVER_ERROR,
            Self::NotFound(_) => StatusCode::NOT_FOUND,
            Self::Unauthorized => StatusCode::UNAUTHORIZED,
            Self::Forbidden => StatusCode::FORBIDDEN,
            Self::Conflict(_) => StatusCode::CONFLICT,
            Self::RateLimited => StatusCode::TOO_MANY_REQUESTS,
            Self::LengthRequired(_) => StatusCode::LENGTH_REQUIRED,
        }
    }

    /// Returns the stable machine-readable error code.
    pub fn error_code(&self) -> String {
        match self {
            Self::Validation(_) => "VALIDATION_ERROR".to_string(),
            Self::Database(_) => "DATABASE_ERROR".to_string(),
            Self::Redis(_) => "REDIS_ERROR".to_string(),
            Self::Internal(_) => "INTERNAL_ERROR".to_string(),
            Self::NotFound(_) => "NOT_FOUND".to_string(),
            Self::Unauthorized => "UNAUTHORIZED".to_string(),
            Self::Forbidden => "FORBIDDEN".to_string(),
            Self::Conflict(_) => "CONFLICT".to_string(),
            Self::RateLimited => "RATE_LIMITED".to_string(),
            Self::LengthRequired(_) => "LENGTH_REQUIRED".to_string(),
        }
    }
}

impl IntoResponse for ApiError {
    fn into_response(self) -> Response {
        let status = self.status_code();
        let body = Json(ApiErrorResponse {
            code: self.error_code(),
            message: self.to_string(),
            timestamp: chrono::Utc::now().to_rfc3339(),
            details: None,
            request_id: None,
        });

        if status.is_server_error() {
            error!(error = %self, "API Server Error");
        } else {
            warn!(error = %self, "API Client Error");
        }

        (status, body).into_response()
    }
}

// ---------------------------------------------------------------------------
// Success Response Contract
// ---------------------------------------------------------------------------

/// Standardized API success wrapper.
#[derive(Debug, Serialize, Deserialize)]
pub struct ApiResponse<T> {
    pub data: T,
    pub status: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub meta: Option<ResponseMeta>,
}

impl<T> ApiResponse<T> {
    /// Builds a successful API response without metadata.
    pub fn new(data: T) -> Self {
        Self {
            data,
            status: "success".to_string(),
            meta: None,
        }
    }

    /// Attaches response metadata such as pagination or request correlation.
    pub fn with_meta(mut self, meta: ResponseMeta) -> Self {
        self.meta = Some(meta);
        self
    }
}

impl<T: Serialize> IntoResponse for ApiResponse<T> {
    fn into_response(self) -> Response {
        (StatusCode::OK, Json(self)).into_response()
    }
}

/// Optional response metadata shared by list and asynchronous endpoints.
#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq, Eq)]
pub struct ResponseMeta {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub request_id: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub pagination: Option<PaginationMeta>,
}

/// Pagination metadata for bounded list responses.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct PaginationMeta {
    pub limit: u32,
    pub offset: u32,
    pub total: u64,
    pub has_more: bool,
}

impl PaginationMeta {
    /// Creates pagination metadata from the requested window and total count.
    pub fn new(limit: u32, offset: u32, total: u64) -> Self {
        let next_offset = u64::from(offset) + u64::from(limit);
        Self {
            limit,
            offset,
            total,
            has_more: next_offset < total,
        }
    }
}

// ---------------------------------------------------------------------------
// Validation Contract
// ---------------------------------------------------------------------------

/// Trait for types that can be validated.
pub trait Validate {
    /// Perform validation.
    ///
    /// # Errors
    /// Returns a string describing the validation failure.
    fn validate(&self) -> Result<(), String>;
}

/// Axum extractor that performs validation after JSON deserialization.
pub struct ValidatedJson<T>(pub T);

#[async_trait]
impl<S, T> FromRequest<S> for ValidatedJson<T>
where
    T: serde::de::DeserializeOwned + Validate,
    S: Send + Sync,
{
    type Rejection = ApiError;

    async fn from_request(req: Request, state: &S) -> Result<Self, Self::Rejection> {
        let Json(value) = Json::<T>::from_request(req, state).await.map_err(
            |e: axum::extract::rejection::JsonRejection| ApiError::Validation(e.to_string()),
        )?;

        value.validate().map_err(ApiError::Validation)?;

        Ok(ValidatedJson(value))
    }
}

/// Common query contract for bounded collection endpoints.
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
pub struct Pagination {
    #[serde(default = "Pagination::default_limit")]
    pub limit: u32,
    #[serde(default)]
    pub offset: u32,
}

impl Default for Pagination {
    fn default() -> Self {
        Self {
            limit: Self::default_limit(),
            offset: 0,
        }
    }
}

impl Pagination {
    const MAX_LIMIT: u32 = 500;

    fn default_limit() -> u32 {
        100
    }
}

impl Validate for Pagination {
    fn validate(&self) -> Result<(), String> {
        if self.limit == 0 || self.limit > Self::MAX_LIMIT {
            return Err(format!("limit must be between 1 and {}", Self::MAX_LIMIT));
        }
        Ok(())
    }
}

// ---------------------------------------------------------------------------
// Domain Contracts
// ---------------------------------------------------------------------------

/// Contract for System Status response.
#[derive(Debug, Serialize, Deserialize, Clone)]
pub struct SystemStatus {
    pub status: String,
    pub uptime_secs: u64,
    pub memory_used_bytes: u64,
    pub active_recovery_tasks: usize,
}

/// Contract for Profiling trigger request.
#[derive(Debug, Serialize, Deserialize, Clone)]
pub struct ProfileTriggerRequest {
    pub duration_secs: u32,
    pub sample_rate_hz: u32,
    pub label: String,
}

impl Validate for ProfileTriggerRequest {
    fn validate(&self) -> Result<(), String> {
        if self.duration_secs == 0 || self.duration_secs > 3600 {
            return Err("duration_secs must be between 1 and 3600".to_string());
        }
        if self.sample_rate_hz == 0 || self.sample_rate_hz > 1000 {
            return Err("sample_rate_hz must be between 1 and 1000".to_string());
        }
        if self.label.is_empty() {
            return Err("label cannot be empty".to_string());
        }
        Ok(())
    }
}

/// Contract for Profiling trigger response.
#[derive(Debug, Serialize, Deserialize, Clone)]
pub struct ProfileTriggerResponse {
    pub profile_id: uuid::Uuid,
    pub message: String,
    pub estimated_completion: chrono::DateTime<chrono::Utc>,
}

/// Stellar RPC transaction status values returned by `getTransaction`.
///
/// The Stellar RPC API currently documents `SUCCESS`, `NOT_FOUND`, and `FAILED`
/// as the allowed values for transaction lookup results.
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "SCREAMING_SNAKE_CASE")]
pub enum StellarTransactionStatus {
    Success,
    NotFound,
    Failed,
}

/// Request contract for looking up a Stellar transaction by hash.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct StellarTransactionLookupRequest {
    pub hash: String,
    #[serde(default)]
    pub xdr_format: XdrFormat,
}

impl Validate for StellarTransactionLookupRequest {
    fn validate(&self) -> Result<(), String> {
        validate_stellar_transaction_hash(&self.hash)
    }
}

/// Stellar RPC XDR response encoding preference.
#[derive(Debug, Clone, Copy, Default, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "lowercase")]
pub enum XdrFormat {
    #[default]
    Base64,
    Json,
}

/// Type-safe subset of the Stellar RPC `getTransaction` result.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct StellarTransactionResponse {
    pub status: StellarTransactionStatus,
    pub latest_ledger: u32,
    pub latest_ledger_close_time: u64,
    pub oldest_ledger: u32,
    pub oldest_ledger_close_time: u64,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub ledger: Option<u32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub created_at: Option<u64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub application_order: Option<u32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub fee_bump: Option<bool>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub envelope_xdr: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub result_xdr: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub result_meta_xdr: Option<String>,
}

/// Validates a Stellar transaction hash as a 64-character lowercase hex value.
pub fn validate_stellar_transaction_hash(hash: &str) -> Result<(), String> {
    if hash.len() != 64 {
        return Err("hash must be exactly 64 lowercase hexadecimal characters".to_string());
    }

    if !hash
        .bytes()
        .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte))
    {
        return Err("hash must contain only lowercase hexadecimal characters".to_string());
    }

    Ok(())
}

/// Cache analytics request contract shared by API handlers and services.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct CacheMetricsQuery {
    #[serde(default)]
    pub namespace: Option<String>,
    #[serde(default)]
    pub from: Option<chrono::DateTime<chrono::Utc>>,
    #[serde(default)]
    pub to: Option<chrono::DateTime<chrono::Utc>>,
    #[serde(default)]
    pub pagination: Pagination,
}

impl Validate for CacheMetricsQuery {
    fn validate(&self) -> Result<(), String> {
        self.pagination.validate()?;
        if let (Some(from), Some(to)) = (self.from.as_ref(), self.to.as_ref()) {
            if from > to {
                return Err("from must be less than or equal to to".to_string());
            }
        }
        if let Some(namespace) = &self.namespace {
            validate_identifier(namespace, "namespace")?;
        }
        Ok(())
    }
}

fn validate_identifier(value: &str, field: &str) -> Result<(), String> {
    if value.is_empty() || value.len() > 128 {
        return Err(format!("{field} must be between 1 and 128 characters"));
    }

    if !value
        .bytes()
        .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'_' | b'-' | b':' | b'.'))
    {
        return Err(format!(
            "{field} may contain only letters, numbers, '_', '-', ':' or '.'"
        ));
    }

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_validation_success() {
        let req = ProfileTriggerRequest {
            duration_secs: 60,
            sample_rate_hz: 10,
            label: "test-run".to_string(),
        };
        assert!(req.validate().is_ok());
    }

    #[test]
    fn test_validation_failure_duration() {
        let req = ProfileTriggerRequest {
            duration_secs: 0,
            sample_rate_hz: 10,
            label: "test-run".to_string(),
        };
        assert!(req.validate().is_err());
    }

    #[test]
    fn test_validation_failure_label() {
        let req = ProfileTriggerRequest {
            duration_secs: 60,
            sample_rate_hz: 10,
            label: "".to_string(),
        };
        assert!(req.validate().is_err());
    }

    #[test]
    fn test_api_response_serialization() {
        let resp = ApiResponse::new("test data");
        let json = serde_json::to_string(&resp).unwrap();
        assert!(json.contains("\"data\":\"test data\""));
        assert!(json.contains("\"status\":\"success\""));
        assert!(!json.contains("\"meta\""));
    }

    #[test]
    fn test_api_error_status_codes() {
        let err = ApiError::Validation("invalid".to_string());
        assert_eq!(err.status_code(), StatusCode::BAD_REQUEST);

        let err = ApiError::Internal("oops".to_string());
        assert_eq!(err.status_code(), StatusCode::INTERNAL_SERVER_ERROR);

        let err = ApiError::LengthRequired("length required".to_string());
        assert_eq!(err.status_code(), StatusCode::LENGTH_REQUIRED);
    }

    #[test]
    fn test_pagination_metadata_has_more() {
        let meta = PaginationMeta::new(25, 50, 100);
        assert!(meta.has_more);

        let meta = PaginationMeta::new(50, 50, 100);
        assert!(!meta.has_more);
    }

    #[test]
    fn test_pagination_validation() {
        assert!(Pagination {
            limit: 1,
            offset: 0
        }
        .validate()
        .is_ok());
        assert!(Pagination {
            limit: 501,
            offset: 0
        }
        .validate()
        .is_err());
    }

    #[test]
    fn test_stellar_transaction_hash_validation() {
        let hash = "32f7e5c3afd281fcaa99c0e990adf62f33e3bb341b1641a5c8b0b4a4dc55c487";
        assert!(validate_stellar_transaction_hash(hash).is_ok());
        assert!(validate_stellar_transaction_hash("ABC").is_err());
        assert!(validate_stellar_transaction_hash(
            "32F7E5C3AFD281FCAA99C0E990ADF62F33E3BB341B1641A5C8B0B4A4DC55C487"
        )
        .is_err());
    }

    #[test]
    fn test_cache_metrics_query_validation() {
        let query = CacheMetricsQuery {
            namespace: Some("api:v1".to_string()),
            from: None,
            to: None,
            pagination: Pagination::default(),
        };
        assert!(query.validate().is_ok());

        let query = CacheMetricsQuery {
            namespace: Some("bad namespace".to_string()),
            from: None,
            to: None,
            pagination: Pagination::default(),
        };
        assert!(query.validate().is_err());
    }
}
