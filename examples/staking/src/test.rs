#![cfg(test)]
extern crate std;

use crucible::assert_reverts;
use crucible::prelude::*;

use crate::{Staking, StakingClient};

const STAKE_AMOUNT: i128 = 1_000_000;

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

struct Ctx {
    pub env: MockEnv,
    pub id: soroban_sdk::Address,
    pub alice: AccountHandle,
    pub bob: AccountHandle,
    pub charlie: AccountHandle,
    pub token: MockToken,
}

impl Ctx {
    fn setup() -> Self {
        let env = MockEnv::builder()
            .with_contract::<Staking>()
            .with_account("alice", Stroops::xlm(100))
            .with_account("bob", Stroops::xlm(100))
            .with_account("charlie", Stroops::xlm(100))
            .build();

        let id = env.contract_id::<Staking>();
        let alice = env.account("alice");
        let bob = env.account("bob");
        let charlie = env.account("charlie");

        let token = MockToken::new(&env, "STK", 7);
        token.mint(&alice, STAKE_AMOUNT * 3);
        token.mint(&bob, STAKE_AMOUNT * 3);

        env.with_mock_all_auths(|| {
            StakingClient::new(env.inner(), &id).initialize(&alice, &token.address());
        });

        Ctx {
            env,
            id,
            alice,
            bob,
            charlie,
            token,
        }
    }

    fn client(&self) -> StakingClient<'_> {
        StakingClient::new(self.env.inner(), &self.id)
    }
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[test]
fn test_stake_transfers_tokens_to_contract() {
    let ctx = Ctx::setup();
    ctx.env.with_mock_all_auths(|| ctx.client().stake(&ctx.alice.address(), &STAKE_AMOUNT, &None));

    assert_eq!(ctx.token.balance(&ctx.id), STAKE_AMOUNT);
    assert_eq!(ctx.token.balance(&ctx.alice), STAKE_AMOUNT * 2);
}

#[test]
fn test_stake_self_delegation_by_default() {
    let ctx = Ctx::setup();
    ctx.env.with_mock_all_auths(|| ctx.client().stake(&ctx.alice.address(), &STAKE_AMOUNT, &None));

    // Without explicit delegate, voting power goes to self.
    assert_eq!(ctx.client().voting_power(&ctx.alice), STAKE_AMOUNT);
}

#[test]
fn test_stake_with_explicit_delegate() {
    let ctx = Ctx::setup();
    ctx.env.mock_all_auths();
    ctx.client()
        .stake(&ctx.alice.address(), &STAKE_AMOUNT, &Some(ctx.bob.address()));

    assert_eq!(ctx.client().voting_power(&ctx.bob), STAKE_AMOUNT);
    assert_eq!(ctx.client().voting_power(&ctx.alice), 0);
}

#[test]
fn test_delegate_changes_voting_power() {
    let ctx = Ctx::setup();
    ctx.env.with_mock_all_auths(|| ctx.client().stake(&ctx.alice.address(), &STAKE_AMOUNT, &None));

    // Alice delegates to Bob.
    ctx.client().delegate(&ctx.alice.address(), &ctx.bob);

    assert_eq!(ctx.client().voting_power(&ctx.alice), 0);
    assert_eq!(ctx.client().voting_power(&ctx.bob), STAKE_AMOUNT);
}

#[test]
fn test_delegate_then_redelegate() {
    let ctx = Ctx::setup();
    {
        let _auth = ctx.env.mock_all_auths_scoped();
        ctx.client().stake(&ctx.alice.address(), &STAKE_AMOUNT, &None);
        ctx.client().delegate(&ctx.alice.address(), &ctx.bob);
        ctx.client().delegate(&ctx.alice.address(), &ctx.charlie);
    }

    assert_eq!(ctx.client().voting_power(&ctx.alice), 0);
    assert_eq!(ctx.client().voting_power(&ctx.bob), 0);
    assert_eq!(ctx.client().voting_power(&ctx.charlie), STAKE_AMOUNT);
}

#[test]
fn test_unstake_returns_tokens() {
    let ctx = Ctx::setup();
    ctx.env.with_mock_all_auths(|| ctx.client().stake(&ctx.alice.address(), &STAKE_AMOUNT, &None));
    ctx.client().unstake(&ctx.alice);

    assert_eq!(ctx.token.balance(&ctx.alice), STAKE_AMOUNT * 3);
    assert_eq!(ctx.token.balance(&ctx.id), 0);
}

#[test]
fn test_unstake_removes_voting_power() {
    let ctx = Ctx::setup();
    ctx.env.with_mock_all_auths(|| ctx.client().stake(&ctx.alice.address(), &STAKE_AMOUNT, &None));
    ctx.client().unstake(&ctx.alice);

    assert_eq!(ctx.client().voting_power(&ctx.alice), 0);
}

#[test]
fn test_unstake_removes_delegated_voting_power() {
    let ctx = Ctx::setup();
    ctx.env.mock_all_auths();
    ctx.client()
        .stake(&ctx.alice.address(), &STAKE_AMOUNT, &Some(ctx.bob.address()));
    ctx.client().unstake(&ctx.alice);

    assert_eq!(ctx.client().voting_power(&ctx.bob), 0);
}

#[test]
fn test_multiple_stakers_accumulate_voting_power() {
    let ctx = Ctx::setup();
    {
        let _auth = ctx.env.mock_all_auths_scoped();
        // Both alice and bob delegate to charlie.
        ctx.client()
            .stake(&ctx.alice.address(), &STAKE_AMOUNT, &Some(ctx.charlie.address()));
        ctx.client()
            .stake(&ctx.bob.address(), &STAKE_AMOUNT, &Some(ctx.charlie.address()));
    }

    assert_eq!(ctx.client().voting_power(&ctx.charlie), STAKE_AMOUNT * 2);
}

#[test]
fn test_stake_zero_amount_reverts() {
    let ctx = Ctx::setup();
    ctx.env.mock_all_auths();
    assert_reverts!(ctx.client().stake(&ctx.alice.address(), &0_i128, &None), "positive");
}

#[test]
fn test_unstake_without_stake_reverts() {
    let ctx = Ctx::setup();
    ctx.env.mock_all_auths();
    assert_reverts!(ctx.client().unstake(&ctx.alice), "no stake found");
}

#[test]
fn test_delegate_without_stake_reverts() {
    let ctx = Ctx::setup();
    ctx.env.mock_all_auths();
    assert_reverts!(
        ctx.client().delegate(&ctx.alice.address(), &ctx.bob),
        "no stake found"
    );
}

#[test]
fn test_stake_emits_event() {
    let ctx = Ctx::setup();
    ctx.env.with_mock_all_auths(|| ctx.client().stake(&ctx.alice.address(), &STAKE_AMOUNT, &None));
    // Verify the staked event is present (alongside the SAC transfer event).
    let matching = ctx
        .env
        .events_matching((soroban_sdk::symbol_short!("staked"),));
    assert!(!matching.is_empty(), "expected staked event to be emitted");
}

#[test]
fn test_unstake_emits_event() {
    let ctx = Ctx::setup();
    {
        let _auth = ctx.env.mock_all_auths_scoped();
        ctx.client().stake(&ctx.alice.address(), &STAKE_AMOUNT, &None);
        ctx.client().unstake(&ctx.alice);
    }
    let matching = ctx
        .env
        .events_matching((soroban_sdk::symbol_short!("unstaked"),));
    assert!(
        !matching.is_empty(),
        "expected unstaked event to be emitted"
    );
}

#[test]
fn test_delegate_emits_event() {
    let ctx = Ctx::setup();
    {
        let _auth = ctx.env.mock_all_auths_scoped();
        ctx.client().stake(&ctx.alice.address(), &STAKE_AMOUNT, &None);
        ctx.client().delegate(&ctx.alice.address(), &ctx.bob);
    }
    let matching = ctx
        .env
        .events_matching((soroban_sdk::symbol_short!("delegated"),));
    assert!(
        !matching.is_empty(),
        "expected delegated event to be emitted"
    );
}

#[test]
fn test_get_stake_returns_correct_info() {
    let ctx = Ctx::setup();
    ctx.env.with_mock_all_auths(|| {
        ctx.client()
            .stake(&ctx.alice.address(), &STAKE_AMOUNT, &Some(ctx.bob.address()));
    });

    let info = ctx.client().get_stake(&ctx.alice);
    assert_eq!(info.amount, STAKE_AMOUNT);
    assert_eq!(info.delegate, Some(ctx.bob.address()));
}

#[test]
fn test_get_stake_returns_zero_for_unknown() {
    let ctx = Ctx::setup();
    let info = ctx.client().get_stake(&ctx.charlie);
    assert_eq!(info.amount, 0);
    assert_eq!(info.delegate, None);
}

#[test]
fn test_additional_stake_accumulates() {
    let ctx = Ctx::setup();
    {
        let _auth = ctx.env.mock_all_auths_scoped();
        ctx.client().stake(&ctx.alice.address(), &STAKE_AMOUNT, &None);
        ctx.client().stake(&ctx.alice.address(), &STAKE_AMOUNT, &None);
    }

    let info = ctx.client().get_stake(&ctx.alice);
    assert_eq!(info.amount, STAKE_AMOUNT * 2);
    assert_eq!(ctx.client().voting_power(&ctx.alice), STAKE_AMOUNT * 2);
}
