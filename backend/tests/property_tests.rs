use backend::{
    api::{
        handlers::{
            dashboard::{ContractStats, DashboardMetrics},
            profiling::MetricsReport,
        },
        middleware::rate_limit::{RateLimitConfig, RateLimitResult},
    },
    services::{
        audit::{AuditEventRecord, AuditEventRequest},
        compilation::CompilationResult,
        security_scanner::{SecurityFinding, SecurityReport},
    },
};
use chrono::{TimeZone, Utc};
use proptest::prelude::*;
use serde::{de::DeserializeOwned, Serialize};
use std::time::Duration;

/// Helper function to assert serde JSON roundtrip equivalence.
fn assert_json_roundtrip<T>(val: &T)
where
    T: Serialize + DeserializeOwned + std::fmt::Debug + PartialEq,
{
    let json = serde_json::to_string(val).expect("Serialization failed");
    let deserialized: T = serde_json::from_str(&json).expect("Deserialization failed");
    assert_eq!(val, &deserialized);
}

// ---------------------------------------------------------------------------
// Proptest Strategies
// ---------------------------------------------------------------------------

fn arb_rate_limit_config() -> impl Strategy<Value = RateLimitConfig> {
    (any::<u64>(), any::<u64>(), any::<u64>()).prop_map(|(capacity, refill_rate, ttl_secs)| RateLimitConfig {
        capacity,
        refill_rate_per_sec: refill_rate,
        ttl: Duration::from_secs(ttl_secs % 86400),
    })
}

fn arb_rate_limit_result() -> impl Strategy<Value = RateLimitResult> {
    (any::<bool>(), any::<u64>(), any::<u64>(), any::<u64>()).prop_map(|(allowed, limit, remaining, reset_secs)| {
        RateLimitResult {
            allowed,
            limit,
            remaining,
            reset_secs,
        }
    })
}

fn arb_compilation_result() -> impl Strategy<Value = CompilationResult> {
    (
        ".*",
        "[a-z_]+",
        ".*",
        "[0-9a-f]{64}",
        any::<usize>(),
        any::<i64>(),
    )
        .prop_map(|(build_id, status, logs, wasm_hash, wasm_size_bytes, compile_time_ms)| CompilationResult {
            build_id,
            status,
            logs,
            wasm_hash,
            wasm_size_bytes,
            compile_time_ms,
        })
}

fn arb_metrics_report() -> impl Strategy<Value = MetricsReport> {
    (
        any::<u64>(),
        any::<u64>(),
        any::<u32>(),
        (0..1000u64).prop_map(|n| n as f64 / 1000.0),
        any::<u32>(),
    )
        .prop_map(
            |(uptime_secs, memory_usage_bytes, active_requests, error_rate, ledger_ingestion_latency_ms)| {
                MetricsReport {
                    uptime_secs,
                    memory_usage_bytes,
                    active_requests,
                    error_rate,
                    ledger_ingestion_latency_ms,
                }
            },
        )
}

fn arb_security_finding() -> impl Strategy<Value = SecurityFinding> {
    (
        "[a-zA-Z0-9_-]+",
        "(LOW|MEDIUM|HIGH|CRITICAL)",
        ".*",
        ".*",
        proptest::option::of(any::<u32>()),
        ".*",
    )
        .prop_map(
            |(id, severity, title, description, line_number, recommendation)| SecurityFinding {
                id,
                severity,
                title,
                description,
                line_number,
                recommendation,
            },
        )
}

fn arb_security_report() -> impl Strategy<Value = SecurityReport> {
    (
        "[a-zA-Z0-9_-]+",
        proptest::collection::vec(arb_security_finding(), 0..5),
        (0..100u64).prop_map(|n| n as f64),
        0..1800000000i64,
        any::<bool>(),
    )
        .prop_map(|(contract_name, findings, risk_score, timestamp_sec, passed)| SecurityReport {
            contract_name,
            findings,
            risk_score,
            scanned_at: Utc.timestamp_opt(timestamp_sec, 0).unwrap(),
            passed,
        })
}

fn arb_dashboard_metrics() -> impl Strategy<Value = DashboardMetrics> {
    (
        any::<u64>(),
        any::<u64>(),
        (0..10000u64).prop_map(|n| n as f64 / 10.0),
        any::<u64>(),
        0..1800000000i64,
    )
        .prop_map(
            |(total_contracts, total_transactions, avg_processing_time_ms, failed_transactions_24h, ts)| {
                DashboardMetrics {
                    total_contracts,
                    total_transactions,
                    avg_processing_time_ms,
                    failed_transactions_24h,
                    timestamp: Utc.timestamp_opt(ts, 0).unwrap(),
                }
            },
        )
}

fn arb_contract_stats() -> impl Strategy<Value = ContractStats> {
    (
        "[a-zA-Z0-9_-]+",
        any::<u64>(),
        proptest::option::of((0..1800000000i64).prop_map(|ts| Utc.timestamp_opt(ts, 0).unwrap())),
        (0..10000u64).prop_map(|n| n as f64 / 10.0),
    )
        .prop_map(|(contract_id, invocation_count, last_invoked, avg_gas_cost)| ContractStats {
            contract_id,
            invocation_count,
            last_invoked,
            avg_gas_cost,
        })
}

fn arb_audit_event_request() -> impl Strategy<Value = AuditEventRequest> {
    (
        proptest::option::of("[a-zA-Z0-9_-]+".prop_map(String::from)),
        "[a-zA-Z_]+".prop_map(String::from),
        proptest::option::of("[a-zA-Z0-9_-]+".prop_map(String::from)),
        "[a-zA-Z0-9_]+".prop_map(|s| serde_json::json!({ "msg": s })),
    )
        .prop_map(|(aggregate_id, event_type, user_id, details)| AuditEventRequest {
            aggregate_id,
            event_type,
            user_id,
            details,
        })
}

fn arb_audit_event_record() -> impl Strategy<Value = AuditEventRecord> {
    (
        any::<i64>(),
        "[a-zA-Z_]+".prop_map(String::from),
        proptest::option::of("[a-zA-Z0-9_-]+".prop_map(String::from)),
        "[a-zA-Z0-9_]+".prop_map(|s| serde_json::json!({ "key": s })),
        0..1800000000i64,
        "[0-9a-f]{64}".prop_map(String::from),
        "[0-9a-f]{64}".prop_map(String::from),
    )
        .prop_map(
            |(id, event_type, user_id, details, ts, hash, previous_hash)| AuditEventRecord {
                id,
                event_type,
                user_id,
                details,
                timestamp: Utc.timestamp_opt(ts, 0).unwrap(),
                hash,
                previous_hash,
            },
        )
}

// ---------------------------------------------------------------------------
// Property Test Suites
// ---------------------------------------------------------------------------

proptest! {
    #![proptest_config(ProptestConfig::with_cases(100))]

    #[test]
    fn test_rate_limit_config_serde_roundtrip(config in arb_rate_limit_config()) {
        assert_json_roundtrip(&config);
    }

    #[test]
    fn test_rate_limit_result_serde_roundtrip(res in arb_rate_limit_result()) {
        assert_json_roundtrip(&res);
    }

    #[test]
    fn test_compilation_result_serde_roundtrip(res in arb_compilation_result()) {
        assert_json_roundtrip(&res);
    }

    #[test]
    fn test_metrics_report_serde_roundtrip(report in arb_metrics_report()) {
        let json = serde_json::to_string(&report).expect("Serialization failed");
        let deserialized: MetricsReport = serde_json::from_str(&json).expect("Deserialization failed");
        assert_eq!(report.uptime_secs, deserialized.uptime_secs);
        assert_eq!(report.memory_usage_bytes, deserialized.memory_usage_bytes);
        assert_eq!(report.active_requests, deserialized.active_requests);
        assert_eq!(report.ledger_ingestion_latency_ms, deserialized.ledger_ingestion_latency_ms);
        assert!((report.error_rate - deserialized.error_rate).abs() < 1e-6);
    }

    #[test]
    fn test_security_finding_serde_roundtrip(finding in arb_security_finding()) {
        let json = serde_json::to_string(&finding).expect("Serialization failed");
        let deserialized: SecurityFinding = serde_json::from_str(&json).expect("Deserialization failed");
        assert_eq!(finding.id, deserialized.id);
        assert_eq!(finding.severity, deserialized.severity);
        assert_eq!(finding.title, deserialized.title);
        assert_eq!(finding.description, deserialized.description);
        assert_eq!(finding.line_number, deserialized.line_number);
        assert_eq!(finding.recommendation, deserialized.recommendation);
    }

    #[test]
    fn test_security_report_serde_roundtrip(report in arb_security_report()) {
        let json = serde_json::to_string(&report).expect("Serialization failed");
        let deserialized: SecurityReport = serde_json::from_str(&json).expect("Deserialization failed");
        assert_eq!(report.contract_name, deserialized.contract_name);
        assert_eq!(report.findings.len(), deserialized.findings.len());
        assert_eq!(report.scanned_at, deserialized.scanned_at);
        assert_eq!(report.passed, deserialized.passed);
        assert!((report.risk_score - deserialized.risk_score).abs() < 1e-6);
    }

    #[test]
    fn test_dashboard_metrics_serde_roundtrip(metrics in arb_dashboard_metrics()) {
        let json = serde_json::to_string(&metrics).expect("Serialization failed");
        let deserialized: DashboardMetrics = serde_json::from_str(&json).expect("Deserialization failed");
        assert_eq!(metrics.total_contracts, deserialized.total_contracts);
        assert_eq!(metrics.total_transactions, deserialized.total_transactions);
        assert_eq!(metrics.failed_transactions_24h, deserialized.failed_transactions_24h);
        assert_eq!(metrics.timestamp, deserialized.timestamp);
        assert!((metrics.avg_processing_time_ms - deserialized.avg_processing_time_ms).abs() < 1e-6);
    }

    #[test]
    fn test_contract_stats_serde_roundtrip(stats in arb_contract_stats()) {
        let json = serde_json::to_string(&stats).expect("Serialization failed");
        let deserialized: ContractStats = serde_json::from_str(&json).expect("Deserialization failed");
        assert_eq!(stats.contract_id, deserialized.contract_id);
        assert_eq!(stats.invocation_count, deserialized.invocation_count);
        assert_eq!(stats.last_invoked, deserialized.last_invoked);
        assert!((stats.avg_gas_cost - deserialized.avg_gas_cost).abs() < 1e-6);
    }

    #[test]
    fn test_audit_event_request_serde_roundtrip(req in arb_audit_event_request()) {
        let json = serde_json::to_string(&req).expect("Serialization failed");
        let deserialized: AuditEventRequest = serde_json::from_str(&json).expect("Deserialization failed");
        assert_eq!(req.aggregate_id, deserialized.aggregate_id);
        assert_eq!(req.event_type, deserialized.event_type);
        assert_eq!(req.user_id, deserialized.user_id);
        assert_eq!(req.details, deserialized.details);
    }

    #[test]
    fn test_audit_event_record_serde_roundtrip(rec in arb_audit_event_record()) {
        let json = serde_json::to_string(&rec).expect("Serialization failed");
        let deserialized: AuditEventRecord = serde_json::from_str(&json).expect("Deserialization failed");
        assert_eq!(rec.id, deserialized.id);
        assert_eq!(rec.event_type, deserialized.event_type);
        assert_eq!(rec.user_id, deserialized.user_id);
        assert_eq!(rec.details, deserialized.details);
        assert_eq!(rec.timestamp, deserialized.timestamp);
        assert_eq!(rec.hash, deserialized.hash);
        assert_eq!(rec.previous_hash, deserialized.previous_hash);
    }
}
