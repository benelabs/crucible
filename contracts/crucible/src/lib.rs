pub use soroban_sdk;
pub mod account;
pub mod cost;
pub mod env;
#[cfg(test)]
mod env_event_filter_tests;
mod event_topic_match;
pub mod fixture;
pub mod macros;
pub mod prelude;
pub mod reputation;
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
