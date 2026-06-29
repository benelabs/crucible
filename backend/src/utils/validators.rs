//! Comprehensive Request Validation System
//!
//! This module provides a comprehensive set of validation utilities for HTTP request
//! data, including string, numeric, date/time, and custom business logic validation.

use std::collections::HashMap;
use std::fmt::{self, Display, Formatter};
use std::str::FromStr;
use chrono::{DateTime, Utc, ParseError as ChronoParseError};
use regex::Regex;
use serde::{Deserialize, Serialize};
use tracing::{debug, error, info};

/// Validation result type
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ValidationResult {
    /// Validation passed successfully
    Valid,
    /// Validation failed with error message
    Invalid(String),
    /// Validation encountered an internal error
    Error(String),
}

impl Display for ValidationResult {
    fn fmt(&self, f: &mut Formatter<'_>) -> fmt::Result {
        match self {
            ValidationResult::Valid => write!(f, "valid"),
            ValidationResult::Invalid(msg) => write!(f, "invalid: {}", msg),
            ValidationResult::Error(msg) => write!(f, "error: {}", msg),
        }
    }
}

/// Validation error type
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ValidationError {
    /// Field name that failed validation
    pub field: String,
    /// Validation rule that failed
    pub rule: String,
    /// Error message
    pub message: String,
    /// Timestamp of validation failure
    pub timestamp: DateTime<Utc>,
}

impl ValidationError {
    pub fn new(field: String, rule: String, message: String) -> Self {
        Self {
            field,
            rule,
            message,
            timestamp: Utc::now(),
        }
    }
}

/// Validation context for tracking multiple validation results
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct ValidationContext {
    /// List of validation errors
    pub errors: Vec<ValidationError>,
    /// Whether validation passed overall
    pub is_valid: bool,
}

impl ValidationContext {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn add_error(&mut self, error: ValidationError) {
        self.errors.push(error);
        self.is_valid = false;
    }

    pub fn add_errors(&mut self, errors: Vec<ValidationError>) {
        if !errors.is_empty() {
            self.is_valid = false;
        }
        self.errors.extend(errors);
    }

    pub fn is_valid(&self) -> bool {
        self.is_valid && self.errors.is_empty()
    }

    pub fn get_errors(&self) -> &[ValidationError] {
        &self.errors
    }
}

/// String validation rules
pub struct StringValidator;

impl StringValidator {
    /// Validate string length (min/max)
    pub fn length(value: &str, min: usize, max: usize) -> ValidationResult {
        if value.len() < min {
            ValidationResult::Invalid(format!("string length must be at least {} characters", min))
        } else if value.len() > max {
            ValidationResult::Invalid(format!("string length must be at most {} characters", max))
        } else {
            ValidationResult::Valid
        }
    }

    /// Validate string is not empty
    pub fn required(value: &str) -> ValidationResult {
        if value.trim().is_empty() {
            ValidationResult::Invalid("field is required".to_string())
        } else {
            ValidationResult::Valid
        }
    }

    /// Validate string matches regex pattern
    pub fn regex(value: &str, pattern: &str) -> ValidationResult {
        match regex::Regex::new(pattern) {
            Ok(re) => {
                if regex::Regex::is_match(&re, value) {
                    ValidationResult::Valid
                } else {
                    ValidationResult::Invalid(format!("string does not match pattern '{}'", pattern))
                }
            }
            Err(e) => ValidationResult::Error(format!("invalid regex pattern '{}': {}", pattern, e)),
        }
    }

    /// Validate email format
    pub fn email(value: &str) -> ValidationResult {
        // Simple email validation - more sophisticated validation would use a proper email library
        let email_regex = r"^[a-zA-Z0-9._%+-]+@[a-zA-Z0-9.-]+\\.[a-zA-Z]{2,}$";
        Self::regex(value, email_regex)
    }

    /// Validate URL format
    pub fn url(value: &str) -> ValidationResult {
        let url_regex = r"^https?://[^s/$.?#].[^s]*$";
        Self::regex(value, url_regex)
    }

    /// Validate phone number format
    pub fn phone(value: &str) -> ValidationResult {
        // Basic phone number validation
        let phone_regex = r"^\\+?[1-9]\\d{1,14}$";
        Self::regex(value, phone_regex)
    }

    /// Validate string contains only alphanumeric characters
    pub fn alphanumeric(value: &str) -> ValidationResult {
        if value.chars().all(|c| c.is_alphanumeric()) {
            ValidationResult::Valid
        } else {
            ValidationResult::Invalid("string must contain only alphanumeric characters".to_string())
        }
    }

    /// Validate string contains only letters
    pub fn letters(value: &str) -> ValidationResult {
        if value.chars().all(|c| c.is_alphabetic()) {
            ValidationResult::Valid
        } else {
            ValidationResult::Invalid("string must contain only letters".to_string())
        }
    }
}

/// Numeric validation rules
pub struct NumberValidator;

impl NumberValidator {
    /// Validate number is within range
    pub fn range<T>(value: T, min: T, max: T) -> ValidationResult 
    where
        T: PartialOrd + Display + Copy,
    {
        if value < min {
            ValidationResult::Invalid(format!("value must be at least {}", min))
        } else if value > max {
            ValidationResult::Invalid(format!("value must be at most {}", max))
        } else {
            ValidationResult::Valid
        }
    }

    /// Validate number is positive
    pub fn positive<T>(value: T) -> ValidationResult 
    where
        T: PartialOrd + Display + Copy + From<u8>,
    {
        if value < T::from(0u8) {
            ValidationResult::Invalid("value must be positive".to_string())
        } else {
            ValidationResult::Valid
        }
    }

    /// Validate number is non-negative
    pub fn non_negative<T>(value: T) -> ValidationResult 
    where
        T: PartialOrd + Display + Copy + From<u8>,
    {
        if value < T::from(0u8) {
            ValidationResult::Invalid("value must be non-negative".to_string())
        } else {
            ValidationResult::Valid
        }
    }
}

/// Date/time validation rules
pub struct DateTimeValidator;

impl DateTimeValidator {
    /// Validate date/time string can be parsed
    pub fn parse_datetime(value: &str) -> Result<DateTime<Utc>, String> {
        // Try common datetime formats
        let formats = [
            "%Y-%m-%dT%H:%M:%S%.fZ",
            "%Y-%m-%dT%H:%M:%SZ",
            "%Y-%m-%dT%H:%M:%S%.f%z",
            "%Y-%m-%dT%H:%M:%S%z",
            "%Y-%m-%d %H:%M:%S%.fZ",
            "%Y-%m-%d %H:%M:%SZ",
        ];
        
        for format in &formats {
            if let Ok(dt) = DateTime::parse_from_str(value, format) {
                return Ok(dt.to_utc());
            }
        }
        
        // Try ISO 8601 basic format
        if let Ok(dt) = DateTime::parse_from_rfc3339(value) {
            return Ok(dt.to_utc());
        }
        
        Err("could not parse datetime".to_string())
    }

    /// Validate datetime is in the past
    pub fn past(value: &str) -> ValidationResult {
        match Self::parse_datetime(value) {
            Ok(dt) => {
                if dt < Utc::now() {
                    ValidationResult::Valid
                } else {
                    ValidationResult::Invalid("datetime must be in the past".to_string())
                }
            }
            Err(e) => ValidationResult::Error(format!("invalid datetime format: {}", e)),
        }
    }

    /// Validate datetime is in the future
    pub fn future(value: &str) -> ValidationResult {
        match Self::parse_datetime(value) {
            Ok(dt) => {
                if dt > Utc::now() {
                    ValidationResult::Valid
                } else {
                    ValidationResult::Invalid("datetime must be in the future".to_string())
                }
            }
            Err(e) => ValidationResult::Error(format!("invalid datetime format: {}", e)),
        }
    }
}

/// Custom validation rules
pub struct CustomValidator;

impl CustomValidator {
    /// Validate using custom function
    pub fn custom<F, T>(value: T, validator: F, message: &str) -> ValidationResult 
    where
        F: FnOnce(T) -> bool,
    {
        if validator(value) {
            ValidationResult::Valid
        } else {
            ValidationResult::Invalid(message.to_string())
        }
    }

    /// Validate array length
    pub fn array_length<T>(array: &[T], min: usize, max: usize) -> ValidationResult {
        if array.len() < min {
            ValidationResult::Invalid(format!("array length must be at least {} items", min))
        } else if array.len() > max {
            ValidationResult::Invalid(format!("array length must be at most {} items", max))
        } else {
            ValidationResult::Valid
        }
    }
}

/// Validation helpers for common scenarios
pub struct ValidationHelpers;

impl ValidationHelpers {
    /// Validate a complete request object
    pub fn validate_request<T: serde::Serialize + std::fmt::Debug>(
        request: &T,
        rules: HashMap<String, Vec<String>>,
    ) -> ValidationContext {
        let mut context = ValidationContext::new();
        
        // Convert request to JSON for field access
        let json_value = match serde_json::to_value(request) {
            Ok(v) => v,
            Err(e) => {
                context.add_error(ValidationError::new(
                    "request".to_string(),
                    "serialization".to_string(),
                    format!("failed to serialize request: {}", e),
                ));
                return context;
            }
        };
        
        // Apply validation rules
        for (field, field_rules) in rules {
            if let Some(field_value) = json_value.get(&field) {
                for rule in field_rules {
                    let result = Self::apply_rule(field_value, &rule);
                    if let ValidationResult::Invalid(msg) | ValidationResult::Error(msg) = result {
                        context.add_error(ValidationError::new(
                            field.clone(),
                            rule.clone(),
                            msg,
                        ));
                    }
                }
            } else {
                // Field doesn't exist in request
                context.add_error(ValidationError::new(
                    field.clone(),
                    "required".to_string(),
                    "field is required but missing".to_string(),
                ));
            }
        }
        
        context
    }

    /// Apply a single validation rule
    fn apply_rule(field_value: &serde_json::Value, rule: &str) -> ValidationResult {
        match rule {
            "required" => {
                if field_value.is_null() || field_value.is_object() && field_value.as_object().unwrap().is_empty() {
                    ValidationResult::Invalid("field is required".to_string())
                } else {
                    ValidationResult::Valid
                }
            }
            "email" => {
                if let Some(s) = field_value.as_str() {
                    StringValidator::email(s)
                } else {
                    ValidationResult::Invalid("field must be a string".to_string())
                }
            }
            "url" => {
                if let Some(s) = field_value.as_str() {
                    StringValidator::url(s)
                } else {
                    ValidationResult::Invalid("field must be a string".to_string())
                }
            }
            "phone" => {
                if let Some(s) = field_value.as_str() {
                    StringValidator::phone(s)
                } else {
                    ValidationResult::Invalid("field must be a string".to_string())
                }
            }
            _ => ValidationResult::Invalid(format!("unknown validation rule: {}", rule)),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    
    #[test]
    fn test_string_length_validation() {
        assert_eq!(StringValidator::length("hello", 3, 10), ValidationResult::Valid);
        assert_eq!(StringValidator::length("hi", 3, 10), ValidationResult::Invalid("string length must be at least 3 characters".to_string()));
        assert_eq!(StringValidator::length("hello world!", 3, 10), ValidationResult::Invalid("string length must be at most 10 characters".to_string()));
    }
    
    #[test]
    fn test_string_required_validation() {
        assert_eq!(StringValidator::required("hello"), ValidationResult::Valid);
        assert_eq!(StringValidator::required(""), ValidationResult::Invalid("field is required".to_string()));
        assert_eq!(StringValidator::required("   "), ValidationResult::Invalid("field is required".to_string()));
    }
    
    #[test]
    fn test_email_validation() {
        assert_eq!(StringValidator::email("test@example.com"), ValidationResult::Valid);
        assert_eq!(StringValidator::email("invalid-email"), ValidationResult::Invalid("string does not match pattern '^[a-zA-Z0-9._%+-]+@[a-zA-Z0-9.-]+\\.[a-zA-Z]{2,}$'".to_string()));
    }
    
    #[test]
    fn test_number_range_validation() {
        assert_eq!(NumberValidator::range(5, 1, 10), ValidationResult::Valid);
        assert_eq!(NumberValidator::range(0, 1, 10), ValidationResult::Invalid("value must be at least 1".to_string()));
        assert_eq!(NumberValidator::range(15, 1, 10), ValidationResult::Invalid("value must be at most 10".to_string()));
    }
    
    #[test]
    fn test_datetime_validation() {
        // Test valid datetime
        assert_eq!(DateTimeValidator::past("2020-01-01T00:00:00Z"), ValidationResult::Valid);
        
        // Test future datetime
        assert_eq!(DateTimeValidator::future("3000-01-01T00:00:00Z"), ValidationResult::Valid);
        
        // Test invalid format
        assert!(matches!(DateTimeValidator::past("invalid-date"), ValidationResult::Error(_)));
    }
    
    #[test]
    fn test_validation_context() {
        let mut context = ValidationContext::new();
        assert!(context.is_valid());
        
        context.add_error(ValidationError::new("email".to_string(), "email".to_string(), "invalid email".to_string()));
        assert!(!context.is_valid());
        assert_eq!(context.errors.len(), 1);
    }
}