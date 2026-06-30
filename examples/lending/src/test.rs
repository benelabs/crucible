#![cfg(test)]
extern crate std;

use crucible::assert_reverts;
use crucible::prelude::*;
use soroban_sdk::Address;

use crate::{Lending, LendingClient};

const BASE_TIME: u64 = 1_000_000;
const LIQUIDITY: i128 = 1_000_000;
const COLLATERAL: i128 = 2_000_000;

struct Ctx {
    env: MockEnv,
    id: Address,
    admin: AccountHandle,
    lender: AccountHandle,
    borrower: AccountHandle,
    asset: MockToken,
    collateral: MockToken,
}

impl Ctx {
    fn setup() -> Self {
        let env = MockEnv::builder()
            .at_timestamp(BASE_TIME)
            .with_contract::<Lending>()
            .with_account("admin", Stroops::xlm(100))
            .with_account("lender", Stroops::xlm(100))
            .with_account("borrower", Stroops::xlm(100))
            .build();

        let id = env.contract_id::<Lending>();
        let admin = env.account("admin");
        let lender = env.account("lender");
        let borrower = env.account("borrower");
        let asset = MockToken::new(&env, "USDC", 6);
        let collateral = MockToken::new(&env, "XLM", 7);

        asset.mint(&lender, LIQUIDITY * 2);
        asset.mint(&borrower, LIQUIDITY);
        collateral.mint(&borrower, COLLATERAL);

        env.mock_all_auths();
        LendingClient::new(env.inner(), &id).initialize(
            &admin,
            &asset.address(),
            &collateral.address(),
            &500_i128,
            &1_500_i128,
            &7_500_i128,
        );

        Self {
            env,
            id,
            admin,
            lender,
            borrower,
            asset,
            collateral,
        }
    }

    fn client(&self) -> LendingClient<'_> {
        LendingClient::new(self.env.inner(), &self.id)
    }

    fn fund_pool_and_collateral(&self) {
        self.env.mock_all_auths();
        self.client().deposit(&self.lender, &LIQUIDITY);
        self.client()
            .deposit_collateral(&self.borrower, &COLLATERAL);
    }
}

#[test]
fn test_deposit_supplies_liquidity() {
    let ctx = Ctx::setup();
    ctx.env.mock_all_auths();
    ctx.client().deposit(&ctx.lender.address(), &LIQUIDITY);

    assert_eq!(ctx.asset.balance(&ctx.id), LIQUIDITY);
    assert_eq!(ctx.client().liquidity(), LIQUIDITY);
    assert_eq!(ctx.client().position(&ctx.lender).supplied, LIQUIDITY);
}

#[test]
fn test_deposit_emits_event() {
    let ctx = Ctx::setup();
    ctx.env.mock_all_auths();
    ctx.client().deposit(&ctx.lender.address(), &LIQUIDITY);

    let emitted = ctx.env.events_all().events().iter().any(|event| {
        let soroban_sdk::xdr::ContractEventBody::V0(body) = &event.body;
        body.topics.first().is_some_and(|topic| {
            topic == &soroban_sdk::xdr::ScVal::Symbol("deposit".try_into().unwrap())
        })
    });
    assert!(emitted, "deposit event should be emitted");
}

#[test]
fn test_borrow_transfers_liquidity_and_records_debt() {
    let ctx = Ctx::setup();
    ctx.fund_pool_and_collateral();

    ctx.env.mock_all_auths();
    ctx.client().borrow(&ctx.borrower.address(), &500_000_i128);

    let position = ctx.client().position(&ctx.borrower);
    assert_eq!(position.borrowed, 500_000);
    assert_eq!(position.collateral, COLLATERAL);
    assert_eq!(ctx.asset.balance(&ctx.borrower), LIQUIDITY + 500_000);
    assert_eq!(ctx.client().liquidity(), 500_000);
}

#[test]
fn test_borrow_above_collateral_factor_reverts() {
    let ctx = Ctx::setup();
    ctx.fund_pool_and_collateral();

    ctx.env.mock_all_auths();
    assert_reverts!(
        ctx.client().borrow(&ctx.borrower.address(), &1_600_000_i128),
        "insufficient pool liquidity"
    );
}

#[test]
fn test_borrow_without_enough_collateral_reverts() {
    let ctx = Ctx::setup();
    ctx.env.mock_all_auths();
    ctx.client().deposit(&ctx.lender.address(), &(LIQUIDITY * 2));
    ctx.client()
        .deposit_collateral(&ctx.borrower.address(), &100_000_i128);

    assert_reverts!(
        ctx.client().borrow(&ctx.borrower.address(), &100_000_i128),
        "insufficient collateral"
    );
}

#[test]
fn test_repay_caps_overpayment_to_current_debt() {
    let ctx = Ctx::setup();
    ctx.fund_pool_and_collateral();
    ctx.env.mock_all_auths();
    ctx.client().borrow(&ctx.borrower.address(), &400_000_i128);
    ctx.client().repay(&ctx.borrower.address(), &900_000_i128);

    assert_eq!(ctx.client().position(&ctx.borrower).borrowed, 0);
    assert_eq!(ctx.client().liquidity(), LIQUIDITY);
}

#[test]
fn test_interest_accrues_to_debt_and_supply() {
    let ctx = Ctx::setup();
    ctx.fund_pool_and_collateral();
    ctx.env.mock_all_auths();
    ctx.client().borrow(&ctx.borrower.address(), &500_000_i128);

    ctx.env.advance_time(Duration::days(365));

    let borrower = ctx.client().position(&ctx.borrower);
    let lender = ctx.client().position(&ctx.lender);

    assert_eq!(borrower.borrowed, 562_500);
    assert_eq!(lender.supplied, 1_062_500);
    assert_eq!(ctx.client().reserve().total_borrowed, 562_500);
}

#[test]
fn test_withdraw_respects_available_liquidity() {
    let ctx = Ctx::setup();
    ctx.fund_pool_and_collateral();
    ctx.env.mock_all_auths();
    ctx.client().borrow(&ctx.borrower.address(), &750_000_i128);

    assert_reverts!(
        ctx.client().withdraw(&ctx.lender.address(), &500_000_i128),
        "insufficient pool liquidity"
    );
}

#[test]
fn test_withdraw_reduces_supply_balance() {
    let ctx = Ctx::setup();
    ctx.fund_pool_and_collateral();
    ctx.env.mock_all_auths();
    ctx.client().withdraw(&ctx.lender.address(), &250_000_i128);

    assert_eq!(ctx.client().position(&ctx.lender).supplied, 750_000);
    assert_eq!(ctx.asset.balance(&ctx.lender), LIQUIDITY + 250_000);
}

#[test]
fn test_collateral_withdrawal_requires_healthy_position() {
    let ctx = Ctx::setup();
    ctx.fund_pool_and_collateral();
    ctx.env.mock_all_auths();
    ctx.client().borrow(&ctx.borrower.address(), &1_000_000_i128);

    assert_reverts!(
        ctx.client()
            .withdraw_collateral(&ctx.borrower.address(), &1_000_000_i128),
        "insufficient collateral"
    );
}

#[test]
fn test_collateral_can_be_withdrawn_after_repay() {
    let ctx = Ctx::setup();
    ctx.fund_pool_and_collateral();
    ctx.env.mock_all_auths();
    ctx.client().borrow(&ctx.borrower.address(), &500_000_i128);
    ctx.client().repay(&ctx.borrower.address(), &500_000_i128);
    ctx.client().withdraw_collateral(&ctx.borrower.address(), &COLLATERAL);

    assert_eq!(ctx.client().position(&ctx.borrower).collateral, 0);
    assert_eq!(ctx.collateral.balance(&ctx.borrower), COLLATERAL);
}

#[test]
fn test_invalid_initialization_reverts() {
    let ctx = Ctx::setup();
    let env = MockEnv::builder()
        .at_timestamp(BASE_TIME)
        .with_contract::<Lending>()
        .with_account("admin", Stroops::xlm(100))
        .build();
    let id = env.contract_id::<Lending>();
    let admin = env.account("admin");
    env.mock_all_auths();

    assert_reverts!(
        LendingClient::new(env.inner(), &id).initialize(
            &admin,
            &ctx.asset.address(),
            &ctx.collateral.address(),
            &10_001_i128,
            &0_i128,
            &7_500_i128,
        ),
        "base rate"
    );
}

#[test]
fn test_double_initialize_reverts() {
    let ctx = Ctx::setup();
    ctx.env.mock_all_auths();

    assert_reverts!(
        ctx.client().initialize(
            &ctx.admin.address(),
            &ctx.asset.address(),
            &ctx.collateral.address(),
            &500_i128,
            &1_500_i128,
            &7_500_i128,
        ),
        "already initialized"
    );
}
