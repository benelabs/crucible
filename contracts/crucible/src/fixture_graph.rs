//! Fixture dependency graph resolution.
//!
//! Complex test suites share multi-contract environments — a token, a DEX, an
//! oracle, a staking pool — and without a way to compose them, every test file
//! re-creates the same wiring by hand. The duplicated setup drifts apart, and a
//! change to one contract's construction has to be chased through each copy.
//!
//! `#[fixture(requires = [..])]` lets a fixture declare the fixtures it builds
//! on. The macro emits the traits in this module to describe that graph to the
//! compiler, generating a `setup_deps()` constructor that builds each dependency
//! in declaration order.
//!
//! # Cycle detection
//!
//! A cycle in the graph is rejected at compile time. A fixture is
//! [`AcyclicFixture`] only if every fixture it requires is, so a cycle becomes a
//! trait obligation that refers back to itself — which the compiler reports as
//! an overflow while evaluating the requirement, rather than accepting and
//! recursing forever at runtime. A fixture that names itself in `requires` is
//! caught earlier still, by the macro, with a message naming the fixture.
//!
//! # Example
//!
//! ```ignore
//! use crucible::prelude::*;
//!
//! #[fixture]
//! pub struct TokenFixture {
//!     pub env: MockEnv,
//! }
//!
//! impl TokenFixture {
//!     pub fn setup() -> Self {
//!         Self { env: MockEnv::default() }
//!     }
//! }
//!
//! #[fixture(requires = [TokenFixture])]
//! pub struct DexFixture {
//!     pub token: TokenFixture,
//! }
//!
//! impl DexFixture {
//!     pub fn setup() -> Self {
//!         let (token,) = Self::setup_deps();
//!         Self { token }
//!     }
//! }
//! ```

/// Describes a fixture's direct dependencies.
///
/// Implemented for every `#[fixture]` struct by the attribute macro; there is
/// no reason to implement it by hand.
pub trait FixtureDeps {
    /// The fixtures this one directly requires, as a tuple in declaration
    /// order. A fixture with no `requires` list has the unit type here.
    type Deps;

    /// Names of the required fixtures, in declaration order.
    ///
    /// Useful for diagnostics and for asserting a suite's graph in tests.
    const DEPENDENCY_NAMES: &'static [&'static str];

    /// Builds this fixture, having resolved its dependency graph.
    fn setup_checked() -> Self;
}

/// Marks a fixture whose dependency graph contains no cycles.
///
/// The macro implements this conditionally: a fixture is `AcyclicFixture` only
/// when all of its dependencies are. A cycle therefore has no base case, and
/// the compiler rejects the resulting obligation instead of accepting a graph
/// that could never be constructed.
pub trait AcyclicFixture: FixtureDeps {}
