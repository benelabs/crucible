//! Dynamic auth invocation tree verification tests.
//!
//! Covers the two shapes the issue calls out: a standard Stellar token
//! transfer, where the signer authorizes a single call, and a multi-party
//! escrow, where one signature must cover a nested delegation down into the
//! token contract.

use crucible::prelude::*;
use crucible::{assert_auth_tree, auth_tree::verify_auth_tree};
use soroban_sdk::testutils::Address as _;
use soroban_sdk::{
    contract, contractimpl, contracttype, Address, Env, IntoVal, Symbol, Val, Vec as SorobanVec,
};

#[contracttype]
#[derive(Clone)]
enum DataKey {
    Balance(Address),
    Token,
    Arbiter,
    Beneficiary,
}

/// A minimal token whose `transfer` requires the sender's authorization.
#[contract]
struct Token;

#[contractimpl]
impl Token {
    pub fn mint(env: Env, to: Address, amount: i128) {
        let key = DataKey::Balance(to);
        let balance: i128 = env.storage().instance().get(&key).unwrap_or(0);
        env.storage().instance().set(&key, &(balance + amount));
    }

    pub fn balance(env: Env, who: Address) -> i128 {
        env.storage()
            .instance()
            .get(&DataKey::Balance(who))
            .unwrap_or(0)
    }

    pub fn transfer(env: Env, from: Address, to: Address, amount: i128) {
        from.require_auth();

        let from_key = DataKey::Balance(from);
        let from_balance: i128 = env.storage().instance().get(&from_key).unwrap_or(0);
        assert!(from_balance >= amount, "insufficient balance");
        env.storage()
            .instance()
            .set(&from_key, &(from_balance - amount));

        let to_key = DataKey::Balance(to);
        let to_balance: i128 = env.storage().instance().get(&to_key).unwrap_or(0);
        env.storage().instance().set(&to_key, &(to_balance + amount));
    }
}

/// A two-party escrow. The depositor authorizes the escrow call, and that one
/// signature must also cover the token `transfer` the escrow makes on their
/// behalf — the nested delegation this engine exists to verify.
#[contract]
struct Escrow;

#[contractimpl]
impl Escrow {
    pub fn init(env: Env, token: Address, arbiter: Address, beneficiary: Address) {
        env.storage().instance().set(&DataKey::Token, &token);
        env.storage().instance().set(&DataKey::Arbiter, &arbiter);
        env.storage()
            .instance()
            .set(&DataKey::Beneficiary, &beneficiary);
    }

    /// Pulls `amount` from `depositor` into the escrow.
    pub fn deposit(env: Env, depositor: Address, amount: i128) {
        depositor.require_auth();

        let token: Address = env.storage().instance().get(&DataKey::Token).unwrap();
        TokenClient::new(&env, &token).transfer(
            &depositor,
            &env.current_contract_address(),
            &amount,
        );
    }
}

/// A release gated on two parties, so one call requires two signatures and the
/// depositor's signature additionally covers the token transfer.
#[contract]
struct MultiPartyEscrow;

#[contractimpl]
impl MultiPartyEscrow {
    pub fn settle(
        env: Env,
        depositor: Address,
        arbiter: Address,
        token: Address,
        to: Address,
        amount: i128,
    ) {
        depositor.require_auth();
        arbiter.require_auth();

        TokenClient::new(&env, &token).transfer(&depositor, &to, &amount);
    }
}

struct Fixture {
    env: MockEnv,
    token: Address,
    alice: Address,
    bob: Address,
}

fn setup() -> Fixture {
    let env = MockEnv::default();
    let inner = env.inner();
    let token = inner.register(Token, ());
    let alice = Address::generate(inner);
    let bob = Address::generate(inner);

    env.mock_all_auths();
    TokenClient::new(inner, &token).mint(&alice, &1_000);

    Fixture {
        env,
        token,
        alice,
        bob,
    }
}

// ── Standard Stellar token transfer ─────────────────────────────────────────

#[test]
fn token_transfer_records_a_single_flat_authorization() {
    let f = setup();
    let client = TokenClient::new(f.env.inner(), &f.token);

    client.transfer(&f.alice, &f.bob, &100);

    let token = f.token.clone();
    let alice = f.alice.clone();
    assert_auth_tree!(f.env, [
        alice => token.transfer(f.alice.clone(), f.bob.clone(), 100_i128),
    ]);

    assert_eq!(client.balance(&f.alice), 900);
    assert_eq!(client.balance(&f.bob), 100);
}

#[test]
fn a_wrong_signer_is_reported_as_a_mismatch() {
    let f = setup();
    TokenClient::new(f.env.inner(), &f.token).transfer(&f.alice, &f.bob, &100);

    // Declaring bob as the signer must fail: alice signed the transfer.
    let expected = vec![ExpectedAuth {
        address: f.bob.clone(),
        invocation: ExpectedInvocation::new(
            f.token.clone(),
            Symbol::new(f.env.inner(), "transfer"),
            args(&f.env, &[
                f.alice.clone().into_val(f.env.inner()),
                f.bob.clone().into_val(f.env.inner()),
                100_i128.into_val(f.env.inner()),
            ]),
        ),
    }];

    let report = verify_auth_tree(f.env.inner(), &expected);
    assert!(!report.matches());
    assert!(matches!(
        report.mismatches()[0],
        AuthMismatch::Mismatched { .. }
    ));
    assert!(report.diagnostic().contains("auth[0].address"));
}

#[test]
fn wrong_arguments_are_reported_as_a_mismatch() {
    let f = setup();
    TokenClient::new(f.env.inner(), &f.token).transfer(&f.alice, &f.bob, &100);

    // The signer and function are right, but the amount is not the one signed.
    let expected = vec![ExpectedAuth {
        address: f.alice.clone(),
        invocation: ExpectedInvocation::new(
            f.token.clone(),
            Symbol::new(f.env.inner(), "transfer"),
            args(&f.env, &[
                f.alice.clone().into_val(f.env.inner()),
                f.bob.clone().into_val(f.env.inner()),
                999_i128.into_val(f.env.inner()),
            ]),
        ),
    }];

    let report = verify_auth_tree(f.env.inner(), &expected);
    assert!(
        !report.matches(),
        "an amount the signer never approved must not verify"
    );
    assert!(matches!(
        report.mismatches()[0],
        AuthMismatch::Mismatched { .. }
    ));
}

#[test]
fn wrong_function_is_reported_as_a_mismatch() {
    let f = setup();
    TokenClient::new(f.env.inner(), &f.token).transfer(&f.alice, &f.bob, &100);

    let expected = vec![ExpectedAuth {
        address: f.alice.clone(),
        invocation: ExpectedInvocation::new(
            f.token.clone(),
            Symbol::new(f.env.inner(), "mint"),
            args(&f.env, &[]),
        ),
    }];

    let report = verify_auth_tree(f.env.inner(), &expected);
    assert!(!report.matches());
    assert!(report.diagnostic().contains("mismatched authorization"));
}

// ── Multi-party escrow delegation ───────────────────────────────────────────

#[test]
fn escrow_deposit_records_a_nested_delegation() {
    let f = setup();
    let inner = f.env.inner();
    let escrow = inner.register(Escrow, ());

    f.env.mock_all_auths();
    EscrowClient::new(inner, &escrow).init(&f.token, &f.alice, &f.bob);

    EscrowClient::new(inner, &escrow).deposit(&f.alice, &250);

    // Alice's one signature on `deposit` must also cover the token `transfer`
    // the escrow makes on her behalf; that nesting is the point of the tree.
    let escrow_id = escrow.clone();
    let token = f.token.clone();
    let alice = f.alice.clone();
    assert_auth_tree!(f.env, [
        alice => escrow_id.deposit(f.alice.clone(), 250_i128) => [
            token.transfer(f.alice.clone(), escrow.clone(), 250_i128),
        ],
    ]);

    assert_eq!(TokenClient::new(inner, &f.token).balance(&escrow), 250);
    assert_eq!(TokenClient::new(inner, &f.token).balance(&f.alice), 750);
}

#[test]
fn an_undeclared_sub_invocation_is_reported_with_its_path() {
    let f = setup();
    let inner = f.env.inner();
    let escrow = inner.register(Escrow, ());

    f.env.mock_all_auths();
    EscrowClient::new(inner, &escrow).init(&f.token, &f.alice, &f.bob);
    EscrowClient::new(inner, &escrow).deposit(&f.alice, &250);

    // Declaring the root without its sub-invocation leaves the recorded
    // `transfer` unaccounted for — the under-specified test this engine
    // exists to catch.
    let expected = vec![ExpectedAuth {
        address: f.alice.clone(),
        invocation: ExpectedInvocation::new(
            escrow.clone(),
            Symbol::new(inner, "deposit"),
            args(&f.env, &[
                f.alice.clone().into_val(inner),
                250_i128.into_val(inner),
            ]),
        ),
    }];

    let report = verify_auth_tree(inner, &expected);
    assert!(
        !report.matches(),
        "an unaccounted-for sub-invocation must fail verification"
    );
    let diagnostic = report.diagnostic();
    assert!(
        diagnostic.contains("unexpected authorization at auth[0].sub[0]"),
        "diagnostic must locate the extra node, was:
{diagnostic}"
    );
    assert!(
        diagnostic.contains("recorded authorization tree:"),
        "diagnostic must show the recorded tree"
    );
}

#[test]
fn an_expected_sub_invocation_that_never_happened_is_reported() {
    let f = setup();
    let inner = f.env.inner();

    TokenClient::new(inner, &f.token).transfer(&f.alice, &f.bob, &100);

    // A flat transfer authorizes nothing beneath it, so a declared child is
    // missing rather than merely different.
    let expected = vec![ExpectedAuth {
        address: f.alice.clone(),
        invocation: ExpectedInvocation::new(
            f.token.clone(),
            Symbol::new(inner, "transfer"),
            args(&f.env, &[
                f.alice.clone().into_val(inner),
                f.bob.clone().into_val(inner),
                100_i128.into_val(inner),
            ]),
        )
        .with_sub_invocations(vec![ExpectedInvocation::new(
            f.token.clone(),
            Symbol::new(inner, "burn"),
            args(&f.env, &[]),
        )]),
    }];

    let report = verify_auth_tree(inner, &expected);
    assert!(!report.matches());
    assert!(matches!(
        report.mismatches()[0],
        AuthMismatch::Missing { .. }
    ));
    assert!(report
        .diagnostic()
        .contains("missing authorization at auth[0].sub[0]"));
}

#[test]
fn multi_party_settlement_records_one_tree_per_signer() {
    let f = setup();
    let inner = f.env.inner();
    let escrow = inner.register(MultiPartyEscrow, ());
    let carol = Address::generate(inner);

    f.env.mock_all_auths();

    MultiPartyEscrowClient::new(inner, &escrow)
        .settle(&f.alice, &carol, &f.token, &f.bob, &250);

    // Two signers authorize the same root call. Only alice, the depositor,
    // carries the token transfer beneath her signature.
    let escrow_id = escrow.clone();
    let token = f.token.clone();
    let alice = f.alice.clone();
    let carol_signer = carol.clone();
    assert_auth_tree!(f.env, [
        alice => escrow_id.settle(
            f.alice.clone(), carol.clone(), f.token.clone(), f.bob.clone(), 250_i128
        ) => [
            token.transfer(f.alice.clone(), f.bob.clone(), 250_i128),
        ],
        carol_signer => escrow_id.settle(
            f.alice.clone(), carol.clone(), f.token.clone(), f.bob.clone(), 250_i128
        ),
    ]);

    assert_eq!(TokenClient::new(inner, &f.token).balance(&f.bob), 250);
}

#[test]
fn an_unexpected_extra_signer_is_reported() {
    let f = setup();
    let inner = f.env.inner();
    let escrow = inner.register(MultiPartyEscrow, ());
    let carol = Address::generate(inner);

    f.env.mock_all_auths();
    MultiPartyEscrowClient::new(inner, &escrow)
        .settle(&f.alice, &carol, &f.token, &f.bob, &250);

    // Declaring only alice leaves carol's signature unaccounted for.
    let settle_args = args(&f.env, &[
        f.alice.clone().into_val(inner),
        carol.clone().into_val(inner),
        f.token.clone().into_val(inner),
        f.bob.clone().into_val(inner),
        250_i128.into_val(inner),
    ]);
    let expected = vec![ExpectedAuth {
        address: f.alice.clone(),
        invocation: ExpectedInvocation::new(
            escrow.clone(),
            Symbol::new(inner, "settle"),
            settle_args,
        )
        .with_sub_invocations(vec![ExpectedInvocation::new(
            f.token.clone(),
            Symbol::new(inner, "transfer"),
            args(&f.env, &[
                f.alice.clone().into_val(inner),
                f.bob.clone().into_val(inner),
                250_i128.into_val(inner),
            ]),
        )]),
    }];

    let report = verify_auth_tree(inner, &expected);
    assert!(!report.matches());
    assert!(report
        .diagnostic()
        .contains("unexpected authorization at auth[1]"));
}

// ── Report behaviour ────────────────────────────────────────────────────────

#[test]
fn a_matching_tree_reports_no_mismatches() {
    let f = setup();
    TokenClient::new(f.env.inner(), &f.token).transfer(&f.alice, &f.bob, &100);

    let expected = vec![ExpectedAuth {
        address: f.alice.clone(),
        invocation: ExpectedInvocation::new(
            f.token.clone(),
            Symbol::new(f.env.inner(), "transfer"),
            args(&f.env, &[
                f.alice.clone().into_val(f.env.inner()),
                f.bob.clone().into_val(f.env.inner()),
                100_i128.into_val(f.env.inner()),
            ]),
        ),
    }];

    let report = verify_auth_tree(f.env.inner(), &expected);
    assert!(report.matches());
    assert!(report.mismatches().is_empty());
    assert_eq!(report.diagnostic(), "authorization tree matched");
    report.assert_matches();
}

#[test]
fn expecting_an_authorization_that_never_occurred_is_missing() {
    let f = setup();

    // No call was made, so nothing was authorized.
    let expected = vec![ExpectedAuth {
        address: f.alice.clone(),
        invocation: ExpectedInvocation::new(
            f.token.clone(),
            Symbol::new(f.env.inner(), "transfer"),
            args(&f.env, &[]),
        ),
    }];

    let report = verify_auth_tree(f.env.inner(), &expected);
    assert!(!report.matches());
    assert!(matches!(
        report.mismatches()[0],
        AuthMismatch::Missing { .. }
    ));
    assert!(report
        .diagnostic()
        .contains("<no authorizations recorded>"));
}

#[test]
fn assert_auth_tree_panics_with_the_full_diagnostic() {
    let f = setup();
    TokenClient::new(f.env.inner(), &f.token).transfer(&f.alice, &f.bob, &100);

    let token = f.token.clone();
    let bob = f.bob.clone();
    let failure = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        // bob never signed anything.
        assert_auth_tree!(f.env, [
            bob => token.transfer(f.alice.clone(), f.bob.clone(), 100_i128),
        ]);
    }))
    .expect_err("a wrong signer must panic");

    let message = failure
        .downcast_ref::<String>()
        .expect("panic carries a message");
    assert!(message.contains("authorization tree did not match"));
    assert!(message.contains("recorded authorization tree:"));
}

#[test]
fn assert_auth_tree_accepts_a_bare_env() {
    let f = setup();
    let inner = f.env.inner().clone();
    TokenClient::new(&inner, &f.token).transfer(&f.alice, &f.bob, &100);

    let token = f.token.clone();
    let alice = f.alice.clone();
    assert_auth_tree!(inner, [
        alice => token.transfer(f.alice.clone(), f.bob.clone(), 100_i128),
    ]);
}

#[test]
fn an_empty_expectation_rejects_any_recorded_authorization() {
    let f = setup();
    TokenClient::new(f.env.inner(), &f.token).transfer(&f.alice, &f.bob, &100);

    let report = verify_auth_tree(f.env.inner(), &[]);
    assert!(
        !report.matches(),
        "declaring no authorizations must not silently pass"
    );
    assert!(matches!(
        report.mismatches()[0],
        AuthMismatch::Unexpected { .. }
    ));
}

/// Collects pre-converted `Val`s into the `Vec<Val>` the comparison expects.
fn args(env: &MockEnv, values: &[Val]) -> SorobanVec<Val> {
    let mut out = SorobanVec::new(env.inner());
    for value in values {
        out.push_back(*value);
    }
    out
}
