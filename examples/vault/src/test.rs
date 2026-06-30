#![cfg(test)]
extern crate std;

use crucible::prelude::*;
use crucible::{assert_emitted, assert_reverts};
use soroban_sdk::symbol_short;

use crate::{Vault, VaultClient};

const BASE_TIME: u64 = 1_000_000;
const LOCK_DAYS: u64 = 7;
const AMOUNT: i128 = 5_000_000;

struct Ctx {
    env: MockEnv,
    id: soroban_sdk::Address,
    alice: AccountHandle,
    bob: AccountHandle,
    token: MockToken,
}

impl Ctx {
    fn setup() -> Self {
        let env = MockEnv::builder()
            .at_timestamp(BASE_TIME)
            .with_contract::<Vault>()
            .with_account("alice", Stroops::xlm(10))
            .with_account("bob", Stroops::xlm(10))
            .build();

        let id = env.contract_id::<Vault>();
        let alice = env.account("alice");
        let bob = env.account("bob");
        let token = MockToken::new(&env, "TOK", 7);
        token.mint(&alice, AMOUNT * 3);
        token.mint(&bob, AMOUNT * 3);

        Ctx { env, id, alice, bob, token }
    }

    fn client(&self) -> VaultClient<'_> {
        VaultClient::new(self.env.inner(), &self.id)
    }

    fn unlock_time(&self) -> u64 {
        BASE_TIME + Duration::days(LOCK_DAYS).as_seconds()
    }

    /// Deposit AMOUNT from alice with default LOCK_DAYS lock.
    fn alice_deposit(&self) -> u64 {
        self.env.mock_all_auths();
        self.client()
            .deposit(&self.alice, &self.token.address(), &AMOUNT, &self.unlock_time())
    }
}

// ---------------------------------------------------------------------------
// Deposit tests
// ---------------------------------------------------------------------------

#[test]
fn test_deposit_returns_sequential_ids() {
    let ctx = Ctx::setup();
    ctx.env.mock_all_auths();
    let id0 = ctx.client().deposit(&ctx.alice, &ctx.token.address(), &AMOUNT, &ctx.unlock_time());
    let id1 = ctx.client().deposit(&ctx.alice, &ctx.token.address(), &AMOUNT, &ctx.unlock_time());
    assert_eq!(id0, 0);
    assert_eq!(id1, 1);
}

#[test]
fn test_deposit_transfers_tokens_to_vault() {
    let ctx = Ctx::setup();
    let before = ctx.token.balance(&ctx.alice);
    ctx.alice_deposit();
    assert_eq!(ctx.token.balance(&ctx.alice), before - AMOUNT);
    assert_eq!(ctx.token.balance(&ctx.id), AMOUNT);
}

#[test]
fn test_deposit_zero_amount_reverts() {
    let ctx = Ctx::setup();
    ctx.env.mock_all_auths();
    assert_reverts!(
        ctx.client().deposit(&ctx.alice, &ctx.token.address(), &0, &ctx.unlock_time()),
        "positive"
    );
}

#[test]
fn test_deposit_negative_amount_reverts() {
    let ctx = Ctx::setup();
    ctx.env.mock_all_auths();
    assert_reverts!(
        ctx.client()
            .deposit(&ctx.alice, &ctx.token.address(), &(-1_i128), &ctx.unlock_time()),
        "positive"
    );
}

#[test]
fn test_deposit_past_unlock_time_reverts() {
    let ctx = Ctx::setup();
    ctx.env.mock_all_auths();
    // unlock_time == current timestamp is not in the future
    assert_reverts!(
        ctx.client()
            .deposit(&ctx.alice, &ctx.token.address(), &AMOUNT, &BASE_TIME),
        "future"
    );
}

#[test]
fn test_deposit_emits_event() {
    let ctx = Ctx::setup();
    let id = ctx.alice_deposit();
    assert_emitted!(
        ctx.env,
        ctx.id,
        (symbol_short!("deposit"),),
        (id, ctx.alice.address(), AMOUNT)
    );
}

#[test]
fn test_get_deposit_returns_correct_record() {
    let ctx = Ctx::setup();
    let id = ctx.alice_deposit();
    let dep = ctx.client().get_deposit(&id);
    assert_eq!(dep.owner, ctx.alice.address());
    assert_eq!(dep.amount, AMOUNT);
    assert_eq!(dep.unlock_time, ctx.unlock_time());
    assert!(!dep.withdrawn);
}

// ---------------------------------------------------------------------------
// Withdraw tests
// ---------------------------------------------------------------------------

#[test]
fn test_withdraw_before_unlock_reverts() {
    let ctx = Ctx::setup();
    let id = ctx.alice_deposit();
    // Advance to just before unlock
    ctx.env.advance_time(Duration::days(LOCK_DAYS - 1));
    ctx.env.mock_all_auths();
    assert_reverts!(ctx.client().withdraw(&id), "locked");
}

#[test]
fn test_withdraw_exactly_at_unlock_time_reverts() {
    let ctx = Ctx::setup();
    let id = ctx.alice_deposit();
    // advance to exactly unlock_time — still locked (must be strictly past)
    ctx.env.advance_time(Duration::days(LOCK_DAYS));
    ctx.env.mock_all_auths();
    // unlock_time = BASE_TIME + LOCK_DAYS*86400; now == unlock_time → still locked
    assert_reverts!(ctx.client().withdraw(&id), "locked");
}

#[test]
fn test_withdraw_after_unlock_succeeds() {
    let ctx = Ctx::setup();
    let id = ctx.alice_deposit();
    let before = ctx.token.balance(&ctx.alice);
    // Advance past lock
    ctx.env.advance_time(Duration::days(LOCK_DAYS + 1));
    ctx.env.mock_all_auths();
    ctx.client().withdraw(&id);
    assert_eq!(ctx.token.balance(&ctx.alice), before + AMOUNT);
    assert_eq!(ctx.token.balance(&ctx.id), 0);
}

#[test]
fn test_withdraw_marks_deposit_as_withdrawn() {
    let ctx = Ctx::setup();
    let id = ctx.alice_deposit();
    ctx.env.advance_time(Duration::days(LOCK_DAYS + 1));
    ctx.env.mock_all_auths();
    ctx.client().withdraw(&id);
    assert!(ctx.client().get_deposit(&id).withdrawn);
}

#[test]
fn test_double_withdraw_reverts() {
    let ctx = Ctx::setup();
    let id = ctx.alice_deposit();
    ctx.env.advance_time(Duration::days(LOCK_DAYS + 1));
    ctx.env.mock_all_auths();
    ctx.client().withdraw(&id);
    assert_reverts!(ctx.client().withdraw(&id), "already withdrawn");
}

#[test]
fn test_withdraw_emits_event() {
    let ctx = Ctx::setup();
    let id = ctx.alice_deposit();
    ctx.env.advance_time(Duration::days(LOCK_DAYS + 1));
    ctx.env.mock_all_auths();
    ctx.client().withdraw(&id);
    assert_emitted!(
        ctx.env,
        ctx.id,
        (symbol_short!("withdraw"),),
        (id, ctx.alice.address(), AMOUNT)
    );
}

#[test]
fn test_get_nonexistent_deposit_reverts() {
    let ctx = Ctx::setup();
    assert_reverts!(ctx.client().get_deposit(&999u64), "not found");
}

// ---------------------------------------------------------------------------
// Multi-deposit scenarios
// ---------------------------------------------------------------------------

#[test]
fn test_multiple_owners_independent_deposits() {
    let ctx = Ctx::setup();
    ctx.env.mock_all_auths();
    let alice_unlock = BASE_TIME + Duration::days(7).as_seconds();
    let bob_unlock = BASE_TIME + Duration::days(14).as_seconds();

    let aid = ctx.client().deposit(&ctx.alice, &ctx.token.address(), &AMOUNT, &alice_unlock);
    let bid = ctx.client().deposit(&ctx.bob, &ctx.token.address(), &AMOUNT, &bob_unlock);

    // Advance past alice's lock but not bob's
    ctx.env.advance_time(Duration::days(8));

    // Alice can withdraw
    ctx.client().withdraw(&aid);
    assert_eq!(ctx.token.balance(&ctx.alice), AMOUNT * 3); // she got her deposit back

    // Bob cannot yet
    assert_reverts!(ctx.client().withdraw(&bid), "locked");
}

#[test]
fn test_withdraw_invalid_id_reverts() {
    let ctx = Ctx::setup();
    ctx.env.mock_all_auths();
    assert_reverts!(ctx.client().withdraw(&42u64), "not found");
}
