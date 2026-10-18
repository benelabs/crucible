//! Dynamic auth invocation tree verification.
//!
//! Soroban Protocol 21+ authorizes an invocation as a *tree*: an address signs
//! a root call, and that signature covers a specific set of sub-invocations
//! reached through `require_auth` / `require_auth_for_args`. A test that only
//! checks "some auth was required" cannot tell a correct delegation graph from
//! one where a sub-invocation quietly inherited a signature it should have
//! demanded separately — so the test passes while the deployed contract fails,
//! or worse, authorizes more than the signer agreed to.
//!
//! This module compares the authorization tree the host actually recorded
//! against the tree a test declares, and reports precisely where the two
//! diverge: a missing requirement, an unexpected one, or one attached at the
//! wrong point in the invocation stack.
//!
//! The entry point is the [`assert_auth_tree!`](crate::assert_auth_tree) macro.
//!
//! **Host-only:** this module depends on `std` and the Soroban host test
//! utilities, and is intended for `#[cfg(test)]` use on the host.
//!
//! # Example
//!
//! ```ignore
//! use crucible::prelude::*;
//! use soroban_sdk::IntoVal;
//!
//! // `alice` signs the escrow release, which in turn moves tokens.
//! assert_auth_tree!(env, [
//!     alice => escrow.release(token_id, amount) => [
//!         token.transfer(escrow_id, bob, amount),
//!     ],
//! ]);
//! ```

use soroban_sdk::testutils::{AuthorizedFunction, AuthorizedInvocation};
use soroban_sdk::{Address, Env, Symbol, Val, Vec as SorobanVec};

/// One node of an expected authorization tree.
///
/// Built by the [`assert_auth_tree!`](crate::assert_auth_tree) macro; construct
/// it directly only when generating expectations programmatically.
#[derive(Clone, Debug)]
pub struct ExpectedInvocation {
    /// Contract that called `require_auth` / `require_auth_for_args`.
    pub contract: Address,
    /// Name of the function whose invocation was authorized.
    pub function: Symbol,
    /// Arguments the authorization covers.
    ///
    /// These are the `require_auth_for_args` arguments, which need not equal
    /// the function's own arguments.
    pub args: SorobanVec<Val>,
    /// Sub-invocations this authorization is expected to cover, in order.
    pub sub_invocations: Vec<ExpectedInvocation>,
}

impl ExpectedInvocation {
    /// Creates a leaf expectation with no sub-invocations.
    pub fn new(contract: Address, function: Symbol, args: SorobanVec<Val>) -> Self {
        Self {
            contract,
            function,
            args,
            sub_invocations: Vec::new(),
        }
    }

    /// Adds the expected sub-invocations covered by this authorization.
    pub fn with_sub_invocations(mut self, sub_invocations: Vec<ExpectedInvocation>) -> Self {
        self.sub_invocations = sub_invocations;
        self
    }
}

/// An expected `(address, invocation tree)` entry.
#[derive(Clone, Debug)]
pub struct ExpectedAuth {
    /// The address expected to have authorized the tree.
    pub address: Address,
    /// The root of the invocation tree that address authorized.
    pub invocation: ExpectedInvocation,
}

/// A single way in which the recorded auth tree differs from the expected one.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum AuthMismatch {
    /// The test expected an authorization the host never recorded.
    Missing {
        /// Path from the tree root down to the missing node.
        path: String,
        /// What the test declared.
        expected: String,
    },
    /// The host recorded an authorization the test did not declare.
    Unexpected {
        /// Path from the tree root down to the extra node.
        path: String,
        /// What the host recorded.
        found: String,
    },
    /// An authorization exists at this position but does not match.
    ///
    /// This is the case that catches a delegation attached at the wrong point
    /// in the invocation stack: the requirement is present, but for a different
    /// contract, function, or argument set than the signer agreed to.
    Mismatched {
        /// Path from the tree root down to the differing node.
        path: String,
        /// What the test declared.
        expected: String,
        /// What the host recorded.
        found: String,
    },
}

impl AuthMismatch {
    /// Renders the mismatch as an indented diagnostic block.
    fn render(&self) -> String {
        match self {
            AuthMismatch::Missing { path, expected } => std::format!(
                "  missing authorization at {path}\n    expected: {expected}\n    found:    <nothing>"
            ),
            AuthMismatch::Unexpected { path, found } => std::format!(
                "  unexpected authorization at {path}\n    expected: <nothing>\n    found:    {found}"
            ),
            AuthMismatch::Mismatched {
                path,
                expected,
                found,
            } => std::format!(
                "  mismatched authorization at {path}\n    expected: {expected}\n    found:    {found}"
            ),
        }
    }
}

/// The outcome of comparing a recorded auth tree against an expected one.
#[derive(Clone, Debug, Default)]
pub struct AuthTreeReport {
    mismatches: Vec<AuthMismatch>,
    recorded: String,
}

impl AuthTreeReport {
    /// `true` when the recorded tree matched the expectation exactly.
    pub fn matches(&self) -> bool {
        self.mismatches.is_empty()
    }

    /// Every way in which the trees differ, outermost first.
    pub fn mismatches(&self) -> &[AuthMismatch] {
        &self.mismatches
    }

    /// The recorded authorization tree, rendered for reading.
    pub fn recorded_tree(&self) -> &str {
        &self.recorded
    }

    /// A full diagnostic showing each divergence and the recorded tree.
    pub fn diagnostic(&self) -> String {
        if self.matches() {
            return "authorization tree matched".to_string();
        }
        let mut out = String::from("authorization tree did not match:\n");
        for mismatch in &self.mismatches {
            out.push_str(&mismatch.render());
            out.push('\n');
        }
        out.push_str("\nrecorded authorization tree:\n");
        if self.recorded.is_empty() {
            out.push_str("  <no authorizations recorded>\n");
        } else {
            out.push_str(&self.recorded);
        }
        out
    }

    /// Panics with [`diagnostic`](Self::diagnostic) unless the trees matched.
    pub fn assert_matches(&self) {
        if !self.matches() {
            panic!("{}", self.diagnostic());
        }
    }
}

/// Compares the environment's recorded authorizations against `expected`.
///
/// Entries are matched positionally: the *n*th expected entry is compared with
/// the *n*th recorded one, which is the order the host reports them in.
pub fn verify_auth_tree(env: &Env, expected: &[ExpectedAuth]) -> AuthTreeReport {
    let recorded = env.auths();
    let mut mismatches = Vec::new();

    for (index, entry) in expected.iter().enumerate() {
        let path = std::format!("auth[{index}]");
        match recorded.get(index) {
            None => mismatches.push(AuthMismatch::Missing {
                path,
                expected: render_expected_entry(entry),
            }),
            Some((address, invocation)) => {
                if address != &entry.address {
                    mismatches.push(AuthMismatch::Mismatched {
                        path: std::format!("{path}.address"),
                        expected: render_address(&entry.address),
                        found: render_address(address),
                    });
                }
                compare_invocation(&path, &entry.invocation, invocation, &mut mismatches);
            }
        }
    }

    for (index, (address, invocation)) in recorded.iter().enumerate().skip(expected.len()) {
        mismatches.push(AuthMismatch::Unexpected {
            path: std::format!("auth[{index}]"),
            found: std::format!(
                "{} => {}",
                render_address(address),
                render_function(&invocation.function)
            ),
        });
    }

    AuthTreeReport {
        mismatches,
        recorded: render_recorded(&recorded),
    }
}

/// Compares one node and recurses into its sub-invocations.
fn compare_invocation(
    path: &str,
    expected: &ExpectedInvocation,
    found: &AuthorizedInvocation,
    mismatches: &mut Vec<AuthMismatch>,
) {
    let expected_rendered = render_expected_invocation(expected);
    let found_rendered = render_function(&found.function);

    if !function_matches(expected, &found.function) {
        mismatches.push(AuthMismatch::Mismatched {
            path: path.to_string(),
            expected: expected_rendered,
            found: found_rendered,
        });
    }

    for (index, sub) in expected.sub_invocations.iter().enumerate() {
        let sub_path = std::format!("{path}.sub[{index}]");
        match found.sub_invocations.get(index) {
            None => mismatches.push(AuthMismatch::Missing {
                path: sub_path,
                expected: render_expected_invocation(sub),
            }),
            Some(found_sub) => compare_invocation(&sub_path, sub, found_sub, mismatches),
        }
    }

    for (index, extra) in found
        .sub_invocations
        .iter()
        .enumerate()
        .skip(expected.sub_invocations.len())
    {
        mismatches.push(AuthMismatch::Unexpected {
            path: std::format!("{path}.sub[{index}]"),
            found: render_function(&extra.function),
        });
    }
}

/// `true` when the recorded function equals the expected contract, name, and args.
fn function_matches(expected: &ExpectedInvocation, found: &AuthorizedFunction) -> bool {
    match found {
        AuthorizedFunction::Contract((address, function, args)) => {
            address == &expected.contract && function == &expected.function && args == &expected.args
        }
        // Contract-creation authorizations are never produced by a
        // `require_auth` on a contract function, so they can never satisfy an
        // expectation expressed as `contract.function(args)`.
        _ => false,
    }
}

// ── Rendering ───────────────────────────────────────────────────────────────

fn render_address(address: &Address) -> String {
    address.to_string().to_string()
}

fn render_args(args: &SorobanVec<Val>) -> String {
    std::format!("{args:?}")
}

fn render_expected_invocation(expected: &ExpectedInvocation) -> String {
    std::format!(
        "{}.{:?}({})",
        render_address(&expected.contract),
        expected.function,
        render_args(&expected.args)
    )
}

fn render_expected_entry(entry: &ExpectedAuth) -> String {
    std::format!(
        "{} => {}",
        render_address(&entry.address),
        render_expected_invocation(&entry.invocation)
    )
}

fn render_function(function: &AuthorizedFunction) -> String {
    match function {
        AuthorizedFunction::Contract((address, name, args)) => std::format!(
            "{}.{:?}({})",
            render_address(address),
            name,
            render_args(args)
        ),
        other => std::format!("{other:?}"),
    }
}

/// Renders the host's recorded authorizations as an indented tree.
fn render_recorded(recorded: &[(Address, AuthorizedInvocation)]) -> String {
    let mut out = String::new();
    for (index, (address, invocation)) in recorded.iter().enumerate() {
        out.push_str(&std::format!(
            "  auth[{index}] {} =>\n",
            render_address(address)
        ));
        render_recorded_invocation(&mut out, invocation, 2);
    }
    out
}

fn render_recorded_invocation(out: &mut String, invocation: &AuthorizedInvocation, indent: usize) {
    out.push_str(&"  ".repeat(indent));
    out.push_str(&render_function(&invocation.function));
    out.push('\n');
    for sub in &invocation.sub_invocations {
        render_recorded_invocation(out, sub, indent + 1);
    }
}

/// Lets [`assert_auth_tree!`](crate::assert_auth_tree) accept either a
/// [`MockEnv`](crate::env::MockEnv) or a bare [`Env`].
///
/// Both borrow to the same underlying `Env`, so the macro does not need to know
/// which one a test happens to hold.
pub trait AsAuthEnv {
    /// Borrows the underlying Soroban environment.
    fn as_auth_env(&self) -> &Env;
}

impl AsAuthEnv for Env {
    fn as_auth_env(&self) -> &Env {
        self
    }
}

impl AsAuthEnv for crate::env::MockEnv {
    fn as_auth_env(&self) -> &Env {
        self.inner()
    }
}
