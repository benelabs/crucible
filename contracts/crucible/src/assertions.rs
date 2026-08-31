//! Fluent assertion builders for Soroban contract testing.
//!
//! This module complements the declarative [`assert_reverts!`](crate::assert_reverts)
//! macro with a *chainable* alternative built around [`RevertAssertion`].
//!
//! Where the macro must be written as a statement, a [`RevertAssertion`] is an
//! ordinary value: it can be stored in a `let` binding, returned from a helper,
//! or threaded through a fixture-based test suite before its expectations are
//! finally checked.
//!
//! # Example
//!
//! ```rust,ignore
//! use crucible::prelude::*;
//!
//! env.expect_revert(|| client.transfer(&alice.address(), &bob.address(), &200_i128))
//!     .with_error(ContractError::Unauthorized)
//!     .verify();
//! ```
//!
//! **Host-only:** [`RevertAssertion`] depends on [`std::panic::catch_unwind`]
//! and is intended exclusively for `#[cfg(test)]` use on the host.

/// The expectation a [`RevertAssertion`] checks about *why* a call reverted.
#[derive(Debug, Clone, PartialEq, Eq)]
enum ErrorExpectation {
    /// No expectation was declared yet.
    Unset,
    /// Any revert satisfies the assertion.
    Any,
    /// The revert must carry this contract error code.
    Code(u32),
    /// The captured panic message must contain this substring.
    Message(String),
}

/// A chainable assertion that a closure reverted (panicked).
///
/// Obtain one from [`MockEnv::expect_revert`](crate::env::MockEnv::expect_revert).
/// The closure is invoked **eagerly** when the builder is created, so the
/// revert has already happened by the time expectations are attached — this is
/// what makes it safe to inspect post-revert state inside
/// [`and_assert`](Self::and_assert).
///
/// The assertion is checked by [`verify`](Self::verify) or by
/// [`and_assert`](Self::and_assert). A `RevertAssertion` that is dropped
/// without either call panics, so a forgotten `.verify()` cannot silently
/// weaken a test.
///
/// # Example
///
/// ```rust,ignore
/// use crucible::prelude::*;
///
/// // Any revert.
/// env.expect_revert(|| client.claim()).with_any_error().verify();
///
/// // A specific `#[contracterror]` variant.
/// env.expect_revert(|| client.admin_only())
///     .with_error(ContractError::Unauthorized)
///     .verify();
///
/// // Assert on state after confirming the revert rolled everything back.
/// env.expect_revert(|| client.transfer(&alice.address(), &bob.address(), &999_i128))
///     .with_any_error()
///     .and_assert(|| {
///         assert_eq!(token.balance(&alice.address()), 500_i128);
///     });
/// ```
#[must_use = "a RevertAssertion does nothing until `.verify()` or `.and_assert(..)` is called"]
pub struct RevertAssertion {
    /// Whether the closure panicked.
    reverted: bool,
    /// The panic payload rendered as a string, when one could be extracted.
    message: Option<String>,
    /// The contract error code parsed out of the panic payload, if present.
    code: Option<u32>,
    /// What the caller expects about the revert.
    expectation: ErrorExpectation,
    /// Set once the assertion has been checked, so `Drop` can detect misuse.
    verified: bool,
}

impl std::fmt::Debug for RevertAssertion {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("RevertAssertion")
            .field("reverted", &self.reverted)
            .field("message", &self.message)
            .field("code", &self.code)
            .field("expectation", &self.expectation)
            .finish()
    }
}

impl RevertAssertion {
    /// Runs `f`, capturing whether it reverted and why.
    ///
    /// This is called by [`MockEnv::expect_revert`](crate::env::MockEnv::expect_revert);
    /// prefer that entry point over constructing the builder directly.
    pub(crate) fn capture<F, T>(f: F) -> Self
    where
        F: FnOnce() -> T,
    {
        // `AssertUnwindSafe` mirrors `assert_reverts!`: Soroban test clients hold
        // `&Env` internally and are never `UnwindSafe`, and a reverted host call
        // leaves no observable broken invariant in the mock environment.
        let outcome = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
            f();
        }));

        match outcome {
            Ok(()) => Self {
                reverted: false,
                message: None,
                code: None,
                expectation: ErrorExpectation::Unset,
                verified: false,
            },
            Err(payload) => {
                let message = panic_payload_message(payload.as_ref());
                let code = message.as_deref().and_then(parse_contract_error_code);
                Self {
                    reverted: true,
                    message,
                    code,
                    expectation: ErrorExpectation::Unset,
                    verified: false,
                }
            }
        }
    }

    /// Requires the revert to carry the given contract error.
    ///
    /// `error` is any `#[contracterror]` enum value (or, more generally, anything
    /// convertible into [`soroban_sdk::Error`]). Matching is by contract error
    /// code, which is what the Soroban host reports in the panic payload.
    ///
    /// # Example
    ///
    /// ```rust,ignore
    /// env.expect_revert(|| client.admin_only())
    ///     .with_error(ContractError::Unauthorized)
    ///     .verify();
    /// ```
    pub fn with_error<E>(mut self, error: E) -> Self
    where
        E: Into<soroban_sdk::Error>,
    {
        self.expectation = ErrorExpectation::Code(contract_error_code(error.into()));
        self
    }

    /// Requires the revert to carry the given raw contract error code.
    ///
    /// Use this when the error type is not in scope — for example when asserting
    /// against a contract compiled in another crate.
    pub fn with_error_code(mut self, code: u32) -> Self {
        self.expectation = ErrorExpectation::Code(code);
        self
    }

    /// Accepts any revert, regardless of the error it carried.
    pub fn with_any_error(mut self) -> Self {
        self.expectation = ErrorExpectation::Any;
        self
    }

    /// Requires the captured panic message to contain `substring`.
    ///
    /// Note that the Soroban host renders contract failures as
    /// `HostError: Error(Contract, #N)` rather than reproducing a contract's
    /// own `panic!` text, so this is most useful for host-level errors and for
    /// panics raised by test-side helper code.
    pub fn with_message_containing(mut self, substring: impl Into<String>) -> Self {
        self.expectation = ErrorExpectation::Message(substring.into());
        self
    }

    /// Returns `true` if the closure reverted.
    ///
    /// This is a plain query: it does not check the declared expectation, and
    /// does not mark the assertion as verified.
    pub fn reverted(&self) -> bool {
        self.reverted
    }

    /// Returns the contract error code carried by the revert, if one was found.
    pub fn error_code(&self) -> Option<u32> {
        self.code
    }

    /// Returns the captured panic message, if one could be extracted.
    pub fn message(&self) -> Option<&str> {
        self.message.as_deref()
    }

    /// Checks the assertion, panicking with a detailed report if it does not hold.
    ///
    /// Calling `verify` without first declaring an expectation is treated as
    /// [`with_any_error`](Self::with_any_error).
    ///
    /// # Panics
    ///
    /// Panics if the closure did not revert, or if it reverted for a different
    /// reason than the one declared.
    #[track_caller]
    pub fn verify(mut self) {
        self.check();
    }

    /// Checks the assertion, then runs `assertions` for post-revert checks.
    ///
    /// The revert is verified *first*, so `assertions` only runs once the call
    /// is known to have failed as expected — which is exactly when checking
    /// that state was left untouched is meaningful.
    ///
    /// # Panics
    ///
    /// Panics if the revert expectation does not hold. Any panic raised by
    /// `assertions` itself propagates normally.
    ///
    /// # Example
    ///
    /// ```rust,ignore
    /// env.expect_revert(|| client.withdraw(&alice.address(), &999_i128))
    ///     .with_any_error()
    ///     .and_assert(|| {
    ///         assert_eq!(token.balance(&alice.address()), 500_i128);
    ///     });
    /// ```
    #[track_caller]
    pub fn and_assert<F, T>(mut self, assertions: F) -> T
    where
        F: FnOnce() -> T,
    {
        self.check();
        assertions()
    }

    /// Shared body of [`verify`](Self::verify) and [`and_assert`](Self::and_assert).
    #[track_caller]
    fn check(&mut self) {
        // Set first so a failing assertion below reports the real problem rather
        // than being masked by the `Drop` guard firing during unwind.
        self.verified = true;

        if !self.reverted {
            panic!(
                "expect_revert failed: the call did not revert.\n\
                 \n\
                 Expected : {expected}",
                expected = describe(&self.expectation),
            );
        }

        match &self.expectation {
            ErrorExpectation::Unset | ErrorExpectation::Any => {}
            ErrorExpectation::Code(expected) => {
                if self.code != Some(*expected) {
                    panic!(
                        "expect_revert failed: the call reverted with a different error.\n\
                         \n\
                         Expected : contract error #{expected}\n\
                         Actual   : {actual}\n\
                         Panic    : {panic}",
                        expected = expected,
                        actual = match self.code {
                            Some(code) => std::format!("contract error #{code}"),
                            None => "no contract error code in the panic payload".to_string(),
                        },
                        panic = self.message.as_deref().unwrap_or("<unavailable>"),
                    );
                }
            }
            ErrorExpectation::Message(substring) => {
                let matched = self
                    .message
                    .as_deref()
                    .is_some_and(|m| m.contains(substring.as_str()));
                if !matched {
                    panic!(
                        "expect_revert failed: the panic message did not contain the expected text.\n\
                         \n\
                         Expected substring : {substring}\n\
                         Actual message     : {actual}",
                        substring = substring,
                        actual = self.message.as_deref().unwrap_or("<unavailable>"),
                    );
                }
            }
        }
    }
}

impl Drop for RevertAssertion {
    fn drop(&mut self) {
        // Never turn an in-flight panic into a double panic, which would abort.
        if self.verified || std::thread::panicking() {
            return;
        }
        panic!(
            "a RevertAssertion was dropped without being checked — \
             call `.verify()` or `.and_assert(..)` to assert the revert"
        );
    }
}

/// Renders an expectation for inclusion in a failure message.
fn describe(expectation: &ErrorExpectation) -> String {
    match expectation {
        ErrorExpectation::Unset | ErrorExpectation::Any => "a revert (any error)".to_string(),
        ErrorExpectation::Code(code) => std::format!("a revert with contract error #{code}"),
        ErrorExpectation::Message(substring) => {
            std::format!("a revert whose message contains {substring:?}")
        }
    }
}

/// Extracts a printable message from a `catch_unwind` payload.
fn panic_payload_message(payload: &(dyn std::any::Any + Send)) -> Option<String> {
    if let Some(s) = payload.downcast_ref::<&str>() {
        Some((*s).to_string())
    } else {
        payload.downcast_ref::<String>().cloned()
    }
}

/// Returns the contract error code of a [`soroban_sdk::Error`].
///
/// Non-contract errors (host errors, budget exhaustion, …) carry no contract
/// code. They are reported as [`u32::MAX`] so that they can never accidentally
/// compare equal to a real contract error code parsed from a panic payload.
fn contract_error_code(error: soroban_sdk::Error) -> u32 {
    use soroban_sdk::xdr::ScErrorType;
    if error.is_type(ScErrorType::Contract) {
        error.get_code()
    } else {
        u32::MAX
    }
}

/// Parses the contract error code out of a rendered host panic payload.
///
/// The Soroban host renders contract failures as
/// `HostError: Error(Contract, #7)`; this pulls the `7` out of that. Returns
/// `None` for any payload that is not a contract error.
fn parse_contract_error_code(message: &str) -> Option<u32> {
    let after = message.split("Error(Contract, #").nth(1)?;
    let digits: String = after.chars().take_while(char::is_ascii_digit).collect();
    digits.parse().ok()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_contract_error_code_from_host_payload() {
        assert_eq!(
            parse_contract_error_code("HostError: Error(Contract, #7)"),
            Some(7)
        );
        assert_eq!(
            parse_contract_error_code("HostError: Error(Contract, #4294967295)"),
            Some(u32::MAX)
        );
    }

    #[test]
    fn ignores_non_contract_error_payloads() {
        assert_eq!(
            parse_contract_error_code("HostError: Error(WasmVm, InvalidAction)"),
            None
        );
        assert_eq!(parse_contract_error_code("plain old panic"), None);
    }
}
