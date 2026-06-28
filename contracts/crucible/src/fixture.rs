//! Reusable test fixture support via the [`#[fixture]`][macro@crate::fixture] attribute macro.
//!
//! A *fixture* is a plain Rust struct that bundles a fully configured [`MockEnv`] together
//! with all the helpers a test suite needs — tokens, accounts, contract clients — into a
//! single value that can be constructed in one call.
//!
//! Apply `#[fixture]` to such a struct and supply a `setup() -> Self` associated function.
//! The macro then:
//!
//! * Derives [`Debug`] automatically (unless already present).
//! * Generates a `reset(&mut self)` method so you can restore the fixture to its initial
//!   state at any point inside a test without creating a new binding.
//!
//! # Example
//!
//! ```rust,ignore
//! use crucible::prelude::*;
//! use crucible::fixture;
//!
//! #[fixture]
//! pub struct TokenFixture {
//!     pub env:   MockEnv,
//!     pub token: MockToken,
//! }
//!
//! impl TokenFixture {
//!     pub fn setup() -> Self {
//!         let env = MockEnv::builder().build();
//!         let token = MockToken::xlm(&env);
//!         Self { env, token }
//!     }
//! }
//!
//! #[test]
//! fn mint_increases_balance() {
//!     let f = TokenFixture::setup();
//!     let recipient = f.env.inner().register(soroban_sdk::testutils::MockAuthContract, ());
//!     f.token.mint(&recipient, 1_000);
//!     assert_eq!(f.token.balance(&recipient), 1_000);
//! }
//! ```
//!
//! [`MockEnv`]: crate::env::MockEnv
//!
//! **Host-only:** Fixtures depend on [`MockEnv`] and `std` and are intended
//! exclusively for use in `#[cfg(test)]` contexts on the host.
