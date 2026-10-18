#![cfg(test)]

use super::*;
use soroban_sdk::{testutils::Address as _, String as SorobanString, Address, Env};

#[test]
fn test_submit_price_whitelisted_source() {
    let env = Env::default();
    env.mock_all_auths();

    let contract_id = env.register_contract(None, Oracle);
    let client = OracleClient::new(&env, &contract_id);

    let admin = Address::generate(&env);
    client.initialize(&admin);

    let source_addr = Address::generate(&env);
    let source_name = SorobanString::from_str(&env, "binance");
    let _source_id = client.register_source(&source_addr, &source_name);

    let symbol = SorobanString::from_str(&env, "BTC/USD");
    let price = 50000_0000000i128;

    let res = client.try_submit_price(&source_addr, &symbol, &price, &source_name);
    assert!(res.is_ok());

    let fetched_price = client.get_price(&symbol);
    assert_eq!(fetched_price, price);
}

#[test]
fn test_submit_price_unwhitelisted_source_fails() {
    let env = Env::default();
    env.mock_all_auths();

    let contract_id = env.register_contract(None, Oracle);
    let client = OracleClient::new(&env, &contract_id);

    let admin = Address::generate(&env);
    client.initialize(&admin);

    let unwhitelisted_addr = Address::generate(&env);
    let symbol = SorobanString::from_str(&env, "ETH/USD");
    let price = 3000_0000000i128;
    let source_name = SorobanString::from_str(&env, "untrusted");

    let res = client.try_submit_price(&unwhitelisted_addr, &symbol, &price, &source_name);
    assert!(res.is_err());
}

#[test]
fn test_get_price_rejects_stale_feed() {
    let env = Env::default();
    env.mock_all_auths();

    let contract_id = env.register_contract(None, Oracle);
    let client = OracleClient::new(&env, &contract_id);

    let admin = Address::generate(&env);
    client.initialize(&admin);

    let source_addr = Address::generate(&env);
    let source_name = SorobanString::from_str(&env, "binance");
    let _source_id = client.register_source(&source_addr, &source_name);

    let symbol = SorobanString::from_str(&env, "BTC/USD");
    let price = 50000_0000000i128;
    client.submit_price(&source_addr, &symbol, &price, &source_name);

    // Advance past MAX_STALENESS_SECONDS (3600).
    env.ledger().set_timestamp(env.ledger().timestamp() + 3_601);

    let res = client.try_get_price(&symbol);
    assert!(res.is_err());
}

#[test]
fn test_deactivated_source_cannot_submit_and_purges_cached_price() {
    let env = Env::default();
    env.mock_all_auths();

    let contract_id = env.register_contract(None, Oracle);
    let client = OracleClient::new(&env, &contract_id);

    let admin = Address::generate(&env);
    client.initialize(&admin);

    let source_addr = Address::generate(&env);
    let source_name = SorobanString::from_str(&env, "binance");
    let source_id = client.register_source(&source_addr, &source_name);

    let symbol = SorobanString::from_str(&env, "BTC/USD");
    let price = 50000_0000000i128;
    client.submit_price(&source_addr, &symbol, &price, &source_name);
    assert_eq!(client.get_price(&symbol), price);

    // Deactivate source: should purge cached price and reject future submissions
    client.deactivate_source(&source_id);

    assert!(client.try_get_price(&symbol).is_err());
    assert!(client
        .try_submit_price(&source_addr, &symbol, &price, &source_name)
        .is_err());

    // Even if SourceWhitelist were still true, inactive DataSource must be rejected
    env.as_contract(&contract_id, || {
        env.storage()
            .instance()
            .set(&DataKey::SourceWhitelist(source_addr.clone()), &true);
    });

    assert!(client
        .try_submit_price(&source_addr, &symbol, &price, &source_name)
        .is_err());
}
