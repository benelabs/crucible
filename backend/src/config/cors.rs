use serde::{Deserialize, Serialize};

/// CORS configuration controlling which origins may access the API.
#[derive(Clone, Debug, Deserialize, Serialize, PartialEq, Eq)]
pub struct CorsConfig {
    /// Allowed origins. Use `["*"]` for permissive development; production/staging must
    /// list explicit origins. Wildcard "*" is permitted ONLY when allow_credentials is false.
    #[serde(default = "default_allowed_origins")]
    pub allowed_origins: Vec<String>,

    /// Whether cross-origin requests are permitted to include credentials (cookies, auth headers).
    #[serde(default = "default_allow_credentials")]
    pub allow_credentials: bool,
}

fn default_allowed_origins() -> Vec<String> {
    vec!["*".to_string()]
}

fn default_allow_credentials() -> bool {
    false
}

impl Default for CorsConfig {
    fn default() -> Self {
        Self {
            allowed_origins: default_allowed_origins(),
            allow_credentials: default_allow_credentials(),
        }
    }
}

impl CorsConfig {
    pub fn new(allowed_origins: Vec<String>, allow_credentials: bool) -> Result<Self, String> {
        let config = Self {
            allowed_origins,
            allow_credentials,
        };
        config.validate()?;
        Ok(config)
    }

    /// Validates that CORS configuration does not combine wildcard origins with credentials.
    ///
    /// CRITICAL SECURITY INVARIANT:
    /// In accordance with W3C CORS and RFC 6454 specifications, combining `AllowOrigin::any()` / `"*"`
    /// with `allow_credentials(true)` is strictly prohibited across ALL environments (development,
    /// staging, and production) to prevent cross-origin credential theft.
    pub fn validate(&self) -> Result<(), String> {
        if self.allow_credentials && self.is_wildcard_origin() {
            return Err(
                "Overly permissive CORS configuration: AllowOrigin::any() / '*' is strictly prohibited \
                 in combination with allow_credentials(true) across all environments."
                    .to_string(),
            );
        }
        Ok(())
    }

    pub fn is_wildcard_origin(&self) -> bool {
        self.allowed_origins.iter().any(|o| o.trim() == "*")
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_cors_rejects_wildcard_with_credentials_in_all_environments() {
        let config = CorsConfig {
            allowed_origins: vec!["*".to_string()],
            allow_credentials: true,
        };
        assert!(config.validate().is_err());

        let res = CorsConfig::new(vec!["*".to_string()], true);
        assert!(res.is_err());
        assert!(res.unwrap_err().contains("strictly prohibited"));
    }

    #[test]
    fn test_cors_allows_wildcard_without_credentials() {
        let config = CorsConfig {
            allowed_origins: vec!["*".to_string()],
            allow_credentials: false,
        };
        assert!(config.validate().is_ok());
        assert!(config.is_wildcard_origin());
    }

    #[test]
    fn test_cors_allows_explicit_origins_with_credentials() {
        let config = CorsConfig {
            allowed_origins: vec![
                "https://app.crucible.io".to_string(),
                "https://staging.crucible.io".to_string(),
            ],
            allow_credentials: true,
        };
        assert!(config.validate().is_ok());
        assert!(!config.is_wildcard_origin());
    }

    #[test]
    fn test_default_cors_config_is_valid() {
        let default_config = CorsConfig::default();
        assert_eq!(default_config.allowed_origins, vec!["*".to_string()]);
        assert!(!default_config.allow_credentials);
        assert!(default_config.validate().is_ok());
    }
}
