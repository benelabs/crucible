#![allow(deprecated)]
pub use soroban_sdk;
pub mod account;
pub mod cost;
pub mod env;
pub mod fixture;
pub mod macros;
pub mod prelude;
pub mod sim;
pub mod token;

/// The `#[fixture]` attribute macro for defining reusable test setup structs.
///
/// Re-exported from [`crucible_macros`] when the `derive` feature is enabled
/// (it is enabled by default).
///
/// See the [`crucible_macros`] crate documentation for full details and examples.
#[cfg(feature = "derive")]
pub use crucible_macros::fixture;
