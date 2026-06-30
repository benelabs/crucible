#![cfg(test)]
extern crate std;

use crucible::prelude::*;
use crucible::{assert_emitted, assert_reverts};
use soroban_sdk::{symbol_short, Address};

use crate::{Token, TokenClient};

// ---------------------------------------------------------------------------
// Test fixture
// ---------------------------------------------------------------------------

#[fixture]
struct Ctx {
    pub env: MockEnv,
    pub id: Address,
}

impl Ctx {
    pub fn setup() -> Self {
        let env = MockEnv::builder()
            .with_contract::<Token>()
            .with_account("admin", Stroops::xlm(100))
            .with_account("alice", Stroops::xlm(100))
            .with_account("bob", Stroops::xlm(100))
            .build();
        let id = env.contract_id::<Token>();
        let admin = env.account("admin");

        // Initialize with mock auth so admin.require_auth() in sub-calls passes
        env.mock_all_auths();
        TokenClient::new(env.inner(), &id).initialize(&admin);

        Ctx { env, id }
    }

    fn client(&self) -> TokenClient<'_> {
        TokenClient::new(self.env.inner(), &self.id)
    }
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[test]
fn test_balance_starts_at_zero() {
    let f = Ctx::setup();
    let alice = f.env.account("alice");
    assert_eq!(f.client().balance(&alice), 0);
}

#[test]
fn test_admin_can_mint() {
    let f = Ctx::setup();
    f.env.mock_all_auths();
    let alice = f.env.account("alice");
    f.client().mint(&alice, &1_000_i128);
    assert_eq!(f.client().balance(&alice), 1_000);
}

#[test]
fn test_mint_without_auth_reverts() {
    // Fresh env — no mock_all_auths, so admin.require_auth() panics
    let env = MockEnv::builder()
        .with_contract::<Token>()
        .with_account("admin", Stroops::xlm(100))
        .with_account("alice", Stroops::xlm(100))
        .build();
    let id = env.contract_id::<Token>();
    let admin = env.account("alice"); // wrong admin address
    // Do not mock auths; this should trigger auth failure
    // env.mock_all_auths(); // removed to test real auth behavior
    TokenClient::new(env.inner(), &id).initialize(&admin);
    assert_reverts!(
        TokenClient::new(env.inner(), &id).mint(&env.account("alice"), &0_i128),
        "admin requires auth"
    );
}

#[test]
fn test_transfer_moves_balance() {
    let f = Ctx::setup();
    f.env.mock_all_auths();
    let alice = f.env.account("alice");
    let bob = f.env.account("bob");
    f.client().mint(&alice, &1_000_i128);
    f.client().transfer(&alice, &bob, &400_i128);
    assert_eq!(f.client().balance(&alice), 600);
    assert_eq!(f.client().balance(&bob), 400);
}

#[test]
fn test_transfer_insufficient_balance_reverts() {
    let f = Ctx::setup();
    f.env.mock_all_auths();
    let alice = f.env.account("alice");
    let bob = f.env.account("bob");
    f.client().mint(&alice, &100_i128);
    assert_reverts!(
        f.client().transfer(&alice, &bob, &500_i128),
        "insufficient balance"
    );
}

#[test]
fn test_burn_reduces_balance() {
    let f = Ctx::setup();
    f.env.mock_all_auths();
    let alice = f.env.account("alice");
    f.client().mint(&alice, &1_000_i128);
    f.client().burn(&alice, &300_i128);
    assert_eq!(f.client().balance(&alice), 700);
}

#[test]
fn test_burn_more_than_balance_reverts() {
    let f = Ctx::setup();
    f.env.mock_all_auths();
    let alice = f.env.account("alice");
    f.client().mint(&alice, &100_i128);
    assert_reverts!(f.client().burn(&alice, &500_i128), "insufficient balance");
}

#[test]
fn test_approve_and_transfer_from() {
    let f = Ctx::setup();
    f.env.mock_all_auths();
    let alice = f.env.account("alice");
    let bob = f.env.account("bob");
    f.client().mint(&alice, &1_000_i128);
    f.client().approve(&alice, &bob, &500_i128);
    assert_eq!(f.client().allowance(&alice, &bob), 500);
    f.client().transfer_from(&bob, &alice, &bob, &300_i128);
    assert_eq!(f.client().balance(&alice), 700);
    assert_eq!(f.client().balance(&bob), 300);
    assert_eq!(f.client().allowance(&alice, &bob), 200); // 500 - 300
}

#[test]
fn test_transfer_from_exceeds_allowance_reverts() {
    let f = Ctx::setup();
    f.env.mock_all_auths();
    let alice = f.env.account("alice");
    let bob = f.env.account("bob");
    f.client().mint(&alice, &1_000_i128);
    f.client().approve(&alice, &bob, &100_i128);
    assert_reverts!(
        f.client().transfer_from(&bob, &alice, &bob, &500_i128),
        "insufficient allowance"
    );
}

#[test]
fn test_mint_emits_event() {
    let f = Ctx::setup();
    f.env.mock_all_auths();
    let alice = f.env.account("alice");
    f.client().mint(&alice, &1_000_i128);
    assert_emitted!(
        f.env,
        f.id,
        (symbol_short!("mint"), alice.address()),
        1_000_i128
    );
}

#[test]
fn test_balance_query_emits_no_event() {
    // balance() is a read-only query — calling it must not add new events.
    // Capture the count before and after to check no new events were emitted
    // (setup may emit XLM-mint events, which we don't want to interfere).
    let f = Ctx::setup();
    let alice = f.env.account("alice");
    {
        use soroban_sdk::testutils::Events as _;
        let before = f.env.inner().events().all().events().len();
        f.client().balance(&alice);
        let after = f.env.inner().events().all().events().len();
        assert_eq!(before, after, "balance() should not emit events");
    }
}

#[test]
fn test_xlm_token_is_independent() {
    // Demonstrates MockToken (SAC) alongside our custom contract token.
    // Both are independent; balances don't interfere.
    let f = Ctx::setup();
    f.env.mock_all_auths();
    let alice = f.env.account("alice");

    let xlm = MockToken::xlm(&f.env);
    xlm.mint(&alice, 5_000_000); // 0.5 XLM

    f.client().mint(&alice, &250_i128);

    assert_eq!(
        xlm.balance(&alice),
        Stroops::xlm(100).as_stroops() + 5_000_000
    );
    assert_eq!(f.client().balance(&alice), 250);
}
