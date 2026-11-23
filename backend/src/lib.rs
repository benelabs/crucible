//! Crucible backend library crate.

// Pre-existing query row tuples, service constructors and inherent parsers are
// intentionally kept stable; deny every other warning in backend CI.
#![allow(
    clippy::type_complexity,
    clippy::too_many_arguments,
    clippy::should_implement_trait
)]

pub mod api;
pub mod app_state;
pub mod config;
pub mod db;
pub mod error;
pub mod jobs;
pub mod router;
pub mod services;
pub mod telemetry;
pub mod utils;
pub mod workers;

#[cfg(any(test, feature = "testutils"))]
pub mod test_utils;

pub use error::AppError;
