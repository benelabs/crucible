#![cfg(test)]
extern crate std;

use crucible::prelude::*;
use crate::{PortfolioIndex, PortfolioIndexClient};

#[test]
fn test_portfolio_index_rebalance() {
    let env = MockEnv::builder()
        .at_timestamp(1_000_000)
        .with_contract::<PortfolioIndex>()
        .with_account("admin", Stroops::xlm(100))
        .with_account("investor", Stroops::xlm(100))
        .with_account("keeper", Stroops::xlm(10))
        .build();

    let contract_id = env.contract_id::<PortfolioIndex>();
    let admin = env.account("admin");
    let investor = env.account("investor");
    let keeper = env.account("keeper");

    let token_a = MockToken::new(&env, "TKNA", 6);
    let token_b = MockToken::new(&env, "TKNB", 6);

    token_a.mint(&investor, 1_000_000);
    token_b.mint(&investor, 1_000_000);

    let client = PortfolioIndexClient::new(env.inner(), &contract_id);

    // Initialize 50% / 50% index with 50 bps keeper reward
    client.initialize(
        &admin.address(),
        &token_a.address(),
        &token_b.address(),
        &5000,
        &5000,
        &50,
    );

    // Deposit 500,000 of TokenA and 500,000 of TokenB
    client.deposit(&investor.address(), &500_000, &500_000);

    // When prices are equal (100:100), drift is 0 -> rebalance returns false
    let rebalanced = client.rebalance(&keeper.address(), &100, &100);
    assert_eq!(rebalanced, false);

    // TokenA price surges (150:100) -> weight drifts to 60% vs 50% (drift = 1000 bps > 200 bps)
    let rebalanced_surge = client.rebalance(&keeper.address(), &150, &100);
    assert_eq!(rebalanced_surge, true);
    assert!(token_a.balance(&keeper) > 0);
}
