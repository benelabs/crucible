//! Observability configuration.

use serde::{Deserialize, Serialize};
use std::str::FromStr;
use tracing::Level;

#[derive(Clone, Debug, Deserialize, Serialize)]
pub struct ObservabilityConfig {
    pub log_level: String,
    pub tracing_endpoint: Option<String>,
    pub enable_metrics: bool,
}

impl ObservabilityConfig {
    pub fn parsed_log_level(&self) -> Level {
        Level::from_str(&self.log_level).unwrap_or(Level::INFO)
    }

    pub fn init_tracing(&self, env: crate::config::Environment) {
        crate::utils::logger::init_tracing(&self.log_level, env);
    }

    pub fn json_logs(&self, env: crate::config::Environment) -> bool {
        matches!(
            env,
            crate::config::Environment::Staging | crate::config::Environment::Production
        )
    }
}
