#![cfg(test)]
extern crate std;

use crucible::prelude::*;
use crucible::{assert_emitted, assert_reverts};
use soroban_sdk::symbol_short;

use crate::{ReputationContract, ReputationContractClient};

struct Ctx {
    env: MockEnv,
    id: soroban_sdk::Address,
    admin: AccountHandle,
    alice: AccountHandle,
    bob: AccountHandle,
    carol: AccountHandle,
}

impl Ctx {
    fn setup() -> Self {
        let env = MockEnv::builder()
            .with_contract::<ReputationContract>()
            .with_account("admin", Stroops::xlm(10))
            .with_account("alice", Stroops::xlm(10))
            .with_account("bob", Stroops::xlm(10))
            .with_account("carol", Stroops::xlm(10))
            .build();

        let id = env.contract_id::<ReputationContract>();
        let admin = env.account("admin");
        let alice = env.account("alice");
        let bob = env.account("bob");
        let carol = env.account("carol");

        env.mock_all_auths();
        ReputationContractClient::new(env.inner(), &id).initialize(&admin);

        Ctx { env, id, admin, alice, bob, carol }
    }

    fn client(&self) -> ReputationContractClient<'_> {
        ReputationContractClient::new(self.env.inner(), &self.id)
    }
}

#[test]
fn test_initial_reputation_is_zero() {
    let ctx = Ctx::setup();
    let rep = ctx.client().reputation(&ctx.alice);
    assert_eq!(rep.score, 0);
    assert_eq!(rep.endorsements, 0);
    assert_eq!(rep.flags, 0);
}

#[test]
fn test_endorse_increases_score() {
    let ctx = Ctx::setup();
    ctx.env.mock_all_auths();
    ctx.client().endorse(&ctx.alice, &ctx.bob);

    let rep = ctx.client().reputation(&ctx.bob);
    assert_eq!(rep.score, 1);
    assert_eq!(rep.endorsements, 1);
    assert_eq!(rep.flags, 0);
}

#[test]
fn test_flag_decreases_score() {
    let ctx = Ctx::setup();
    ctx.env.mock_all_auths();
    ctx.client().flag(&ctx.alice, &ctx.bob);

    let rep = ctx.client().reputation(&ctx.bob);
    assert_eq!(rep.score, -1);
    assert_eq!(rep.endorsements, 0);
    assert_eq!(rep.flags, 1);
}

#[test]
fn test_multiple_endorsements_accumulate() {
    let ctx = Ctx::setup();
    ctx.env.mock_all_auths();
    ctx.client().endorse(&ctx.alice, &ctx.bob);
    ctx.client().endorse(&ctx.carol, &ctx.bob);

    let rep = ctx.client().reputation(&ctx.bob);
    assert_eq!(rep.score, 2);
    assert_eq!(rep.endorsements, 2);
}

#[test]
fn test_mixed_endorsements_and_flags() {
    let ctx = Ctx::setup();
    ctx.env.mock_all_auths();
    ctx.client().endorse(&ctx.alice, &ctx.bob);
    ctx.client().endorse(&ctx.carol, &ctx.bob);
    ctx.client().flag(&ctx.admin, &ctx.bob);

    let rep = ctx.client().reputation(&ctx.bob);
    assert_eq!(rep.score, 1);
    assert_eq!(rep.endorsements, 2);
    assert_eq!(rep.flags, 1);
}

#[test]
fn test_double_endorse_reverts() {
    let ctx = Ctx::setup();
    ctx.env.mock_all_auths();
    ctx.client().endorse(&ctx.alice, &ctx.bob);
    assert_reverts!(ctx.client().endorse(&ctx.alice, &ctx.bob), "already endorsed");
}

#[test]
fn test_double_flag_reverts() {
    let ctx = Ctx::setup();
    ctx.env.mock_all_auths();
    ctx.client().flag(&ctx.alice, &ctx.bob);
    assert_reverts!(ctx.client().flag(&ctx.alice, &ctx.bob), "already flagged");
}

#[test]
fn test_self_endorse_reverts() {
    let ctx = Ctx::setup();
    ctx.env.mock_all_auths();
    assert_reverts!(ctx.client().endorse(&ctx.alice, &ctx.alice), "self");
}

#[test]
fn test_self_flag_reverts() {
    let ctx = Ctx::setup();
    ctx.env.mock_all_auths();
    assert_reverts!(ctx.client().flag(&ctx.alice, &ctx.alice), "self");
}

#[test]
fn test_endorse_emits_event() {
    let ctx = Ctx::setup();
    ctx.env.mock_all_auths();
    ctx.client().endorse(&ctx.alice, &ctx.bob);
    assert_emitted!(
        ctx.env,
        ctx.id,
        (symbol_short!("endorse"),),
        (ctx.alice.address(), ctx.bob.address())
    );
}

#[test]
fn test_flag_emits_event() {
    let ctx = Ctx::setup();
    ctx.env.mock_all_auths();
    ctx.client().flag(&ctx.alice, &ctx.bob);
    assert_emitted!(
        ctx.env,
        ctx.id,
        (symbol_short!("flag"),),
        (ctx.alice.address(), ctx.bob.address())
    );
}

#[test]
fn test_admin_revoke_resets_reputation() {
    let ctx = Ctx::setup();
    ctx.env.mock_all_auths();
    ctx.client().endorse(&ctx.alice, &ctx.bob);
    ctx.client().endorse(&ctx.carol, &ctx.bob);
    assert_eq!(ctx.client().reputation(&ctx.bob).score, 2);

    ctx.client().revoke(&ctx.bob);
    assert_eq!(ctx.client().reputation(&ctx.bob).score, 0);
}

#[test]
fn test_double_initialize_reverts() {
    let ctx = Ctx::setup();
    ctx.env.mock_all_auths();
    assert_reverts!(ctx.client().initialize(&ctx.admin), "already initialized");
}
