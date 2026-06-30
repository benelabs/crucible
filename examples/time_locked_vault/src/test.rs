#![cfg(test)]
extern crate std;

use crucible::assert_reverts;
use crucible::prelude::*;

use crate::{TimeLockedVault, TimeLockedVaultClient};

const AMOUNT: i128 = 5_000_000;
const BASE_TIME: u64 = 1_000_000;
const LOCK_DURATION: u64 = 86_400; // 1 day

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

struct Ctx {
    pub env: MockEnv,
    pub id: soroban_sdk::Address,
    pub alice: AccountHandle,
    pub bob: AccountHandle,
    pub token: MockToken,
}

impl Ctx {
    fn setup() -> Self {
        let env = MockEnv::builder()
            .at_timestamp(BASE_TIME)
            .with_contract::<TimeLockedVault>()
            .with_account("alice", Stroops::xlm(100))
            .with_account("bob", Stroops::xlm(100))
            .build();

        let id = env.contract_id::<TimeLockedVault>();
        let alice = env.account("alice");
        let bob = env.account("bob");

        let token = MockToken::new(&env, "USDC", 6);
        token.mint(&alice, AMOUNT * 3);
        token.mint(&bob, AMOUNT * 3);

        Ctx {
            env,
            id,
            alice,
            bob,
            token,
        }
    }

    fn client(&self) -> TimeLockedVaultClient<'_> {
        TimeLockedVaultClient::new(self.env.inner(), &self.id)
    }

    fn unlock_time(&self) -> u64 {
        BASE_TIME + LOCK_DURATION
    }
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[test]
fn test_deposit_transfers_tokens_to_vault() {
    let ctx = Ctx::setup();
    ctx.env.mock_all_auths();
    ctx.client().deposit(
        &ctx.alice.address(),
        &ctx.token.address(),
        &AMOUNT,
        &ctx.unlock_time(),
    );

    assert_eq!(ctx.token.balance(&ctx.id), AMOUNT);
    assert_eq!(ctx.token.balance(&ctx.alice), AMOUNT * 2);
}

#[test]
fn test_deposit_returns_incrementing_ids() {
    let ctx = Ctx::setup();
    ctx.env.mock_all_auths();
    let id0 = ctx.client().deposit(
        &ctx.alice.address(),
        &ctx.token.address(),
        &AMOUNT,
        &ctx.unlock_time(),
    );
    let id1 = ctx.client().deposit(
        &ctx.alice.address(),
        &ctx.token.address(),
        &AMOUNT,
        &ctx.unlock_time(),
    );

    assert_eq!(id0, 0);
    assert_eq!(id1, 1);
}

#[test]
fn test_withdraw_after_unlock_time() {
    let ctx = Ctx::setup();
    ctx.env.mock_all_auths();
    let id = ctx.client().deposit(
        &ctx.alice.address(),
        &ctx.token.address(),
        &AMOUNT,
        &ctx.unlock_time(),
    );

    ctx.env.advance_time(Duration::seconds(LOCK_DURATION + 1));
    ctx.client().withdraw(&id);

    assert_eq!(ctx.token.balance(&ctx.alice), AMOUNT * 3);
    assert_eq!(ctx.token.balance(&ctx.id), 0);
    assert!(ctx.client().get_deposit(&id).withdrawn);
}

#[test]
fn test_withdraw_before_unlock_time_reverts() {
    let ctx = Ctx::setup();
    ctx.env.mock_all_auths();
    let id = ctx.client().deposit(
        &ctx.alice.address(),
        &ctx.token.address(),
        &AMOUNT,
        &ctx.unlock_time(),
    );

    // Do NOT advance time.
    assert_reverts!(ctx.client().withdraw(&id), "time lock");
}

#[test]
fn test_double_withdraw_reverts() {
    let ctx = Ctx::setup();
    ctx.env.mock_all_auths();
    let id = ctx.client().deposit(
        &ctx.alice.address(),
        &ctx.token.address(),
        &AMOUNT,
        &ctx.unlock_time(),
    );

    ctx.env.advance_time(Duration::seconds(LOCK_DURATION + 1));
    ctx.client().withdraw(&id);

    assert_reverts!(ctx.client().withdraw(&id), "already withdrawn");
}

#[test]
fn test_deposit_zero_amount_reverts() {
    let ctx = Ctx::setup();
    ctx.env.mock_all_auths();
    assert_reverts!(
        ctx.client().deposit(
            &ctx.alice.address(),
            &ctx.token.address(),
            &0_i128,
            &ctx.unlock_time()
        ),
        "positive"
    );
}

#[test]
fn test_deposit_past_unlock_time_reverts() {
    let ctx = Ctx::setup();
    ctx.env.mock_all_auths();
    // unlock_time in the past
    assert_reverts!(
        ctx.client()
            .deposit(&ctx.alice.address(), &ctx.token.address(), &AMOUNT, &(BASE_TIME - 1)),
        "future"
    );
}

#[test]
fn test_multiple_depositors_independent() {
    let ctx = Ctx::setup();
    ctx.env.mock_all_auths();
    let id_a = ctx.client().deposit(
        &ctx.alice.address(),
        &ctx.token.address(),
        &AMOUNT,
        &ctx.unlock_time(),
    );
    let id_b = ctx
        .client()
        .deposit(&ctx.bob.address(), &ctx.token.address(), &AMOUNT, &ctx.unlock_time());

    ctx.env.advance_time(Duration::seconds(LOCK_DURATION + 1));
    ctx.client().withdraw(&id_a);
    ctx.client().withdraw(&id_b);

    assert_eq!(ctx.token.balance(&ctx.alice), AMOUNT * 3);
    assert_eq!(ctx.token.balance(&ctx.bob), AMOUNT * 3);
}

#[test]
fn test_get_deposit_returns_correct_data() {
    let ctx = Ctx::setup();
    ctx.env.mock_all_auths();
    let id = ctx.client().deposit(
        &ctx.alice.address(),
        &ctx.token.address(),
        &AMOUNT,
        &ctx.unlock_time(),
    );

    let dep = ctx.client().get_deposit(&id);
    assert_eq!(dep.owner, ctx.alice.address());
    assert_eq!(dep.amount, AMOUNT);
    assert_eq!(dep.unlock_time, ctx.unlock_time());
    assert!(!dep.withdrawn);
}

#[test]
fn test_deposit_emits_event() {
    let ctx = Ctx::setup();
    ctx.env.mock_all_auths();
    ctx.client().deposit(
        &ctx.alice.address(),
        &ctx.token.address(),
        &AMOUNT,
        &ctx.unlock_time(),
    );
    let matching = ctx
        .env
        .events_matching((soroban_sdk::symbol_short!("deposited"),));
    assert!(
        !matching.is_empty(),
        "expected deposited event to be emitted"
    );
}

#[test]
fn test_withdraw_emits_event() {
    let ctx = Ctx::setup();
    ctx.env.mock_all_auths();
    let id = ctx.client().deposit(
        &ctx.alice.address(),
        &ctx.token.address(),
        &AMOUNT,
        &ctx.unlock_time(),
    );

    ctx.env.advance_time(Duration::seconds(LOCK_DURATION + 1));
    ctx.client().withdraw(&id);

    let matching = ctx
        .env
        .events_matching((soroban_sdk::symbol_short!("withdrew"),));
    assert!(
        !matching.is_empty(),
        "expected withdrew event to be emitted"
    );
}
