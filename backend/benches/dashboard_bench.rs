use backend::api::handlers::dashboard::{ContractStats, DashboardMetrics};
use chrono::Utc;
use criterion::{criterion_group, criterion_main, Criterion};
use std::hint::black_box;

fn benchmark_dashboard_metrics_serialization(c: &mut Criterion) {
    let metrics = DashboardMetrics {
        total_contracts: 10000,
        total_transactions: 500000,
        avg_processing_time_ms: 125.5,
        failed_transactions_24h: 150,
        timestamp: Utc::now(),
    };

    c.bench_function("dashboard_metrics_serialization", |b| {
        b.iter(|| {
            let json = serde_json::to_string(black_box(&metrics)).unwrap();
            serde_json::from_str::<DashboardMetrics>(black_box(&json)).unwrap()
        })
    });
}

fn benchmark_contract_stats_serialization(c: &mut Criterion) {
    let stats = ContractStats {
        contract_id: "test_contract_with_long_id_12345".to_string(),
        invocation_count: 50000,
        last_invoked: Some(Utc::now()),
        avg_gas_cost: 2500.75,
    };

    c.bench_function("contract_stats_serialization", |b| {
        b.iter(|| {
            let json = serde_json::to_string(black_box(&stats)).unwrap();
            serde_json::from_str::<ContractStats>(black_box(&json)).unwrap()
        })
    });
}

criterion_group!(
    benches,
    benchmark_dashboard_metrics_serialization,
    benchmark_contract_stats_serialization
);
criterion_main!(benches);
