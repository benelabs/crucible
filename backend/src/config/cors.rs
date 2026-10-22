use serde::{Deserialize, Serialize};

/// CORS configuration controlling which origins may access the API.
#[derive(Clone, Debug, Deserialize, Serialize)]
pub struct CorsConfig {
    /// Allowed origins. Use `["*"]` for permissive development; production must
    /// list explicit origins.
    #[serde(default = "default_allowed_origins")]
    pub allowed_origins: Vec<String>,
    /// Whether browsers may send credentials (cookies, auth headers) cross-origin.
    /// Never permitted together with the `"*"` wildcard origin, in any environment.
    #[serde(default)]
    pub allow_credentials: bool,
}

impl CorsConfig {
    /// Returns true when the wildcard origin is configured.
    pub fn allows_any_origin(&self) -> bool {
        self.allowed_origins.iter().any(|o| o == "*")
    }
}

fn default_allowed_origins() -> Vec<String> {
    vec!["*".to_string()]
}

impl Default for CorsConfig {
    fn default() -> Self {
        Self {
            allowed_origins: default_allowed_origins(),
            allow_credentials: false,
        }
    }
}
