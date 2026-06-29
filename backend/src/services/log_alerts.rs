//! Log alerting service for monitoring log entries and triggering alerts.
//!
//! This module provides threshold-based alerting on top of the log aggregation
//! pipeline. Alerts are evaluated against configurable rules and can be
//! dispatched to multiple channels (in-memory queue, Redis pub/sub).
//!
//! # Example
//! ```rust,no_run
//! use backend::services::log_alerts::{AlertManager, AlertRule, AlertSeverity};
//!
//! # async fn example() {
//! let manager = AlertManager::new();
//! manager.add_rule(AlertRule {
//!     id: uuid::Uuid::new_v4(),
//!     name: "High error rate".to_string(),
//!     pattern: "ERROR".to_string(),
//!     severity: AlertSeverity::Critical,
//!     threshold: 5,
//!     window_secs: 60,
//! }).await;
//! # }
//! ```

#![allow(dead_code)]

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::sync::Arc;
use thiserror::Error;
use tokio::sync::RwLock;
use tracing::{debug, error, info, warn};
use uuid::Uuid;

use crate::services::log_aggregator::LogEntry;

// ---------------------------------------------------------------------------
// Error types
// ---------------------------------------------------------------------------

/// Errors that can occur within the alert management system.
#[derive(Debug, Error)]
pub enum AlertError {
    /// A rule with the given ID was not found.
    #[error("Alert rule not found: {0}")]
    RuleNotFound(Uuid),

    /// An alert with the given ID was not found.
    #[error("Alert not found: {0}")]
    AlertNotFound(Uuid),

    /// The provided rule configuration is invalid.
    #[error("Invalid rule configuration: {0}")]
    InvalidRule(String),

    /// An internal error occurred.
    #[error("Internal alert error: {0}")]
    Internal(String),
}

// ---------------------------------------------------------------------------
// Domain types
// ---------------------------------------------------------------------------

/// Severity level of a triggered alert.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum AlertSeverity {
    /// Informational – no immediate action required.
    Info,
    /// Warning – should be investigated soon.
    Warning,
    /// Critical – requires immediate attention.
    Critical,
}

impl std::fmt::Display for AlertSeverity {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            AlertSeverity::Info => write!(f, "info"),
            AlertSeverity::Warning => write!(f, "warning"),
            AlertSeverity::Critical => write!(f, "critical"),
        }
    }
}

/// A rule that defines when an alert should fire.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AlertRule {
    /// Unique identifier for this rule.
    pub id: Uuid,
    /// Human-readable name.
    pub name: String,
    /// Substring pattern to match against log messages.
    pub pattern: String,
    /// Severity assigned to alerts fired by this rule.
    pub severity: AlertSeverity,
    /// Number of matching log entries within `window_secs` that triggers the alert.
    pub threshold: u32,
    /// Sliding window size in seconds.
    pub window_secs: u64,
}

impl AlertRule {
    /// Validate that the rule has sensible configuration values.
    pub fn validate(&self) -> Result<(), AlertError> {
        if self.name.trim().is_empty() {
            return Err(AlertError::InvalidRule("name must not be empty".to_string()));
        }
        if self.pattern.trim().is_empty() {
            return Err(AlertError::InvalidRule(
                "pattern must not be empty".to_string(),
            ));
        }
        if self.threshold == 0 {
            return Err(AlertError::InvalidRule("threshold must be > 0".to_string()));
        }
        if self.window_secs == 0 {
            return Err(AlertError::InvalidRule(
                "window_secs must be > 0".to_string(),
            ));
        }
        Ok(())
    }
}

/// A fired alert instance.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Alert {
    /// Unique identifier for this alert instance.
    pub id: Uuid,
    /// The rule that triggered this alert.
    pub rule_id: Uuid,
    /// Human-readable rule name (denormalised for convenience).
    pub rule_name: String,
    /// Severity of this alert.
    pub severity: AlertSeverity,
    /// Number of matching log entries that caused the alert.
    pub match_count: u32,
    /// When the alert was fired.
    pub fired_at: DateTime<Utc>,
    /// Whether the alert has been acknowledged.
    pub acknowledged: bool,
}

// ---------------------------------------------------------------------------
// Internal state
// ---------------------------------------------------------------------------

/// Tracks recent log-entry timestamps per rule for sliding-window evaluation.
#[derive(Debug, Default)]
struct RuleState {
    hits: Vec<DateTime<Utc>>,
}

impl RuleState {
    /// Prune entries older than `window_secs` and return the current hit count.
    fn prune_and_count(&mut self, window_secs: u64) -> u32 {
        let cutoff = Utc::now() - chrono::Duration::seconds(window_secs as i64);
        self.hits.retain(|ts| *ts > cutoff);
        self.hits.len() as u32
    }
}

// ---------------------------------------------------------------------------
// AlertManager
// ---------------------------------------------------------------------------

/// Manages alert rules, evaluates incoming log entries, and stores fired alerts.
pub struct AlertManager {
    rules: Arc<RwLock<HashMap<Uuid, AlertRule>>>,
    alerts: Arc<RwLock<Vec<Alert>>>,
    rule_states: Arc<RwLock<HashMap<Uuid, RuleState>>>,
}

impl AlertManager {
    /// Create a new `AlertManager` with no rules.
    pub fn new() -> Self {
        Self {
            rules: Arc::new(RwLock::new(HashMap::new())),
            alerts: Arc::new(RwLock::new(Vec::new())),
            rule_states: Arc::new(RwLock::new(HashMap::new())),
        }
    }

    /// Add or replace an alert rule.
    pub async fn add_rule(&self, rule: AlertRule) -> Result<(), AlertError> {
        rule.validate()?;
        let id = rule.id;
        info!(rule_id = %id, rule_name = %rule.name, "Adding alert rule");
        self.rules.write().await.insert(id, rule);
        self.rule_states.write().await.entry(id).or_default();
        Ok(())
    }

    /// Remove an alert rule by ID.
    pub async fn remove_rule(&self, rule_id: Uuid) -> Result<(), AlertError> {
        let removed = self.rules.write().await.remove(&rule_id).is_some();
        if !removed {
            return Err(AlertError::RuleNotFound(rule_id));
        }
        self.rule_states.write().await.remove(&rule_id);
        info!(rule_id = %rule_id, "Removed alert rule");
        Ok(())
    }

    /// Return all configured rules.
    pub async fn get_rules(&self) -> Vec<AlertRule> {
        self.rules.read().await.values().cloned().collect()
    }

    /// Evaluate a [`LogEntry`] against all active rules.
    pub async fn evaluate(&self, entry: &LogEntry) {
        let rules = self.rules.read().await;
        let mut states = self.rule_states.write().await;
        let mut fired: Vec<Alert> = Vec::new();

        for (id, rule) in rules.iter() {
            if !entry.message.contains(&rule.pattern) {
                continue;
            }

            debug!(
                rule_id = %id,
                rule_name = %rule.name,
                message = %entry.message,
                "Log entry matched rule pattern"
            );

            let state = states.entry(*id).or_default();
            state.hits.push(entry.timestamp);
            let count = state.prune_and_count(rule.window_secs);

            if count >= rule.threshold {
                warn!(
                    rule_id = %id,
                    rule_name = %rule.name,
                    count = count,
                    threshold = rule.threshold,
                    severity = %rule.severity,
                    "Alert threshold reached – firing alert"
                );
                fired.push(Alert {
                    id: Uuid::new_v4(),
                    rule_id: *id,
                    rule_name: rule.name.clone(),
                    severity: rule.severity.clone(),
                    match_count: count,
                    fired_at: Utc::now(),
                    acknowledged: false,
                });
                state.hits.clear();
            }
        }

        drop(rules);
        drop(states);

        if !fired.is_empty() {
            let mut alerts = self.alerts.write().await;
            for alert in fired {
                error!(
                    alert_id = %alert.id,
                    rule_name = %alert.rule_name,
                    severity = %alert.severity,
                    match_count = alert.match_count,
                    "Alert fired"
                );
                alerts.push(alert);
            }
        }
    }

    /// Return all fired alerts, optionally filtered by severity.
    pub async fn get_alerts(&self, severity: Option<AlertSeverity>) -> Vec<Alert> {
        let alerts = self.alerts.read().await;
        match severity {
            None => alerts.clone(),
            Some(s) => alerts.iter().filter(|a| a.severity == s).cloned().collect(),
        }
    }

    /// Acknowledge an alert by ID.
    pub async fn acknowledge_alert(&self, alert_id: Uuid) -> Result<(), AlertError> {
        let mut alerts = self.alerts.write().await;
        let alert = alerts
            .iter_mut()
            .find(|a| a.id == alert_id)
            .ok_or(AlertError::AlertNotFound(alert_id))?;
        alert.acknowledged = true;
        info!(alert_id = %alert_id, "Alert acknowledged");
        Ok(())
    }

    /// Return only unacknowledged alerts.
    pub async fn get_active_alerts(&self) -> Vec<Alert> {
        self.alerts
            .read()
            .await
            .iter()
            .filter(|a| !a.acknowledged)
            .cloned()
            .collect()
    }

    /// Clear all fired alerts (useful for testing / maintenance).
    pub async fn clear_alerts(&self) {
        self.alerts.write().await.clear();
        info!("Cleared all alerts");
    }
}

impl Default for AlertManager {
    fn default() -> Self {
        Self::new()
    }
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use crate::services::log_aggregator::LogEntry;

    fn make_entry(message: &str) -> LogEntry {
        LogEntry {
            timestamp: Utc::now(),
            level: "ERROR".to_string(),
            message: message.to_string(),
            service: "test".to_string(),
        }
    }

    fn make_rule(pattern: &str, threshold: u32, window_secs: u64) -> AlertRule {
        AlertRule {
            id: Uuid::new_v4(),
            name: format!("rule-{pattern}"),
            pattern: pattern.to_string(),
            severity: AlertSeverity::Warning,
            threshold,
            window_secs,
        }
    }

    #[test]
    fn test_rule_validation_empty_name() {
        let mut rule = make_rule("ERROR", 3, 60);
        rule.name = "  ".to_string();
        assert!(matches!(rule.validate(), Err(AlertError::InvalidRule(_))));
    }

    #[test]
    fn test_rule_validation_empty_pattern() {
        let mut rule = make_rule("ERROR", 3, 60);
        rule.pattern = String::new();
        assert!(matches!(rule.validate(), Err(AlertError::InvalidRule(_))));
    }

    #[test]
    fn test_rule_validation_zero_threshold() {
        let rule = make_rule("ERROR", 0, 60);
        assert!(matches!(rule.validate(), Err(AlertError::InvalidRule(_))));
    }

    #[test]
    fn test_rule_validation_zero_window() {
        let rule = make_rule("ERROR", 3, 0);
        assert!(matches!(rule.validate(), Err(AlertError::InvalidRule(_))));
    }

    #[test]
    fn test_rule_validation_valid() {
        let rule = make_rule("ERROR", 3, 60);
        assert!(rule.validate().is_ok());
    }

    #[tokio::test]
    async fn test_add_and_get_rules() {
        let manager = AlertManager::new();
        let rule = make_rule("ERROR", 3, 60);
        let id = rule.id;
        manager.add_rule(rule).await.unwrap();
        let rules = manager.get_rules().await;
        assert_eq!(rules.len(), 1);
        assert_eq!(rules[0].id, id);
    }

    #[tokio::test]
    async fn test_remove_rule() {
        let manager = AlertManager::new();
        let rule = make_rule("ERROR", 3, 60);
        let id = rule.id;
        manager.add_rule(rule).await.unwrap();
        manager.remove_rule(id).await.unwrap();
        assert!(manager.get_rules().await.is_empty());
    }

    #[tokio::test]
    async fn test_remove_nonexistent_rule_returns_error() {
        let manager = AlertManager::new();
        let result = manager.remove_rule(Uuid::new_v4()).await;
        assert!(matches!(result, Err(AlertError::RuleNotFound(_))));
    }

    #[tokio::test]
    async fn test_no_alert_below_threshold() {
        let manager = AlertManager::new();
        manager.add_rule(make_rule("ERROR", 3, 60)).await.unwrap();
        manager.evaluate(&make_entry("ERROR occurred")).await;
        manager.evaluate(&make_entry("ERROR occurred")).await;
        assert!(manager.get_alerts(None).await.is_empty());
    }

    #[tokio::test]
    async fn test_alert_fires_at_threshold() {
        let manager = AlertManager::new();
        manager.add_rule(make_rule("ERROR", 3, 60)).await.unwrap();
        for _ in 0..3 {
            manager.evaluate(&make_entry("ERROR occurred")).await;
        }
        let alerts = manager.get_alerts(None).await;
        assert_eq!(alerts.len(), 1);
        assert_eq!(alerts[0].match_count, 3);
    }

    #[tokio::test]
    async fn test_non_matching_entry_does_not_fire() {
        let manager = AlertManager::new();
        manager.add_rule(make_rule("ERROR", 1, 60)).await.unwrap();
        manager.evaluate(&make_entry("INFO everything is fine")).await;
        assert!(manager.get_alerts(None).await.is_empty());
    }

    #[tokio::test]
    async fn test_alert_resets_after_firing() {
        let manager = AlertManager::new();
        manager.add_rule(make_rule("ERROR", 2, 60)).await.unwrap();
        manager.evaluate(&make_entry("ERROR a")).await;
        manager.evaluate(&make_entry("ERROR b")).await;
        assert_eq!(manager.get_alerts(None).await.len(), 1);
        manager.evaluate(&make_entry("ERROR c")).await;
        manager.evaluate(&make_entry("ERROR d")).await;
        assert_eq!(manager.get_alerts(None).await.len(), 2);
    }

    #[tokio::test]
    async fn test_acknowledge_alert() {
        let manager = AlertManager::new();
        manager.add_rule(make_rule("CRIT", 1, 60)).await.unwrap();
        manager.evaluate(&make_entry("CRIT failure")).await;
        let alerts = manager.get_alerts(None).await;
        assert_eq!(alerts.len(), 1);
        let alert_id = alerts[0].id;
        manager.acknowledge_alert(alert_id).await.unwrap();
        let active = manager.get_active_alerts().await;
        assert!(active.is_empty());
    }

    #[tokio::test]
    async fn test_acknowledge_nonexistent_alert_returns_error() {
        let manager = AlertManager::new();
        let result = manager.acknowledge_alert(Uuid::new_v4()).await;
        assert!(matches!(result, Err(AlertError::AlertNotFound(_))));
    }

    #[tokio::test]
    async fn test_filter_alerts_by_severity() {
        let manager = AlertManager::new();
        let mut warn_rule = make_rule("WARN", 1, 60);
        warn_rule.severity = AlertSeverity::Warning;
        manager.add_rule(warn_rule).await.unwrap();
        let mut crit_rule = make_rule("CRIT", 1, 60);
        crit_rule.severity = AlertSeverity::Critical;
        manager.add_rule(crit_rule).await.unwrap();
        manager.evaluate(&make_entry("WARN something")).await;
        manager.evaluate(&make_entry("CRIT something")).await;
        let critical = manager.get_alerts(Some(AlertSeverity::Critical)).await;
        assert_eq!(critical.len(), 1);
        assert_eq!(critical[0].severity, AlertSeverity::Critical);
    }

    #[tokio::test]
    async fn test_clear_alerts() {
        let manager = AlertManager::new();
        manager.add_rule(make_rule("ERR", 1, 60)).await.unwrap();
        manager.evaluate(&make_entry("ERR boom")).await;
        assert!(!manager.get_alerts(None).await.is_empty());
        manager.clear_alerts().await;
        assert!(manager.get_alerts(None).await.is_empty());
    }
}
