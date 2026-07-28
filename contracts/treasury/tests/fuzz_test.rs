#![cfg(test)]

use proptest::prelude::*;
use soroban_sdk::{
    testutils::Address as _, token, Address, Env, Vec,
};
use treasury::Treasury;

fn create_token(env: &Env) -> (Address, Address) {
    let admin = Address::generate(env);
    let token_wasm = soroban_sdk::token::StellarAssetClient::new(
        env,
        &env.register_stellar_asset_contract_v2(admin.clone())
            .address(),
    );
    (token_wasm.address.clone(), admin)
}

fn deploy_treasury(env: &Env) -> (Address, Address, Address) {
    let admin1 = Address::generate(env);
    let admin2 = Address::generate(env);
    let treasury_id = env.register(Treasury, ());
    let client = treasury::TreasuryClient::new(env, &treasury_id);
    let mut admins = Vec::new(env);
    admins.push_back(admin1.clone());
    admins.push_back(admin2.clone());
    client.initialize(&admins, &2);
    (treasury_id, admin1, admin2)
}

proptest! {
    #![proptest_config(ProptestConfig::with_cases(20))]

    #[test]
    fn fuzz_deposit_and_withdraw_invariants(
        deposit_amount in 1..10_000_000i128,
        withdraw_ratio in 0.0..1.0f64,
    ) {
        let env = Env::default();
        env.mock_all_auths();

        let (treasury_id, admin1, admin2) = deploy_treasury(&env);
        let client = treasury::TreasuryClient::new(&env, &treasury_id);

        let (token_addr, _) = create_token(&env);
        let sac = token::StellarAssetClient::new(&env, &token_addr);
        sac.mint(&admin1, &deposit_amount);

        // Perform fuzz deposit
        client.deposit(&admin1, &token_addr, &deposit_amount);

        // Assert balance after deposit equals deposit_amount
        let bal_after_deposit = client.balance_of(&treasury_id, &token_addr);
        prop_assert_eq!(bal_after_deposit, deposit_amount);

        // Calculate withdrawal amount based on ratio
        let withdraw_amount = (deposit_amount as f64 * withdraw_ratio) as i128;
        let mut signers = Vec::new(&env);
        signers.push_back(admin1.clone());
        signers.push_back(admin2.clone());

        client.withdraw(&admin1, &token_addr, &withdraw_amount, &signers);

        // Assert remaining balance invariant
        let expected_remaining = deposit_amount - withdraw_amount;
        let actual_remaining = client.balance_of(&treasury_id, &token_addr);
        prop_assert_eq!(actual_remaining, expected_remaining);
        prop_assert!(actual_remaining >= 0);
    }
}
