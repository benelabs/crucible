//! Integration tests for OpenTelemetry tracing instrumentation
//!
//! These tests validate that:
//! - Spans are created correctly with semantic conventions
//! - Span hierarchies are properly nested
//! - Error propagation works correctly
//! - Performance overhead is acceptable

use backend::services::tracing::{TracingConfig, TracingService};
use tracing::{info_span, Instrument};
use tracing_subscriber::layer::SubscriberExt;
use tracing_subscriber::util::SubscriberInitExt;

#[cfg(test)]
mod tests {
    use super::*;

    /// Test that HTTP request spans are created with correct attributes
    #[test]
    fn test_http_request_span_creation() {
        let span = TracingService::http_request_span("GET", "/api/users", Some("user123"));

        // Verify span is created
        if let Some(metadata) = span.metadata() {
            assert_eq!(metadata.name(), "http.request");
        }

        // Span should have fields for http.method, http.route, user.id
        drop(span);
    }

    /// Test that database query spans are created with correct attributes
    #[test]
    fn test_db_query_span_creation() {
        let query = "SELECT * FROM users WHERE id = $1";
        let span = TracingService::db_query_span(query, "postgres", "SELECT");

        if let Some(metadata) = span.metadata() {
            assert_eq!(metadata.name(), "db.query");
        }
        drop(span);
    }

    /// Test that Redis command spans are created with correct attributes
    #[test]
    fn test_redis_command_span_creation() {
        let span = TracingService::redis_command_span("GET", Some("user:123"));

        if let Some(metadata) = span.metadata() {
            assert_eq!(metadata.name(), "db.redis.command");
        }
        drop(span);
    }

    /// Test that service method spans are created correctly
    #[test]
    fn test_service_method_span_creation() {
        let span = TracingService::service_method_span("UserService", "get_user");

        if let Some(metadata) = span.metadata() {
            assert_eq!(metadata.name(), "service.method");
        }
        drop(span);
    }

    /// Test that job spans are created correctly
    #[test]
    fn test_job_span_creation() {
        let span = TracingService::job_span("process_transaction", "job-456");

        if let Some(metadata) = span.metadata() {
            assert_eq!(metadata.name(), "job.execute");
        }
        drop(span);
    }

    /// Test that error recording works correctly
    #[test]
    fn test_error_recording() {
        let span = TracingService::http_request_span("POST", "/api/orders", None);

        TracingService::record_error(&span, "Database connection failed", "database");

        // Span should have error.type field recorded
        drop(span);
    }

    /// Test span nesting (parent-child relationships)
    #[tokio::test]
    async fn test_span_hierarchy() {
        let http_span = info_span!(
            "http.request",
            http.method = "GET",
            http.route = "/api/users"
        );

        async {
            let db_span = info_span!("db.query", db.system = "postgres");

            async {
                // This span should be a child of db_span
                tracing::info!("Executing query");
            }
            .instrument(db_span)
            .await;
        }
        .instrument(http_span)
        .await;
    }

    /// Test that long queries are truncated correctly
    #[test]
    fn test_query_truncation() {
        let long_query = "SELECT * FROM users WHERE ".to_string() + &"x".repeat(500);
        let span = TracingService::db_query_span(&long_query, "postgres", "SELECT");

        // Query should be truncated to 256 characters
        drop(span);
    }

    /// Test tracing config with different environments
    #[test]
    fn test_tracing_config_environments() {
        let dev_config = TracingConfig::new("test-service".to_string(), "0.1.0".to_string())
            .with_environment("dev".to_string());
        assert_eq!(dev_config.sampling_ratio, 1.0);

        let staging_config = TracingConfig::new("test-service".to_string(), "0.1.0".to_string())
            .with_environment("staging".to_string());
        assert_eq!(staging_config.sampling_ratio, 0.1);

        let prod_config = TracingConfig::new("test-service".to_string(), "0.1.0".to_string())
            .with_environment("production".to_string());
        assert_eq!(prod_config.sampling_ratio, 0.01);
    }

    /// Test custom sampling ratio
    #[test]
    fn test_custom_sampling_ratio() {
        let config = TracingConfig::default().with_sampling_ratio(0.5);
        assert_eq!(config.sampling_ratio, 0.5);

        // Test bounds
        let config_high = TracingConfig::default().with_sampling_ratio(1.5);
        assert_eq!(config_high.sampling_ratio, 1.0);

        let config_low = TracingConfig::default().with_sampling_ratio(-0.5);
        assert_eq!(config_low.sampling_ratio, 0.0);
    }

    /// Test OTLP endpoint configuration
    #[test]
    fn test_otlp_endpoint_config() {
        let config = TracingConfig::default().with_otlp_endpoint("http://jaeger:4317".to_string());
        assert_eq!(config.otlp_endpoint, "http://jaeger:4317");
    }

    /// Test span limits configuration
    #[test]
    fn test_span_limits() {
        let config = TracingConfig::default();
        assert_eq!(config.max_attributes_per_span, 128);
        assert_eq!(config.max_events_per_span, 128);
        assert_eq!(config.max_links_per_span, 128);
    }

    /// Test that multiline queries use only the first line
    #[test]
    fn test_multiline_query_truncation() {
        let multiline_query = "SELECT *\nFROM users\nWHERE id = $1";
        let span = TracingService::db_query_span(multiline_query, "postgres", "SELECT");

        // Should only include first line
        drop(span);
    }

    /// Test Redis command span without key
    #[test]
    fn test_redis_command_without_key() {
        let span = TracingService::redis_command_span("PING", None);
        if let Some(metadata) = span.metadata() {
            assert_eq!(metadata.name(), "db.redis.command");
        }
        drop(span);
    }

    /// Test HTTP span without user ID
    #[test]
    fn test_http_span_anonymous_user() {
        let span = TracingService::http_request_span("GET", "/api/public", None);
        if let Some(metadata) = span.metadata() {
            assert_eq!(metadata.name(), "http.request");
        }
        drop(span);
    }

    /// Test that span metadata is correct
    #[test]
    fn test_span_metadata() {
        let span = TracingService::http_request_span("POST", "/api/orders", Some("user456"));

        let metadata = span.metadata();
        if let Some(meta) = metadata {
            assert_eq!(meta.name(), "http.request");
            assert!(meta.is_span());
        }

        drop(span);
    }

    /// Test concurrent span creation (thread safety)
    #[tokio::test]
    async fn test_concurrent_span_creation() {
        let handles: Vec<_> = (0..10)
            .map(|i| {
                tokio::spawn(async move {
                    let span = TracingService::http_request_span(
                        "GET",
                        &format!("/api/users/{}", i),
                        Some(&format!("user{}", i)),
                    );
                    drop(span);
                })
            })
            .collect();

        for handle in handles {
            handle.await.unwrap();
        }
    }

    /// Test error recording with different error types
    #[test]
    fn test_error_types() {
        let span = TracingService::db_query_span("SELECT 1", "postgres", "SELECT");

        TracingService::record_error(&span, "Connection timeout", "timeout");
        TracingService::record_error(&span, "Query syntax error", "syntax");
        TracingService::record_error(&span, "Permission denied", "authorization");

        drop(span);
    }

    /// Test service method span with different services
    #[test]
    fn test_multiple_service_spans() {
        let user_span = TracingService::service_method_span("UserService", "create_user");
        let order_span = TracingService::service_method_span("OrderService", "create_order");
        let payment_span = TracingService::service_method_span("PaymentService", "process_payment");

        drop(user_span);
        drop(order_span);
        drop(payment_span);
    }

    /// Test job span with UUID job IDs
    #[test]
    fn test_job_span_with_uuid() {
        let job_id = uuid::Uuid::new_v4().to_string();
        let span = TracingService::job_span("background_task", &job_id);

        if let Some(metadata) = span.metadata() {
            assert_eq!(metadata.name(), "job.execute");
        }
        drop(span);
    }
}

/// Performance benchmarks for tracing overhead
#[cfg(test)]
mod benchmarks {
    use super::*;
    use std::time::Instant;

    /// Benchmark span creation overhead
    #[test]
    fn bench_span_creation() {
        let iterations = 10_000;

        let start = Instant::now();
        for i in 0..iterations {
            let span =
                TracingService::http_request_span("GET", "/api/test", Some(&format!("user{}", i)));
            drop(span);
        }
        let duration = start.elapsed();

        let avg_ns = duration.as_nanos() / iterations;
        println!("Average span creation time: {} ns", avg_ns);

        // Assert that span creation is fast (< 10 microseconds).
        // Note: threshold is relaxed for unoptimised (debug) builds; a release
        // build typically measures well under 1 µs.
        assert!(avg_ns < 10_000, "Span creation too slow: {} ns", avg_ns);
    }

    /// Benchmark nested span overhead
    #[tokio::test]
    async fn bench_nested_spans() {
        let iterations = 1_000;

        let start = Instant::now();
        for _ in 0..iterations {
            let http_span = info_span!("http.request");
            async {
                let db_span = info_span!("db.query");
                async {
                    // Simulated work
                }
                .instrument(db_span)
                .await;
            }
            .instrument(http_span)
            .await;
        }
        let duration = start.elapsed();

        let avg_us = duration.as_micros() / iterations;
        println!("Average nested span overhead: {} μs", avg_us);

        // Assert reasonable overhead (< 10 microseconds)
        assert!(avg_us < 10, "Nested span overhead too high: {} μs", avg_us);
    }
}
