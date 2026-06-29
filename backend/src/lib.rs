//! Crucible backend library crate.

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
