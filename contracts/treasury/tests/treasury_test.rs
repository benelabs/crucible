#![cfg(test)]

use soroban_sdk::{
    testutils::{Address as _, AuthorizedFunction, AuthorizedInvocation, MockAuth, MockAuthInvoke},
    token, Address, Env, IntoVal, Symbol, Vec,
};
use treasury::Treasury;

/// Deploy a simple SAC-compatible token and return (token_address, admin).
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

// ─── Tests ───────────────────────────────────────────────────────────────────

#[test]
fn test_deposit_and_balance() {
    let env = Env::default();
    env.mock_all_auths();

    let (treasury_id, admin1, _admin2) = deploy_treasury(&env);
    let client = treasury::TreasuryClient::new(&env, &treasury_id);

    let (token_addr, token_admin) = create_token(&env);
    let sac = token::StellarAssetClient::new(&env, &token_addr);
    sac.mint(&admin1, &1_000);

    client.deposit(&admin1, &token_addr, &1_000);

    assert_eq!(client.balance_of(&treasury_id, &token_addr), 1_000);
}

#[test]
fn test_successful_withdraw_with_quorum_auth() {
    let env = Env::default();
    env.mock_all_auths();

    let (treasury_id, admin1, admin2) = deploy_treasury(&env);
    let client = treasury::TreasuryClient::new(&env, &treasury_id);

    let (token_addr, _) = create_token(&env);
    let sac = token::StellarAssetClient::new(&env, &token_addr);
    sac.mint(&admin1, &1_000);

    client.deposit(&admin1, &token_addr, &1_000);

    let mut signers = Vec::new(&env);
    signers.push_back(admin1.clone());
    signers.push_back(admin2.clone());

    client.withdraw(&admin1, &token_addr, &400, &signers);

    assert_eq!(client.balance_of(&treasury_id, &token_addr), 600);
}

#[test]
#[should_panic]
fn test_withdraw_fails_below_quorum() {
    let env = Env::default();
    env.mock_all_auths();

    let (treasury_id, admin1, _admin2) = deploy_treasury(&env);
    let client = treasury::TreasuryClient::new(&env, &treasury_id);

    let (token_addr, _) = create_token(&env);
    let sac = token::StellarAssetClient::new(&env, &token_addr);
    sac.mint(&admin1, &1_000);
    client.deposit(&admin1, &token_addr, &1_000);

    // only one signer — quorum requires 2
    let mut signers = Vec::new(&env);
    signers.push_back(admin1.clone());
    client.withdraw(&admin1, &token_addr, &500, &signers);
}

/// Passing admin addresses without their authorization must be rejected.
#[test]
#[should_panic]
fn test_withdraw_fails_without_signer_auth() {
    let env = Env::default();
    // Do NOT call mock_all_auths — we want real auth checks.

    let (treasury_id, admin1, admin2) = deploy_treasury(&env);

    // We need the deposit to work, so mock auth just for setup.
    env.mock_all_auths();
    let client = treasury::TreasuryClient::new(&env, &treasury_id);
    let (token_addr, _) = create_token(&env);
    let sac = token::StellarAssetClient::new(&env, &token_addr);
    sac.mint(&admin1, &1_000);
    client.deposit(&admin1, &token_addr, &1_000);
    env.mock_auths(&[]); // clear mock auths — subsequent calls need real auth

    // Pass both admins as signers but provide NO authorization entries.
    // require_auth() inside withdraw should panic.
    let mut signers = Vec::new(&env);
    signers.push_back(admin1.clone());
    signers.push_back(admin2.clone());
    client.withdraw(&admin1, &token_addr, &500, &signers);
}

/// Non-admin addresses in the signer list must not count toward quorum.
#[test]
#[should_panic]
fn test_withdraw_fails_with_non_admin_signers() {
    let env = Env::default();
    env.mock_all_auths();

    let (treasury_id, _admin1, _admin2) = deploy_treasury(&env);
    let client = treasury::TreasuryClient::new(&env, &treasury_id);

    let (token_addr, _) = create_token(&env);
    let sac = token::StellarAssetClient::new(&env, &token_addr);
    let depositor = Address::generate(&env);
    sac.mint(&depositor, &1_000);
    client.deposit(&depositor, &token_addr, &1_000);

    // Two non-admin addresses — auth will succeed (mock_all_auths) but quorum check must fail.
    let rando1 = Address::generate(&env);
    let rando2 = Address::generate(&env);
    let mut signers = Vec::new(&env);
    signers.push_back(rando1.clone());
    signers.push_back(rando2.clone());
    client.withdraw(&depositor, &token_addr, &500, &signers);
}
