#![cfg_attr(not(feature = "std"), no_std)]

pub use soroban_sdk;

#[cfg(feature = "std")]
pub mod account;
#[cfg(feature = "std")]
pub mod assertions;
#[cfg(feature = "std")]
pub mod auth_tree;
#[cfg(feature = "std")]
pub mod call_graph;
#[cfg(feature = "std")]
pub mod checkpoint;
#[cfg(feature = "std")]
pub mod cost;
#[cfg(feature = "std")]
pub mod env;
#[cfg(feature = "std")]
pub use crate::env::{MockEnv, MockEnvBuilder};
#[cfg(all(test, feature = "std"))]
mod assertions_tests;
#[cfg(all(test, feature = "std"))]
mod checkpoint_tests;
#[cfg(all(test, feature = "std"))]
mod env_crypto_tests;
#[cfg(all(test, feature = "std"))]
mod env_event_filter_tests;
#[cfg(all(test, feature = "std"))]
mod env_error_demangle_tests;
#[cfg(feature = "std")]
mod event_topic_match;
#[cfg(feature = "std")]
pub mod fixture;
pub mod fixture_graph;

/// Alias under which `#[fixture(requires = [..])]` expansions reach the
/// dependency-graph traits.
///
/// The attribute macro cannot know what a crate calls its `crucible` import, so
/// it emits paths rooted at `crucible_fixture_graph`. Bringing that name into
/// scope is what `use crucible::prelude::*;` does for you.
pub use crate::fixture_graph as crucible_fixture_graph;
#[cfg(feature = "std")]
pub mod macros;
pub mod prelude;
#[cfg(feature = "std")]
pub mod sim;
#[cfg(feature = "std")]
#[path = "time_helpers.rs"]
pub mod time;
#[cfg(feature = "std")]
pub use self::time as time_helpers;
#[cfg(feature = "std")]
pub mod token;
#[cfg(feature = "std")]
pub mod profiler;
#[cfg(feature = "std")]
pub mod quickcheck;
#[cfg(feature = "std")]
pub mod storage_size;
#[cfg(feature = "std")]
pub mod zk;

/// The `#[fixture]` attribute macro for defining reusable test setup structs.
///
/// Re-exported from [`crucible_macros`] when the `derive` feature is enabled
/// (it is enabled by default).
///
/// See the [`crucible_macros`] crate documentation for full details and examples.
#[cfg(all(feature = "derive", feature = "std"))]
pub use crucible_macros::fixture;

/// The `#[quickcheck]` attribute macro for property-based fuzz tests.
///
/// Re-exported from [`crucible_macros`] when the `derive` feature is enabled
/// (it is enabled by default). See [`crate::quickcheck`] for the runtime it
/// expands to, and the macro's own documentation for its arguments.
#[cfg(all(feature = "derive", feature = "std"))]
pub use crucible_macros::quickcheck;
