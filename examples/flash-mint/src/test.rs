#![cfg(test)]

use super::*;
use soroban_sdk::{
    contract, contractimpl, testutils::Address as _, Address, Bytes, BytesN, Env,
};

#[contract]
pub struct ArbitrageBorrower;

#[contractimpl]
impl ArbitrageBorrower {
    pub fn on_flash_loan(
        env: Env,
        _initiator: Address,
        token: Address,
        amount: i128,
        fee: i128,
        data: Bytes,
    ) -> BytesN<32> {
        // Mock arbitrage profit generation: transfer fee back from internal profit
        // If data is "profit", we mint or simulate receiving arbitrage profit to cover the fee
        let token_client = FlashMintTokenClient::new(&env, &token);
        
        // Check if caller wants failure
        if data == Bytes::from_slice(&env, b"fail_callback") {
            return BytesN::from_array(&env, &[0u8; 32]);
        }

        if data == Bytes::from_slice(&env, b"no_fee_repay") {
            // Do not provide extra tokens to cover fee -> will fail repay check
            return BytesN::from_array(&env, &FLASH_LOAN_CALLBACK_SUCCESS);
        }

        // Simulate borrower generating arbitrage profit to pay the flash fee
        // In this test, token contract already minted amount into borrower.
        // We simulate that borrower earned enough profit to pay the fee.
        BytesN::from_array(&env, &FLASH_LOAN_CALLBACK_SUCCESS)
    }
}

#[test]
fn test_flash_mint_arbitrage_success_with_zero_fee() {
    let env = Env::default();
    env.mock_all_auths();

    let admin = Address::generate(&env);
    let token_id = env.register(FlashMintToken, ());
    let token_client = FlashMintTokenClient::new(&env, &token_id);
    token_client.initialize(&admin, &0); // 0% fee

    let borrower_id = env.register(ArbitrageBorrower, ());

    let flash_amount = 500_000_i128;
    let data = Bytes::from_slice(&env, b"arbitrage_loop");

    assert_eq!(token_client.total_supply(), 0);
    assert_eq!(token_client.max_flash_loan(&token_id), i128::MAX);
    assert_eq!(token_client.flash_fee(&token_id, &flash_amount), 0);

    let success = token_client.flash_loan(&borrower_id, &token_id, &flash_amount, &data);
    assert!(success);

    // After flash loan completion, total supply and borrower balance restored
    assert_eq!(token_client.total_supply(), 0);
    assert_eq!(token_client.balance_of(&borrower_id), 0);
}

#[test]
fn test_flash_mint_with_fee_and_arbitrage_profit() {
    let env = Env::default();
    env.mock_all_auths();

    let admin = Address::generate(&env);
    let token_id = env.register(FlashMintToken, ());
    let token_client = FlashMintTokenClient::new(&env, &token_id);
    // 50 bps fee = 0.5%
    token_client.initialize(&admin, &50);

    let borrower_id = env.register(ArbitrageBorrower, ());

    let flash_amount = 100_000_i128;
    let fee = token_client.flash_fee(&token_id, &flash_amount);
    assert_eq!(fee, 500); // 100_000 * 50 / 10000 = 500

    // Provide borrower with upfront profit / capital to cover fee
    token_client.mint(&borrower_id, &fee);
    assert_eq!(token_client.balance_of(&borrower_id), 500);
    assert_eq!(token_client.total_supply(), 500);

    let data = Bytes::from_slice(&env, b"arbitrage_loop");
    let success = token_client.flash_loan(&borrower_id, &token_id, &flash_amount, &data);
    assert!(success);

    // Fee was burned from borrower balance along with loan
    assert_eq!(token_client.balance_of(&borrower_id), 0);
    assert_eq!(token_client.total_supply(), 0);
}

#[test]
#[should_panic(expected = "insufficient balance to repay flash loan + fee")]
fn test_flash_mint_reverts_if_fee_unpaid() {
    let env = Env::default();
    env.mock_all_auths();

    let admin = Address::generate(&env);
    let token_id = env.register(FlashMintToken, ());
    let token_client = FlashMintTokenClient::new(&env, &token_id);
    token_client.initialize(&admin, &100); // 1% fee

    let borrower_id = env.register(ArbitrageBorrower, ());
    let flash_amount = 100_000_i128;

    // Borrower has 0 balance before loan, so amount + fee cannot be repaid
    let data = Bytes::from_slice(&env, b"no_fee_repay");
    token_client.flash_loan(&borrower_id, &token_id, &flash_amount, &data);
}

#[test]
#[should_panic(expected = "invalid flash loan callback response")]
fn test_flash_mint_reverts_on_invalid_callback() {
    let env = Env::default();
    env.mock_all_auths();

    let admin = Address::generate(&env);
    let token_id = env.register(FlashMintToken, ());
    let token_client = FlashMintTokenClient::new(&env, &token_id);
    token_client.initialize(&admin, &0);

    let borrower_id = env.register(ArbitrageBorrower, ());
    let data = Bytes::from_slice(&env, b"fail_callback");
    token_client.flash_loan(&borrower_id, &token_id, &10_000_i128, &data);
}
