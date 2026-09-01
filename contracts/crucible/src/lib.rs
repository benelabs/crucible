pub use soroban_sdk;
pub mod account;
pub mod assertions;
pub mod checkpoint;
pub mod cost;
pub mod env;
#[cfg(test)]
mod assertions_tests;
#[cfg(test)]
mod checkpoint_tests;
#[cfg(test)]
mod env_crypto_tests;
#[cfg(test)]
mod env_event_filter_tests;
mod event_topic_match;
pub mod fixture;
pub mod macros;
pub mod prelude;

pub use crate::env::Stroops;

pub mod sim;
#[path = "time_helpers.rs"]
pub mod time;
pub use self::time as time_helpers;
pub mod token;
pub mod profiler;
pub mod quickcheck;
pub mod storage_size;
pub mod zk;

/// The `#[fixture]` attribute macro for defining reusable test setup structs.
///
/// Re-exported from [`crucible_macros`] when the `derive` feature is enabled
/// (it is enabled by default).
///
/// See the [`crucible_macros`] crate documentation for full details and examples.
#[cfg(feature = "derive")]
pub use crucible_macros::fixture;

/// The `#[quickcheck]` attribute macro for property-based fuzz tests.
///
/// Re-exported from [`crucible_macros`] when the `derive` feature is enabled
/// (it is enabled by default). See [`crate::quickcheck`] for the runtime it
/// expands to, and the macro's own documentation for its arguments.
#[cfg(feature = "derive")]
pub use crucible_macros::quickcheck;
