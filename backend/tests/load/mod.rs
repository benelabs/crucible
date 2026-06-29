//! Load and stress tests for the backend API.
//!
//! These tests exercise the API under concurrent load to verify that the
//! server remains stable and responsive. They are designed to run without
//! external services (PostgreSQL, Redis) by using in-process Axum routers
//! with mock state.
//!
//! # Running
//!
//! ```bash
//! # All load tests
//! cargo test -p backend --test load_tests -- --nocapture
//!
//! # A specific module
//! cargo test -p backend --test load_tests load::status_load -- --nocapture
//! cargo test -p backend --test load_tests load::profile_load -- --nocapture
//! cargo test -p backend --test load_tests load::dashboard_load -- --nocapture
//! cargo test -p backend --test load_tests load::stellar_load -- --nocapture
//! cargo test -p backend --test load_tests load::framework -- --nocapture
//! ```
//!
//! # Architecture
//!
//! Each sub-module builds an in-process Axum [`Router`] with a lightweight
//! mock [`AppState`] (no real DB or Redis connections). Requests are fired
//! via [`tower::ServiceExt::oneshot`], which bypasses the network entirely
//! and exercises only the handler + middleware stack.
//!
//! The [`framework`] module provides shared helpers:
//! - [`LoadConfig`] — concurrency / iteration parameters
//! - [`LoadResult`] — aggregated latency statistics
//! - [`run_load`] — generic concurrent request runner
//! - [`assert_load_result`] — assertion helper for p99 / error-rate targets

pub mod dashboard_load;
pub mod framework;
pub mod profile_load;
pub mod status_load;
pub mod stellar_load;
