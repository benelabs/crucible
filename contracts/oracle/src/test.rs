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
