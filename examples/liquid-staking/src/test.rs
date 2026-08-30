#![cfg(test)]
extern crate std;

use std::time::Duration;

use crucible::prelude::*;
use soroban_sdk::Address;

use crate::{LiquidStaking, LiquidStakingClient};

const DEPOSIT: i128 = 1_000_000;
const COOLDOWN_SECS: u64 = 86_400;

struct Ctx {
    env: MockEnv,
    id: Address,
    alice: AccountHandle,
    bob: AccountHandle,
    admin: AccountHandle,
    token: MockToken,
}

impl Ctx {
    fn setup() -> Self {
        let env = MockEnv::builder()
            .at_timestamp(1_700_000_000)
            .with_contract::<LiquidStaking>()
            .with_account("admin", Stroops::xlm(100))
            .with_account("alice", Stroops::xlm(100))
            .with_account("bob", Stroops::xlm(100))
            .build();

        let id = env.contract_id::<LiquidStaking>();
        let admin = env.account("admin");
        let alice = env.account("alice");
        let bob = env.account("bob");
        let token = MockToken::new(&env, "XLM", 7);
        token.mint(&admin, DEPOSIT * 50);
        token.mint(&alice, DEPOSIT * 20);
        token.mint(&bob, DEPOSIT * 20);

        env.with_mock_all_auths(|| {
            LiquidStakingClient::new(env.inner(), &id).initialize(
                &admin,
                &token.address(),
                &COOLDOWN_SECS,
            );
        });

        Ctx {
            env,
            id,
            alice,
            bob,
            admin,
            token,
        }
    }

    fn client(&self) -> LiquidStakingClient<'_> {
        LiquidStakingClient::new(self.env.inner(), &self.id)
    }
}

#[test]
fn deposit_mints_sxlm_one_to_one_initially() {
    let ctx = Ctx::setup();
    ctx.env.mock_all_auths();

    let shares = ctx.client().deposit(&ctx.alice, &DEPOSIT);
    assert_eq!(shares, DEPOSIT);
    assert_eq!(ctx.client().balance_of(&ctx.alice), DEPOSIT);
    assert_eq!(ctx.client().total_pooled(), DEPOSIT);
    assert_eq!(ctx.client().exchange_rate(), 1_000_000_000);
}

#[test]
fn unbonding_queue_enforces_cooldown_before_withdraw() {
    let ctx = Ctx::setup();
    ctx.env.mock_all_auths();

    ctx.client().deposit(&ctx.alice, &DEPOSIT);
    let id = ctx.client().request_unbond(&ctx.alice, &DEPOSIT);
    let req = ctx.client().get_unbonding(&id);
    assert_eq!(req.assets, DEPOSIT);
    assert!(!req.claimed);

    // Still inside cooldown — withdraw must fail.
    let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        ctx.client().withdraw(&ctx.alice, &id);
    }));
    assert!(result.is_err());

    ctx.env.advance_time(Duration::from_secs(COOLDOWN_SECS));
    let withdrawn = ctx.client().withdraw(&ctx.alice, &id);
    assert_eq!(withdrawn, DEPOSIT);
    assert_eq!(ctx.token.balance(&ctx.alice), DEPOSIT * 20);
}

#[test]
fn rewards_accrue_into_exchange_rate() {
    let ctx = Ctx::setup();
    ctx.env.mock_all_auths();

    let shares = ctx.client().deposit(&ctx.alice, &DEPOSIT);
    let rate_before = ctx.client().exchange_rate();

    ctx.client().accrue_rewards(&ctx.admin, &500_000);

    let rate_after = ctx.client().exchange_rate();
    assert!(rate_after > rate_before);
    let assets = ctx.client().convert_to_assets(&shares);
    assert_eq!(assets, DEPOSIT + 500_000);
}

#[test]
fn multi_epoch_reward_compounding() {
    let ctx = Ctx::setup();
    ctx.env.mock_all_auths();

    // Alice and Bob deposit in epoch 0.
    let alice_shares = ctx.client().deposit(&ctx.alice, &DEPOSIT);
    let bob_shares = ctx.client().deposit(&ctx.bob, &DEPOSIT);
    assert_eq!(alice_shares, bob_shares);

    // Three reward epochs compound into the shared exchange rate.
    for epoch in 1..=3 {
        let reward = 100_000 * i128::from(epoch);
        ctx.client().accrue_rewards(&ctx.admin, &reward);
        ctx.env.advance_time(Duration::from_secs(86_400));
        ctx.env.advance_sequence(1);
    }

    let rate = ctx.client().exchange_rate();
    assert!(rate > 1_000_000_000);

    let alice_assets = ctx.client().convert_to_assets(&alice_shares);
    let bob_assets = ctx.client().convert_to_assets(&bob_shares);
    assert_eq!(alice_assets, bob_assets);
    // Total rewards = 100k + 200k + 300k = 600k split evenly.
    assert_eq!(alice_assets, DEPOSIT + 300_000);
    assert_eq!(ctx.client().total_pooled(), DEPOSIT * 2 + 600_000);
}

#[test]
fn unbond_uses_reward_boosted_exchange_rate() {
    let ctx = Ctx::setup();
    ctx.env.mock_all_auths();

    let shares = ctx.client().deposit(&ctx.alice, &DEPOSIT);
    ctx.client().accrue_rewards(&ctx.admin, &DEPOSIT);

    let id = ctx.client().request_unbond(&ctx.alice, &shares);
    let req = ctx.client().get_unbonding(&id);
    assert_eq!(req.assets, DEPOSIT * 2);

    ctx.env.advance_time(Duration::from_secs(COOLDOWN_SECS));
    let out = ctx.client().withdraw(&ctx.alice, &id);
    assert_eq!(out, DEPOSIT * 2);
}
