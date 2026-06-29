use serde::{Deserialize, Serialize};

/// CORS configuration controlling which origins may access the API.
#[derive(Clone, Debug, Deserialize, Serialize)]
pub struct CorsConfig {
    /// Allowed origins. Use `["*"]` for permissive development; production must
    /// list explicit origins.
    #[serde(default = "default_allowed_origins")]
    pub allowed_origins: Vec<String>,
}

fn default_allowed_origins() -> Vec<String> {
    vec!["*".to_string()]
}

impl Default for CorsConfig {
    fn default() -> Self {
        Self {
            allowed_origins: default_allowed_origins(),
        }
    }
}
