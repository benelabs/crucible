#![allow(dead_code)]

use chrono::Utc;
use serde::{Deserialize, Serialize};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::time::Duration;
use tokio::sync::RwLock;
use tokio::time::{interval, Interval, MissedTickBehavior};
use uuid::Uuid;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MonitorRule {
    pub id: String,
    pub contract_address: String,
    pub rule_type: String,
    pub threshold: f64,
    pub enabled: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MonitorAlert {
    pub id: String,
    pub rule_id: String,
    pub contract_address: String,
    pub message: String,
    pub severity: String,
    pub triggered_at: chrono::DateTime<Utc>,
    pub resolved: bool,
}

#[derive(Clone)]
pub struct ContractMonitor {
    rules: Arc<RwLock<Vec<MonitorRule>>>,
    alerts: Arc<RwLock<Vec<MonitorAlert>>>,
}

impl ContractMonitor {
    pub fn new() -> Self {
        Self {
            rules: Arc::new(RwLock::new(vec![])),
            alerts: Arc::new(RwLock::new(vec![])),
        }
    }

    /// Creates a Tokio interval configured with MissedTickBehavior::Skip
    /// to avoid cumulative scheduling drift under event loop pressure (#1219).
    pub fn create_interval(period: Duration) -> Interval {
        let mut ticker = interval(period);
        ticker.set_missed_tick_behavior(MissedTickBehavior::Skip);
        ticker
    }

    pub async fn add_rule(
        &self,
        contract_address: &str,
        rule_type: &str,
        threshold: f64,
    ) -> MonitorRule {
        let rule = MonitorRule {
            id: Uuid::new_v4().to_string(),
            contract_address: contract_address.to_string(),
            rule_type: rule_type.to_string(),
            threshold,
            enabled: true,
        };
        self.rules.write().await.push(rule.clone());
        rule
    }

    pub async fn remove_rule(&self, id: &str) -> bool {
        let mut rules = self.rules.write().await;
        let before = rules.len();
        rules.retain(|r| r.id != id);
        rules.len() < before
    }

    pub async fn list_rules(&self, contract_address: Option<&str>) -> Vec<MonitorRule> {
        let rules = self.rules.read().await;
        match contract_address {
            Some(addr) => rules
                .iter()
                .filter(|r| r.contract_address == addr)
                .cloned()
                .collect(),
            None => rules.clone(),
        }
    }

    pub async fn check_and_alert(
        &self,
        contract_address: &str,
        metric_value: f64,
    ) -> Vec<MonitorAlert> {
        let rules = self.rules.read().await;
        let mut new_alerts = Vec::new();

        for rule in rules
            .iter()
            .filter(|r| r.enabled && r.contract_address == contract_address)
        {
            if metric_value > rule.threshold {
                let severity = if metric_value > rule.threshold * 2.0 {
                    "critical"
                } else if metric_value > rule.threshold * 1.5 {
                    "warning"
                } else {
                    "info"
                };
                let alert = MonitorAlert {
                    id: Uuid::new_v4().to_string(),
                    rule_id: rule.id.clone(),
                    contract_address: contract_address.to_string(),
                    message: format!(
                        "Rule '{}' triggered: value {:.2} exceeded threshold {:.2}",
                        rule.rule_type, metric_value, rule.threshold
                    ),
                    severity: severity.to_string(),
                    triggered_at: Utc::now(),
                    resolved: false,
                };
                new_alerts.push(alert);
            }
        }
        drop(rules);

        let mut alerts = self.alerts.write().await;
        alerts.extend(new_alerts.clone());
        new_alerts
    }

    pub async fn list_alerts(
        &self,
        contract_address: Option<&str>,
        resolved: Option<bool>,
    ) -> Vec<MonitorAlert> {
        let alerts = self.alerts.read().await;
        alerts
            .iter()
            .filter(|a| {
                contract_address.is_none_or(|addr| a.contract_address == addr)
                    && resolved.is_none_or(|r| a.resolved == r)
            })
            .cloned()
            .collect()
    }

    pub async fn resolve_alert(&self, id: &str) -> bool {
        let mut alerts = self.alerts.write().await;
        if let Some(alert) = alerts.iter_mut().find(|a| a.id == id) {
            alert.resolved = true;
            return true;
        }
        false
    }

    /// Polls contract metrics at a steady cadence using `tokio::time::interval`
    /// configured with `MissedTickBehavior::Skip` to eliminate scheduling drift (#1219).
    pub async fn poll_contract_metrics<F, Fut>(
        &self,
        contract_address: &str,
        period: Duration,
        iterations: usize,
        mut fetch_metric: F,
    ) -> Vec<MonitorAlert>
    where
        F: FnMut() -> Fut,
        Fut: std::future::Future<Output = f64>,
    {
        let mut ticker = Self::create_interval(period);
        let mut all_alerts = Vec::new();

        for _ in 0..iterations {
            ticker.tick().await;
            let val = fetch_metric().await;
            let alerts = self.check_and_alert(contract_address, val).await;
            all_alerts.extend(alerts);
        }

        all_alerts
    }

    /// Runs a steady-cadence polling loop with a cancellation flag, avoiding drift (#1219).
    pub async fn poll_until_stopped<F, Fut>(
        &self,
        period: Duration,
        running: Arc<AtomicBool>,
        mut check_fn: F,
    ) where
        F: FnMut() -> Fut,
        Fut: std::future::Future<Output = ()>,
    {
        let mut ticker = Self::create_interval(period);
        while running.load(Ordering::Relaxed) {
            ticker.tick().await;
            check_fn().await;
        }
    }
}

impl Default for ContractMonitor {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn test_rules_crud() {
        let monitor = ContractMonitor::new();
        let rule = monitor.add_rule("0x123", "high_gas", 5000.0).await;
        assert_eq!(rule.contract_address, "0x123");
        assert_eq!(rule.rule_type, "high_gas");
        assert_eq!(rule.threshold, 5000.0);
        assert!(rule.enabled);

        let rules = monitor.list_rules(Some("0x123")).await;
        assert_eq!(rules.len(), 1);

        let empty_rules = monitor.list_rules(Some("0xOTHER")).await;
        assert_eq!(empty_rules.len(), 0);

        let removed = monitor.remove_rule(&rule.id).await;
        assert!(removed);
        assert_eq!(monitor.list_rules(None).await.len(), 0);
    }

    #[tokio::test]
    async fn test_check_and_alert_severities() {
        let monitor = ContractMonitor::new();
        monitor.add_rule("0xABC", "latency", 100.0).await;

        // Below threshold: no alert
        let alerts = monitor.check_and_alert("0xABC", 50.0).await;
        assert!(alerts.is_empty());

        // Above 1.0x: info
        let alerts = monitor.check_and_alert("0xABC", 110.0).await;
        assert_eq!(alerts.len(), 1);
        assert_eq!(alerts[0].severity, "info");

        // Above 1.5x: warning
        let alerts = monitor.check_and_alert("0xABC", 160.0).await;
        assert_eq!(alerts.len(), 1);
        assert_eq!(alerts[0].severity, "warning");

        // Above 2.0x: critical
        let alerts = monitor.check_and_alert("0xABC", 250.0).await;
        assert_eq!(alerts.len(), 1);
        assert_eq!(alerts[0].severity, "critical");

        let all_alerts = monitor.list_alerts(Some("0xABC"), Some(false)).await;
        assert_eq!(all_alerts.len(), 3);

        let resolved = monitor.resolve_alert(&alerts[0].id).await;
        assert!(resolved);

        let unresolved = monitor.list_alerts(Some("0xABC"), Some(false)).await;
        assert_eq!(unresolved.len(), 2);
    }

    #[tokio::test]
    async fn test_poll_contract_metrics_cadence() {
        let monitor = ContractMonitor::new();
        monitor.add_rule("0xCADENCE", "error_rate", 0.05).await;

        let mut count = 0;
        let alerts = monitor
            .poll_contract_metrics("0xCADENCE", Duration::from_millis(5), 3, || {
                count += 1;
                async move {
                    if count == 2 {
                        0.10 // Triggers warning alert
                    } else {
                        0.01
                    }
                }
            })
            .await;

        assert_eq!(alerts.len(), 1);
        assert_eq!(alerts[0].severity, "warning");
    }

    #[tokio::test]
    async fn test_poll_until_stopped_cancellation() {
        let monitor = ContractMonitor::new();
        let running = Arc::new(AtomicBool::new(true));
        let count = Arc::new(std::sync::atomic::AtomicUsize::new(0));

        let running_clone = running.clone();
        let count_clone = count.clone();

        let handle = tokio::spawn(async move {
            monitor
                .poll_until_stopped(Duration::from_millis(5), running_clone, || {
                    let c = count_clone.clone();
                    async move {
                        c.fetch_add(1, Ordering::Relaxed);
                    }
                })
                .await;
        });

        tokio::time::sleep(Duration::from_millis(25)).await;
        running.store(false, Ordering::Relaxed);
        let _ = handle.await;

        assert!(count.load(Ordering::Relaxed) >= 2);
    }
}
