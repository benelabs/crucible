//! Integration test suite entry point.
//!
//! Cargo treats every `.rs` file directly under `tests/` as a separate test
//! crate. This file pulls in the `integration/` sub-modules so they all share
//! the same compiled crate (faster builds, shared helpers).

mod common;
mod integration;

