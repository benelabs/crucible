#![cfg(test)]
extern crate std;

use crucible::assert_emitted;
use crucible::prelude::*;
use soroban_sdk::{symbol_short, Address};

use crate::{Aggregator, AggregatorClient, Counter, CounterClient, Router, RouterClient};

// ---------------------------------------------------------------------------
// Shared fixture
// ---------------------------------------------------------------------------

struct Ctx {
    env: MockEnv,
    counter_id: Address,
    router_id: Address,
    agg_id: Address,
    token: MockToken,
    alice: AccountHandle,
    bob: AccountHandle,
}

impl Ctx {
    fn setup() -> Self {
        let env = MockEnv::builder()
            .with_contract::<Counter>()
            .with_contract::<Router>()
            .with_contract::<Aggregator>()
            .with_account("alice", Stroops::xlm(100))
            .with_account("bob", Stroops::xlm(10))
            .build();

        let counter_id = env.contract_id::<Counter>();
        let router_id = env.contract_id::<Router>();
        let agg_id = env.contract_id::<Aggregator>();
        let token = MockToken::new(&env, "USDC", 6);
        let alice = env.account("alice");
        let bob = env.account("bob");

        // Wire up: Router knows Counter + Token; Aggregator knows Router.
        env.mock_all_auths();
        RouterClient::new(env.inner(), &router_id).initialize(&counter_id, &token.address());
        AggregatorClient::new(env.inner(), &agg_id).initialize(&router_id);

        Ctx {
            env,
            counter_id,
            router_id,
            agg_id,
            token,
            alice,
            bob,
        }
    }

    fn counter(&self) -> CounterClient<'_> {
        CounterClient::new(self.env.inner(), &self.counter_id)
    }

    fn router(&self) -> RouterClient<'_> {
        RouterClient::new(self.env.inner(), &self.router_id)
    }

    fn agg(&self) -> AggregatorClient<'_> {
        AggregatorClient::new(self.env.inner(), &self.agg_id)
    }
}

// ---------------------------------------------------------------------------
// Counter tests
// ---------------------------------------------------------------------------

#[test]
fn test_counter_starts_at_zero() {
    let ctx = Ctx::setup();
    assert_eq!(ctx.counter().get(), 0);
}

#[test]
fn test_counter_increment_returns_new_value() {
    let ctx = Ctx::setup();
    assert_eq!(ctx.counter().increment(), 1);
    assert_eq!(ctx.counter().increment(), 2);
    assert_eq!(ctx.counter().get(), 2);
}

#[test]
fn test_counter_increment_emits_event() {
    let ctx = Ctx::setup();
    ctx.counter().increment();
    assert_emitted!(ctx.env, ctx.counter_id, (symbol_short!("incr"),), 1_u32);
}

#[test]
fn test_counter_multiple_increments_emit_events() {
    let ctx = Ctx::setup();
    ctx.counter().increment();
    ctx.counter().increment();
    ctx.counter().increment();
    assert_eq!(ctx.counter().get(), 3);
}

// ---------------------------------------------------------------------------
// Router tests
// ---------------------------------------------------------------------------

#[test]
fn test_router_ping_counter_increments_counter() {
    let ctx = Ctx::setup();
    let value = ctx.router().ping_counter();
    assert_eq!(value, 1);
    // Counter state is updated cross-contract.
    assert_eq!(ctx.counter().get(), 1);
}

#[test]
fn test_router_ping_counter_emits_event() {
    let ctx = Ctx::setup();
    ctx.router().ping_counter();
    assert_emitted!(ctx.env, ctx.router_id, (symbol_short!("pinged"),), 1_u32);
}

#[test]
fn test_router_counter_value_reads_cross_contract() {
    let ctx = Ctx::setup();
    ctx.counter().increment();
    ctx.counter().increment();
    assert_eq!(ctx.router().counter_value(), 2);
}

#[test]
fn test_router_route_transfer_moves_tokens() {
    let ctx = Ctx::setup();
    ctx.token.mint(&ctx.alice.address(), 1_000_i128);

    ctx.env.mock_all_auths();
    ctx.router().route_transfer(&ctx.alice.address(), &ctx.bob.address(), &400_i128);

    assert_eq!(ctx.token.balance(&ctx.alice), 600_i128);
    assert_eq!(ctx.token.balance(&ctx.bob), 400_i128);
}

#[test]
fn test_router_route_transfer_emits_event() {
    let ctx = Ctx::setup();
    ctx.token.mint(&ctx.alice.address(), 500_i128);

    ctx.env.mock_all_auths();
    ctx.router().route_transfer(&ctx.alice.address(), &ctx.bob.address(), &500_i128);

    assert_emitted!(ctx.env, ctx.router_id, (symbol_short!("routed"),), 500_i128);
}

#[test]
fn test_router_initialize_twice_panics() {
    let ctx = Ctx::setup();
    // Router is already initialized in Ctx::setup(); calling again must panic.
    let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        ctx.router()
            .initialize(&ctx.counter_id.clone(), &ctx.token.address());
    }));
    assert!(result.is_err());
}

// ---------------------------------------------------------------------------
// Aggregator tests
// ---------------------------------------------------------------------------

#[test]
fn test_aggregator_ping_increments_counter_via_router() {
    let ctx = Ctx::setup();
    let value = ctx.agg().aggregate_ping();
    assert_eq!(value, 1);
    // Counter was incremented through the two-level chain.
    assert_eq!(ctx.counter().get(), 1);
}

#[test]
fn test_aggregator_ping_emits_event() {
    let ctx = Ctx::setup();
    ctx.agg().aggregate_ping();
    assert_emitted!(ctx.env, ctx.agg_id, (symbol_short!("aggping"),), 1_u32);
}

#[test]
fn test_aggregator_counter_value_reads_through_router() {
    let ctx = Ctx::setup();
    ctx.agg().aggregate_ping();
    ctx.agg().aggregate_ping();
    assert_eq!(ctx.agg().counter_value(), 2);
}

#[test]
fn test_aggregator_transfer_moves_tokens() {
    let ctx = Ctx::setup();
    ctx.token.mint(&ctx.alice.address(), 2_000_i128);

    ctx.env.mock_all_auths();
    ctx.agg()
        .aggregate_transfer(&ctx.alice.address(), &ctx.bob.address(), &1_000_i128);

    assert_eq!(ctx.token.balance(&ctx.alice), 1_000_i128);
    assert_eq!(ctx.token.balance(&ctx.bob), 1_000_i128);
}

#[test]
fn test_aggregator_transfer_tracks_total_routed() {
    let ctx = Ctx::setup();
    ctx.token.mint(&ctx.alice.address(), 3_000_i128);

    ctx.env.mock_all_auths();
    ctx.agg()
        .aggregate_transfer(&ctx.alice.address(), &ctx.bob.address(), &1_000_i128);
    ctx.agg()
        .aggregate_transfer(&ctx.alice.address(), &ctx.bob.address(), &500_i128);

    assert_eq!(ctx.agg().total_routed(), 1_500_i128);
}

#[test]
fn test_aggregator_transfer_emits_event_with_cumulative_total() {
    let ctx = Ctx::setup();
    ctx.token.mint(&ctx.alice.address(), 1_000_i128);

    ctx.env.mock_all_auths();
    ctx.agg()
        .aggregate_transfer(&ctx.alice.address(), &ctx.bob.address(), &1_000_i128);

    assert_emitted!(ctx.env, ctx.agg_id, (symbol_short!("aggtxfr"),), 1_000_i128);
}

#[test]
fn test_aggregator_total_routed_starts_at_zero() {
    let ctx = Ctx::setup();
    assert_eq!(ctx.agg().total_routed(), 0);
}

#[test]
fn test_aggregator_initialize_twice_panics() {
    let ctx = Ctx::setup();
    let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        ctx.agg().initialize(&ctx.router_id);
    }));
    assert!(result.is_err());
}

// ---------------------------------------------------------------------------
// Multi-step / integration tests
// ---------------------------------------------------------------------------

#[test]
fn test_full_chain_ping_and_transfer() {
    let ctx = Ctx::setup();
    ctx.token.mint(&ctx.alice.address(), 5_000_i128);

    ctx.env.mock_all_auths();

    // Ping three times through the full chain.
    ctx.agg().aggregate_ping();
    ctx.agg().aggregate_ping();
    ctx.agg().aggregate_ping();

    // Transfer twice.
    ctx.agg()
        .aggregate_transfer(&ctx.alice.address(), &ctx.bob.address(), &2_000_i128);
    ctx.agg()
        .aggregate_transfer(&ctx.alice.address(), &ctx.bob.address(), &1_000_i128);

    assert_eq!(ctx.counter().get(), 3);
    assert_eq!(ctx.agg().total_routed(), 3_000_i128);
    assert_eq!(ctx.token.balance(&ctx.alice), 2_000_i128);
    assert_eq!(ctx.token.balance(&ctx.bob), 3_000_i128);
}

#[test]
fn test_direct_counter_and_router_counter_agree() {
    let ctx = Ctx::setup();

    // Increment directly on Counter.
    ctx.counter().increment();
    // Increment via Router.
    ctx.router().ping_counter();

    // Both views should agree.
    assert_eq!(ctx.counter().get(), 2);
    assert_eq!(ctx.router().counter_value(), 2);
    assert_eq!(ctx.agg().counter_value(), 2);
}

#[test]
fn test_transfer_insufficient_balance_reverts() {
    let ctx = Ctx::setup();
    // Alice has no tokens — transfer must fail.
    ctx.env.mock_all_auths();
    let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        ctx.agg().aggregate_transfer(&ctx.alice.address(), &ctx.bob.address(), &1_i128);
    }));
    assert!(result.is_err());
    // Total routed must remain zero.
    assert_eq!(ctx.agg().total_routed(), 0);
}
