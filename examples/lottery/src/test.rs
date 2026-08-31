#![cfg(test)]

use super::*;
use soroban_sdk::{
    contract, contractimpl, testutils::{Address as _, Ledger}, Address, Bytes, BytesN, Env,
};

#[contract]
pub struct MockToken;

#[contractimpl]
impl MockToken {
    pub fn initialize(env: Env, admin: Address) {
        env.storage().instance().set(&symbol_short!("admin"), &admin);
    }

    pub fn mint(env: Env, to: Address, amount: i128) {
        let bal: i128 = env.storage().instance().get(&to).unwrap_or(0);
        env.storage().instance().set(&to, &(bal + amount));
    }

    pub fn transfer(env: Env, from: Address, to: Address, amount: i128) {
        let from_bal: i128 = env.storage().instance().get(&from).unwrap_or(0);
        if from_bal < amount {
            panic!("insufficient balance");
        }
        let to_bal: i128 = env.storage().instance().get(&to).unwrap_or(0);
        env.storage().instance().set(&from, &(from_bal - amount));
        env.storage().instance().set(&to, &(to_bal + amount));
    }

    pub fn balance(env: Env, id: Address) -> i128 {
        env.storage().instance().get(&id).unwrap_or(0)
    }
}

#[test]
fn test_lottery_commit_reveal_and_winner_payout() {
    let env = Env::default();
    env.mock_all_auths();
    env.ledger().set_timestamp(100);

    let admin = Address::generate(&env);
    let player1 = Address::generate(&env);
    let player2 = Address::generate(&env);

    let token_id = env.register(MockToken, ());
    let token_client = MockTokenClient::new(&env, &token_id);
    token_client.initialize(&admin);

    token_client.mint(&player1, &1000);
    token_client.mint(&player2, &1000);

    let lottery_id = env.register(VerifiableLottery, ());
    let lottery_client = VerifiableLotteryClient::new(&env, &lottery_id);

    let ticket_price = 100_i128;
    let fee_bps = 1000_i128; // 10%
    let commit_deadline = 200_u64;
    let reveal_deadline = 300_u64;

    lottery_client.initialize(
        &admin,
        &token_id,
        &ticket_price,
        &fee_bps,
        &commit_deadline,
        &reveal_deadline,
    );

    // Operator prepares secret & commitment
    let secret = [0x42u8; 32];
    let salt = [0x07u8; 32];
    let mut combined = [0u8; 64];
    for i in 0..32 {
        combined[i] = secret[i];
        combined[32 + i] = salt[i];
    }
    let operator_commitment: BytesN<32> = env
        .crypto()
        .sha256(&Bytes::from_slice(&env, &combined))
        .into();
    lottery_client.set_operator_commitment(&operator_commitment);

    // Players buy tickets
    let player1_commit = BytesN::from_array(&env, &[1u8; 32]);
    let player2_commit = BytesN::from_array(&env, &[2u8; 32]);

    lottery_client.buy_tickets(&player1, &2, &player1_commit);
    lottery_client.buy_tickets(&player2, &3, &player2_commit);

    assert_eq!(token_client.balance(&lottery_id), 500); // 5 tickets * 100

    // Fast-forward past commit deadline into reveal phase
    env.ledger().set_timestamp(250);

    let secret_bn = BytesN::from_array(&env, &secret);
    let salt_bn = BytesN::from_array(&env, &salt);
    lottery_client.reveal_and_distribute(&secret_bn, &salt_bn);

    let state = lottery_client.get_state();
    assert_eq!(state.status, LotteryStatus::Completed);
    assert!(state.winner.is_some());
    assert_eq!(state.total_prize_pool, 500);
    assert_eq!(state.protocol_fee_collected, 50); // 10% of 500
    assert_eq!(state.winner_payout, 450); // 90% of 500

    // Check balances
    assert_eq!(token_client.balance(&admin), 50);
    let winner = state.winner.unwrap();
    if winner == player1 {
        assert_eq!(token_client.balance(&player1), 800 + 450);
    } else {
        assert_eq!(token_client.balance(&player2), 700 + 450);
    }
}

#[test]
#[should_panic(expected = "late commitment rejected: commit deadline passed")]
fn test_late_commitment_rejected() {
    let env = Env::default();
    env.mock_all_auths();
    env.ledger().set_timestamp(100);

    let admin = Address::generate(&env);
    let player = Address::generate(&env);
    let token_id = env.register(MockToken, ());

    let lottery_id = env.register(VerifiableLottery, ());
    let lottery_client = VerifiableLotteryClient::new(&env, &lottery_id);

    lottery_client.initialize(&admin, &token_id, &50, &500, &150, &200);

    // Fast-forward beyond commit deadline
    env.ledger().set_timestamp(160);

    let player_commit = BytesN::from_array(&env, &[1u8; 32]);
    lottery_client.buy_tickets(&player, &1, &player_commit);
}

#[test]
#[should_panic(expected = "invalid commitment reveal")]
fn test_invalid_commitment_reveal_fails() {
    let env = Env::default();
    env.mock_all_auths();
    env.ledger().set_timestamp(100);

    let admin = Address::generate(&env);
    let player = Address::generate(&env);
    let token_id = env.register(MockToken, ());
    let token_client = MockTokenClient::new(&env, &token_id);
    token_client.mint(&player, &500);

    let lottery_id = env.register(VerifiableLottery, ());
    let lottery_client = VerifiableLotteryClient::new(&env, &lottery_id);

    lottery_client.initialize(&admin, &token_id, &50, &500, &150, &200);

    let commitment = BytesN::from_array(&env, &[0xAAu8; 32]);
    lottery_client.set_operator_commitment(&commitment);

    let player_commit = BytesN::from_array(&env, &[1u8; 32]);
    lottery_client.buy_tickets(&player, &1, &player_commit);

    // Advance to reveal phase
    env.ledger().set_timestamp(160);

    // Fake reveal
    let fake_secret = BytesN::from_array(&env, &[0x00u8; 32]);
    let fake_salt = BytesN::from_array(&env, &[0x00u8; 32]);
    lottery_client.reveal_and_distribute(&fake_secret, &fake_salt);
}
