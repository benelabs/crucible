#![cfg(test)]
extern crate std;

use crucible::assert_reverts;
use crucible::prelude::*;

use crate::{make_meta_tx, Gasless, GaslessClient, MetaTx};

const AMOUNT: i128 = 1_000_000;
const BASE_TIME: u64 = 1_000_000;
const DEADLINE: u64 = BASE_TIME + 3_600; // 1 hour from now

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

struct Ctx {
    pub env: MockEnv,
    pub id: soroban_sdk::Address,
    pub relayer: AccountHandle,
    pub alice: AccountHandle,
    pub bob: AccountHandle,
    pub token: MockToken,
}

impl Ctx {
    fn setup() -> Self {
        let env = MockEnv::builder()
            .at_timestamp(BASE_TIME)
            .with_contract::<Gasless>()
            .with_account("relayer", Stroops::xlm(100))
            .with_account("alice", Stroops::xlm(100))
            .with_account("bob", Stroops::xlm(100))
            .build();

        let id = env.contract_id::<Gasless>();
        let relayer = env.account("relayer");
        let alice = env.account("alice");
        let bob = env.account("bob");

        let token = MockToken::new(&env, "USDC", 6);
        token.mint(&alice, AMOUNT * 5);

        env.mock_all_auths();
        GaslessClient::new(env.inner(), &id).initialize(&relayer);

        Ctx {
            env,
            id,
            relayer,
            alice,
            bob,
            token,
        }
    }

    fn client(&self) -> GaslessClient<'_> {
        GaslessClient::new(self.env.inner(), &self.id)
    }

    fn meta_tx(&self, nonce: u64) -> MetaTx {
        make_meta_tx(
            self.env.inner(),
            self.alice.clone(),
            self.bob.clone(),
            self.token.address(),
            AMOUNT,
            nonce,
            DEADLINE,
        )
    }
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[test]
fn test_execute_transfers_tokens() {
    let ctx = Ctx::setup();
    ctx.env.mock_all_auths();
    ctx.client().execute(&ctx.relayer, &ctx.meta_tx(0));

    assert_eq!(ctx.token.balance(&ctx.alice), AMOUNT * 4);
    assert_eq!(ctx.token.balance(&ctx.bob), AMOUNT);
}

#[test]
fn test_nonce_increments_after_execute() {
    let ctx = Ctx::setup();
    ctx.env.mock_all_auths();
    assert_eq!(ctx.client().nonce(&ctx.alice), 0);
    ctx.client().execute(&ctx.relayer, &ctx.meta_tx(0));
    assert_eq!(ctx.client().nonce(&ctx.alice), 1);
}

#[test]
fn test_replay_attack_reverts() {
    let ctx = Ctx::setup();
    ctx.env.mock_all_auths();
    ctx.client().execute(&ctx.relayer, &ctx.meta_tx(0));
    // Replay with same nonce.
    assert_reverts!(ctx.client().execute(&ctx.relayer, &ctx.meta_tx(0)), "nonce");
}

#[test]
fn test_sequential_nonces_succeed() {
    let ctx = Ctx::setup();
    ctx.env.mock_all_auths();
    ctx.client().execute(&ctx.relayer, &ctx.meta_tx(0));
    ctx.client().execute(&ctx.relayer, &ctx.meta_tx(1));
    assert_eq!(ctx.token.balance(&ctx.bob), AMOUNT * 2);
}

#[test]
fn test_expired_meta_tx_reverts() {
    let ctx = Ctx::setup();
    ctx.env.mock_all_auths();
    // Advance time past the deadline.
    ctx.env.advance_time(Duration::seconds(3_601));
    assert_reverts!(
        ctx.client().execute(&ctx.relayer, &ctx.meta_tx(0)),
        "expired"
    );
}

#[test]
fn test_unauthorized_relayer_reverts() {
    let ctx = Ctx::setup();
    ctx.env.mock_all_auths();
    // alice tries to act as relayer.
    assert_reverts!(
        ctx.client().execute(&ctx.alice, &ctx.meta_tx(0)),
        "unauthorized relayer"
    );
}

#[test]
fn test_wrong_nonce_reverts() {
    let ctx = Ctx::setup();
    ctx.env.mock_all_auths();
    // Nonce 1 is wrong when 0 is expected.
    assert_reverts!(ctx.client().execute(&ctx.relayer, &ctx.meta_tx(1)), "nonce");
}

#[test]
fn test_relayer_returns_correct_address() {
    let ctx = Ctx::setup();
    assert_eq!(ctx.client().relayer(), ctx.relayer.clone());
}

#[test]
fn test_nonce_starts_at_zero() {
    let ctx = Ctx::setup();
    assert_eq!(ctx.client().nonce(&ctx.alice), 0);
    assert_eq!(ctx.client().nonce(&ctx.bob), 0);
}

#[test]
fn test_execute_emits_event() {
    let ctx = Ctx::setup();
    ctx.env.mock_all_auths();
    ctx.client().execute(&ctx.relayer, &ctx.meta_tx(0));
    let matching = ctx
        .env
        .events_matching((soroban_sdk::symbol_short!("executed"),));
    assert!(
        !matching.is_empty(),
        "expected executed event to be emitted"
    );
}

#[test]
fn test_multiple_users_independent_nonces() {
    let ctx = Ctx::setup();
    ctx.env.mock_all_auths();

    // Give bob some tokens too.
    ctx.token.mint(&ctx.bob, AMOUNT * 5);

    // alice executes nonce 0.
    ctx.client().execute(&ctx.relayer, &ctx.meta_tx(0));

    // bob's nonce is still 0 independently.
    let bob_tx = make_meta_tx(
        ctx.env.inner(),
        ctx.bob.clone(),
        ctx.alice.clone(),
        ctx.token.address(),
        AMOUNT,
        0,
        DEADLINE,
    );
    ctx.client().execute(&ctx.relayer, &bob_tx);

    assert_eq!(ctx.client().nonce(&ctx.alice), 1);
    assert_eq!(ctx.client().nonce(&ctx.bob), 1);
}
