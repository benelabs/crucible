//! Integration tests for backend services:
//! [`MetricsExporter`], [`ErrorManager`], and [`LogAggregator`].

use backend::services::{
    error_recovery::{ErrorManager, RecoveryError},
    log_aggregator::LogAggregator,
    sys_metrics::MetricsExporter,
};
use std::sync::Arc;

// ── MetricsExporter ──────────────────────────────────────────────────────────

#[tokio::test]
async fn metrics_exporter_defaults_to_zero() {
    let exporter = MetricsExporter::new();
    let m = exporter.get_metrics().await;
    assert_eq!(m.cpu_usage, 0.0);
    assert_eq!(m.memory_usage, 0);
    assert_eq!(m.uptime, 0);
}

#[tokio::test]
async fn metrics_exporter_update_reflects_in_get() {
    let exporter = MetricsExporter::new();
    exporter.update_metrics(42.5, 2048, 120, 1024, 512).await;
    let m = exporter.get_metrics().await;
    assert_eq!(m.cpu_usage, 42.5);
    assert_eq!(m.memory_usage, 2048);
    assert_eq!(m.uptime, 120);
}

#[tokio::test]
async fn metrics_exporter_run_collector_updates_metrics() {
    let exporter = Arc::new(MetricsExporter::new());
    let handle = tokio::spawn(MetricsExporter::run_collector(exporter.clone()));

    // Give the collector one tick (5 s interval is simulated; just wait briefly)
    tokio::time::sleep(tokio::time::Duration::from_millis(50)).await;
    handle.abort();

    // After at least one tick the metrics should have been written
    let m = exporter.get_metrics().await;
    // cpu_usage is set to 12.5 by the simulated collector
    assert_eq!(m.cpu_usage, 12.5);
}

// ── ErrorManager ─────────────────────────────────────────────────────────────

#[tokio::test]
async fn error_manager_registers_new_task_on_first_error() {
    let manager = ErrorManager::new();
    manager
        .handle_error(RecoveryError::Database("conn lost".into()), "task_a")
        .await
        .unwrap();

    let tasks = manager.get_active_tasks().await;
    assert_eq!(tasks.len(), 1);
    assert_eq!(tasks[0].name, "task_a");
    assert_eq!(tasks[0].retries, 1);
}

#[tokio::test]
async fn error_manager_increments_retries() {
    let manager = ErrorManager::new();
    for _ in 0..3 {
        manager
            .handle_error(RecoveryError::Internal("err".into()), "task_b")
            .await
            .unwrap();
    }
    let tasks = manager.get_active_tasks().await;
    assert_eq!(tasks[0].retries, 3);
}

#[tokio::test]
async fn error_manager_returns_err_after_max_retries() {
    let manager = ErrorManager::new();
    // Exhaust the 3 allowed retries
    for _ in 0..3 {
        manager
            .handle_error(RecoveryError::Redis("timeout".into()), "task_c")
            .await
            .unwrap();
    }
    let result = manager
        .handle_error(RecoveryError::Internal("final".into()), "task_c")
        .await;

    assert!(
        matches!(result, Err(RecoveryError::MaxRetriesReached(_))),
        "expected MaxRetriesReached, got {result:?}"
    );
}

#[tokio::test]
async fn error_manager_tracks_independent_tasks() {
    let manager = ErrorManager::new();
    manager
        .handle_error(RecoveryError::Database("x".into()), "task_x")
        .await
        .unwrap();
    manager
        .handle_error(RecoveryError::Redis("y".into()), "task_y")
        .await
        .unwrap();

    let tasks = manager.get_active_tasks().await;
    assert_eq!(tasks.len(), 2);
}

// ── LogAggregator ─────────────────────────────────────────────────────────────

#[tokio::test]
async fn log_aggregator_sends_entry_to_receiver() {
    let (aggregator, mut rx) = LogAggregator::new();
    aggregator
        .log("WARN", "disk almost full", "storage")
        .await
        .unwrap();

    let entry = rx.recv().await.expect("expected a log entry");
    assert_eq!(entry.level, "WARN");
    assert_eq!(entry.message, "disk almost full");
    assert_eq!(entry.service, "storage");
}

#[tokio::test]
async fn log_aggregator_preserves_order() {
    let (aggregator, mut rx) = LogAggregator::new();
    aggregator.log("INFO", "first", "svc").await.unwrap();
    aggregator.log("ERROR", "second", "svc").await.unwrap();

    assert_eq!(rx.recv().await.unwrap().message, "first");
    assert_eq!(rx.recv().await.unwrap().message, "second");
}

#[tokio::test]
async fn log_aggregator_run_worker_drains_channel() {
    let (aggregator, rx) = LogAggregator::new();

    aggregator.log("DEBUG", "msg1", "svc").await.unwrap();
    aggregator.log("DEBUG", "msg2", "svc").await.unwrap();

    // Drop the sender so the worker loop terminates naturally
    drop(aggregator);

    // run_worker should process both entries and return
    LogAggregator::run_worker(rx).await;
}
