//! Load and stress test suite entry point.
//!
//! This file is the integration test binary for all load tests. Each sub-module
//! exercises a specific API endpoint under concurrent load using the shared
//! [`load::framework`] helpers.
//!
//! # Running
//!
//! ```bash
//! # All load tests (with output)
//! cargo test -p backend --test load_tests -- --nocapture
//!
//! # A specific endpoint
//! cargo test -p backend --test load_tests load::status_load -- --nocapture
//! cargo test -p backend --test load_tests load::profile_load -- --nocapture
//! cargo test -p backend --test load_tests load::dashboard_load -- --nocapture
//! cargo test -p backend --test load_tests load::stellar_load -- --nocapture
//!
//! # Framework unit tests only
//! cargo test -p backend --test load_tests load::framework -- --nocapture
//! ```

mod load {
    pub mod dashboard_load;
    pub mod framework;
    pub mod profile_load;
    pub mod status_load;
    pub mod stellar_load;
}
