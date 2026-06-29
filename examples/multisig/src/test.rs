#![cfg(test)]
extern crate std;

use crucible::prelude::*;
use crucible::{assert_emitted, assert_reverts};
use soroban_sdk::symbol_short;

use crate::{MultiSig, MultiSigClient};

struct Ctx {
    pub env: MockEnv,
    pub id: Address,
    pub a: AccountHandle,
    pub b: AccountHandle,
    pub c: AccountHandle,
}

impl Ctx {
    fn setup() -> Self {
        let env = MockEnv::builder()
            .with_contract::<MultiSig>()
            .with_account("a", Stroops::xlm(100))
            .with_account("b", Stroops::xlm(100))
            .with_account("c", Stroops::xlm(100))
            .build();

        let id = env.contract_id::<MultiSig>();
        let a = env.account("a");
        let b = env.account("b");
        let c = env.account("c");

        Ctx { env, id, a, b, c }
    }

    fn client(&self) -> MultiSigClient<'_> {
        MultiSigClient::new(self.env.inner(), &self.id)
    }
}

#[test]
fn test_basic_flow() {
    let ctx = Ctx::setup();
    ctx.env.mock_all_auths();

    // initialize with owners a,b,c and threshold 2
    let owners = vec![ctx.a.address(), ctx.b.address(), ctx.c.address()];
    ctx.client().initialize(&owners, &2u32);
    assert_emitted!(ctx.env, ctx.id, (symbol_short!("initialized"),), ());

    // proposer a creates a proposal
    ctx.env.mock_all_auths();
    let tx = soroban_sdk::Bytes::from_array(&ctx.env.address(), &[1, 2, 3]);
    let id = ctx.client().propose(&ctx.a.address(), &tx);
    assert_emitted!(ctx.env, ctx.id, (symbol_short!("proposed"),), id);

    // approve by b
    ctx.env.mock_all_auths();
    ctx.client().approve(&ctx.b.address(), &id);
    assert_emitted!(ctx.env, ctx.id, (symbol_short!("approved"),), (id, ctx.b.address()));

    // execute should fail because threshold=2 and only 1 approval
    ctx.env.mock_all_auths();
    assert_reverts!(ctx.client().execute(&ctx.a.address(), &id), "not enough approvals");

    // approve by c
    ctx.env.mock_all_auths();
    ctx.client().approve(&ctx.c.address(), &id);

    // now execute should succeed
    ctx.env.mock_all_auths();
    ctx.client().execute(&ctx.a.address(), &id);
    assert_emitted!(ctx.env, ctx.id, (symbol_short!("executed"),), id);

    // double execute should revert
    ctx.env.mock_all_auths();
    assert_reverts!(ctx.client().execute(&ctx.a.address(), &id), "proposal already executed");
}
