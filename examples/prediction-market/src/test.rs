#![cfg(test)]
extern crate std;

use crucible::prelude::*;
use crucible::{assert_emitted, assert_reverts};
use soroban_sdk::{symbol_short, Address};

use crate::{MarketStatus, Outcome, PredictionMarket, PredictionMarketClient};

const BASE_TIME: u64 = 1_000_000;
const CLOSE_DELAY: u64 = 86_400;
const ALICE_STAKE: i128 = 600_000;
const BOB_STAKE: i128 = 400_000;
const CAROL_STAKE: i128 = 500_000;

struct Ctx {
    pub env: MockEnv,
    pub id: Address,
    pub admin: AccountHandle,
    pub alice: AccountHandle,
    pub bob: AccountHandle,
    pub carol: AccountHandle,
    pub token: MockToken,
}

impl Ctx {
    fn setup() -> Self {
        let env = MockEnv::builder()
            .at_timestamp(BASE_TIME)
            .with_contract::<PredictionMarket>()
            .with_account("admin", Stroops::xlm(100))
            .with_account("alice", Stroops::xlm(10))
            .with_account("bob", Stroops::xlm(10))
            .with_account("carol", Stroops::xlm(10))
            .build();

        let id = env.contract_id::<PredictionMarket>();
        let admin = env.account("admin");
        let alice = env.account("alice");
        let bob = env.account("bob");
        let carol = env.account("carol");

        let token = MockToken::new(&env, "USDC", 6);
        token.mint(&alice, 2_000_000);
        token.mint(&bob, 2_000_000);
        token.mint(&carol, 2_000_000);

        Ctx {
            env,
            id,
            admin,
            alice,
            bob,
            carol,
            token,
        }
    }

    fn client(&self) -> PredictionMarketClient<'_> {
        PredictionMarketClient::new(self.env.inner(), &self.id)
    }

    fn initialize(&self) {
        self.env.mock_all_auths();
        self.client().initialize(
            &self.admin,
            &self.token.address(),
            &(BASE_TIME + CLOSE_DELAY),
        );
    }

    fn fund_market(&self) {
        self.initialize();
        self.client().buy(&self.alice, &Outcome::Yes, &ALICE_STAKE);
        self.client().buy(&self.bob, &Outcome::Yes, &BOB_STAKE);
        self.client().buy(&self.carol, &Outcome::No, &CAROL_STAKE);
    }

    fn resolve_yes(&self) {
        self.env.advance_time(Duration::seconds(CLOSE_DELAY));
        self.env.mock_all_auths();
        self.client().resolve(&self.admin, &Outcome::Yes);
    }
}

#[test]
fn test_initialize_sets_open_market_state() {
    let ctx = Ctx::setup();
    ctx.initialize();

    let state = ctx.client().get_state();
    assert_eq!(state.admin, ctx.admin.address());
    assert_eq!(state.token, ctx.token.address());
    assert_eq!(state.close_time, BASE_TIME + CLOSE_DELAY);
    assert_eq!(state.status, MarketStatus::Open);
    assert_eq!(state.yes_total, 0);
    assert_eq!(state.no_total, 0);
}

#[test]
fn test_initialize_rejects_past_close_time() {
    let ctx = Ctx::setup();
    ctx.env.mock_all_auths();
    assert_reverts!(
        ctx.client()
            .initialize(&ctx.admin.address(), &ctx.token.address(), &BASE_TIME),
        "close time"
    );
}

#[test]
fn test_double_initialize_reverts() {
    let ctx = Ctx::setup();
    ctx.initialize();
    assert_reverts!(
        ctx.client()
            .initialize(&ctx.admin.address(), &ctx.token.address(), &(BASE_TIME + CLOSE_DELAY)),
        "already initialized"
    );
}

#[test]
fn test_buy_transfers_collateral_and_tracks_position() {
    let ctx = Ctx::setup();
    ctx.initialize();
    ctx.env.mock_all_auths();
    ctx.client().buy(&ctx.alice.address(), &Outcome::Yes, &ALICE_STAKE);

    assert_eq!(
        ctx.client().position(&ctx.alice.address(), &Outcome::Yes),
        ALICE_STAKE
    );
    assert_eq!(ctx.client().pool_total(), ALICE_STAKE);
    assert_eq!(ctx.token.balance(&ctx.id), ALICE_STAKE);
    assert_eq!(ctx.token.balance(&ctx.alice), 2_000_000 - ALICE_STAKE);
}

#[test]
fn test_buy_accumulates_multiple_positions() {
    let ctx = Ctx::setup();
    ctx.initialize();
    ctx.env.mock_all_auths();
    ctx.client().buy(&ctx.alice.address(), &Outcome::Yes, &ALICE_STAKE);
    ctx.client().buy(&ctx.alice.address(), &Outcome::Yes, &BOB_STAKE);
    ctx.client().buy(&ctx.carol.address(), &Outcome::No, &CAROL_STAKE);

    let state = ctx.client().get_state();
    assert_eq!(ctx.client().position(&ctx.alice.address(), &Outcome::Yes), 1_000_000);
    assert_eq!(state.yes_total, 1_000_000);
    assert_eq!(state.no_total, CAROL_STAKE);
}

#[test]
fn test_buy_rejects_zero_amount() {
    let ctx = Ctx::setup();
    ctx.initialize();
    ctx.env.mock_all_auths();
    assert_reverts!(
        ctx.client().buy(&ctx.alice.address(), &Outcome::Yes, &0_i128),
        "positive"
    );
}

#[test]
fn test_buy_after_close_reverts() {
    let ctx = Ctx::setup();
    ctx.initialize();
    ctx.env.advance_time(Duration::seconds(CLOSE_DELAY));
    ctx.env.mock_all_auths();
    assert_reverts!(
        ctx.client().buy(&ctx.alice.address(), &Outcome::Yes, &ALICE_STAKE),
        "closed"
    );
}

#[test]
fn test_only_admin_can_resolve() {
    let ctx = Ctx::setup();
    ctx.fund_market();
    ctx.env.advance_time(Duration::seconds(CLOSE_DELAY));
    ctx.env.mock_all_auths();
    assert_reverts!(
        ctx.client().resolve(&ctx.alice.address(), &Outcome::Yes),
        "only the admin"
    );
}

#[test]
fn test_resolve_before_close_reverts() {
    let ctx = Ctx::setup();
    ctx.fund_market();
    ctx.env.mock_all_auths();
    assert_reverts!(
        ctx.client().resolve(&ctx.admin.address(), &Outcome::Yes),
        "still open"
    );
}

#[test]
fn test_resolve_sets_winning_outcome() {
    let ctx = Ctx::setup();
    ctx.fund_market();
    ctx.resolve_yes();

    let state = ctx.client().get_state();
    assert_eq!(state.status, MarketStatus::Resolved);
    assert_eq!(state.winning_outcome, Outcome::Yes);
}

#[test]
fn test_double_resolve_reverts() {
    let ctx = Ctx::setup();
    ctx.fund_market();
    ctx.resolve_yes();
    assert_reverts!(
        ctx.client().resolve(&ctx.admin.address(), &Outcome::No),
        "already resolved"
    );
}

#[test]
fn test_claim_before_resolution_reverts() {
    let ctx = Ctx::setup();
    ctx.fund_market();
    ctx.env.mock_all_auths();
    assert_reverts!(ctx.client().claim(&ctx.alice), "not resolved");
}

#[test]
fn test_winners_claim_proportional_payouts() {
    let ctx = Ctx::setup();
    ctx.fund_market();
    ctx.resolve_yes();

    let pool = ALICE_STAKE + BOB_STAKE + CAROL_STAKE;
    let alice_expected = ALICE_STAKE * pool / (ALICE_STAKE + BOB_STAKE);
    let bob_expected = BOB_STAKE * pool / (ALICE_STAKE + BOB_STAKE);

    ctx.env.mock_all_auths();
    assert_eq!(ctx.client().claim(&ctx.alice), alice_expected);
    assert_eq!(ctx.client().claim(&ctx.bob), bob_expected);

    assert_eq!(
        ctx.token.balance(&ctx.alice),
        2_000_000 - ALICE_STAKE + alice_expected
    );
    assert_eq!(
        ctx.token.balance(&ctx.bob),
        2_000_000 - BOB_STAKE + bob_expected
    );
    assert_eq!(ctx.client().position(&ctx.alice.address(), &Outcome::Yes), 0);
    assert_eq!(ctx.client().position(&ctx.bob.address(), &Outcome::Yes), 0);
}

#[test]
fn test_losing_position_cannot_claim() {
    let ctx = Ctx::setup();
    ctx.fund_market();
    ctx.resolve_yes();
    ctx.env.mock_all_auths();
    assert_reverts!(ctx.client().claim(&ctx.carol), "no winning position");
}

#[test]
fn test_double_claim_reverts() {
    let ctx = Ctx::setup();
    ctx.fund_market();
    ctx.resolve_yes();
    ctx.env.mock_all_auths();
    ctx.client().claim(&ctx.alice);
    assert_reverts!(ctx.client().claim(&ctx.alice), "no winning position");
}

#[test]
fn test_no_side_can_win_and_claim_full_pool() {
    let ctx = Ctx::setup();
    ctx.fund_market();
    ctx.env.advance_time(Duration::seconds(CLOSE_DELAY));
    ctx.env.mock_all_auths();
    ctx.client().resolve(&ctx.admin.address(), &Outcome::No);

    let pool = ALICE_STAKE + BOB_STAKE + CAROL_STAKE;
    assert_eq!(ctx.client().claim(&ctx.carol), pool);
    assert_eq!(
        ctx.token.balance(&ctx.carol),
        2_000_000 - CAROL_STAKE + pool
    );
}

#[test]
fn test_initialize_emits_event() {
    let ctx = Ctx::setup();
    ctx.initialize();
    assert_emitted!(
        ctx.env,
        ctx.id,
        (symbol_short!("init"),),
        BASE_TIME + CLOSE_DELAY
    );
}
