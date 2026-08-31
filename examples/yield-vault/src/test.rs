#![cfg(test)]
extern crate std;

use crucible::prelude::*;

const AMOUNT: i128 = 100_000;

struct Ctx {
    env: MockEnv,
    id: soroban_sdk::Address,
    alice: AccountHandle,
    token: MockToken,
}

impl Ctx {
    fn setup() -> Self {
        let env = MockEnv::builder()
            .with_contract::<crate::YieldVault>()
            .with_account("alice", Stroops::xlm(10))
            .build();

        let id = env.contract_id::<crate::YieldVault>();
        let alice = env.account("alice");
        let token = MockToken::new(&env, "TOK", 7);
        token.mint(&alice, AMOUNT * 10);

        Ctx { env, id, alice, token }
    }

    fn client(&self) -> crate::YieldVaultClient<'_> {
        crate::YieldVaultClient::new(self.env.inner(), &self.id)
    }

    fn init_vault(&self) {
        self.env.mock_all_auths();
        self.client().initialize(&self.token.address());
    }
}

#[test]
fn test_initialize_mints_dead_shares() {
    let ctx = Ctx::setup();
    ctx.init_vault();
    let total_assets = ctx.client().total_assets();
    let total_shares = ctx.client().total_shares();
    assert_eq!(total_assets, 1_000);
    assert_eq!(total_shares, 1_000);
    let dead_shares = ctx.client().balance_of(&ctx.id);
    assert_eq!(dead_shares, 1_000);
}

#[test]
fn test_deposit_and_shares_minted() {
    let ctx = Ctx::setup();
    ctx.init_vault();
    ctx.env.mock_all_auths();

    let shares = ctx.client().deposit(&ctx.alice, &AMOUNT, &ctx.alice);
    assert!(shares > 0);
    assert_eq!(ctx.client().balance_of(&ctx.alice), shares);
}

#[test]
fn test_compound_yield_increases_share_value() {
    let ctx = Ctx::setup();
    ctx.init_vault();
    ctx.env.mock_all_auths();

    let alice_shares = ctx.client().deposit(&ctx.alice, &AMOUNT, &ctx.alice);
    
    // Simulate compounding yield from staking protocol
    ctx.client().compound_yield(&50_000);

    // Asset value per share has grown
    let assets_before = AMOUNT;
    let assets_after = ctx.client().convert_to_assets(&alice_shares);
    assert!(assets_after > assets_before);
}

#[test]
fn test_withdraw_and_redeem() {
    let ctx = Ctx::setup();
    ctx.init_vault();
    ctx.env.mock_all_auths();

    let shares = ctx.client().deposit(&ctx.alice, &AMOUNT, &ctx.alice);
    let initial_balance = ctx.token.balance(&ctx.alice);

    // Redeem half shares
    let redeemed_assets = ctx.client().redeem(&ctx.alice, &(shares / 2), &ctx.alice, &ctx.alice);
    assert!(redeemed_assets > 0);
    assert_eq!(ctx.token.balance(&ctx.alice), initial_balance + redeemed_assets);
}
