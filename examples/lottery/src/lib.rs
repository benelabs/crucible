#![no_std]

use soroban_sdk::{
    contract, contractimpl, contracttype, symbol_short, token, Address, BytesN, Env, Vec,
};

#[contracttype]
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum LotteryStatus {
    Open,
    CommitPhase,
    RevealPhase,
    Completed,
}

#[contracttype]
#[derive(Clone)]
pub struct LotteryConfig {
    pub admin: Address,
    pub token: Address,
    pub ticket_price: i128,
    pub protocol_fee_bps: i128, // e.g. 500 = 5%
    pub commit_deadline: u64,
    pub reveal_deadline: u64,
}

#[contracttype]
#[derive(Clone)]
pub struct LotteryState {
    pub status: LotteryStatus,
    pub total_prize_pool: i128,
    pub participants: Vec<Address>,
    pub operator_commitment: BytesN<32>,
    pub operator_revealed_secret: BytesN<32>,
    pub winner: Option<Address>,
    pub winner_payout: i128,
    pub protocol_fee_collected: i128,
}

#[contracttype]
#[derive(Clone)]
pub enum DataKey {
    Config,
    State,
    UserCommitment(Address),
    UserTicketCount(Address),
}

pub const BPS_DENOMINATOR: i128 = 10_000;

#[contract]
#[derive(Default)]
pub struct VerifiableLottery;

#[contractimpl]
impl VerifiableLottery {
    /// Initialize the lottery round with commit and reveal deadlines.
    pub fn initialize(
        env: Env,
        admin: Address,
        token: Address,
        ticket_price: i128,
        protocol_fee_bps: i128,
        commit_deadline: u64,
        reveal_deadline: u64,
    ) {
        if env.storage().instance().has(&DataKey::Config) {
            panic!("already initialized");
        }
        if ticket_price <= 0 {
            panic!("ticket price must be positive");
        }
        if !(0..=BPS_DENOMINATOR).contains(&protocol_fee_bps) {
            panic!("protocol_fee_bps out of bounds");
        }
        if commit_deadline <= env.ledger().timestamp() {
            panic!("commit deadline must be in the future");
        }
        if reveal_deadline <= commit_deadline {
            panic!("reveal deadline must be after commit deadline");
        }

        admin.require_auth();

        let config = LotteryConfig {
            admin,
            token,
            ticket_price,
            protocol_fee_bps,
            commit_deadline,
            reveal_deadline,
        };

        let state = LotteryState {
            status: LotteryStatus::Open,
            total_prize_pool: 0,
            participants: Vec::new(&env),
            operator_commitment: BytesN::from_array(&env, &[0u8; 32]),
            operator_revealed_secret: BytesN::from_array(&env, &[0u8; 32]),
            winner: None,
            winner_payout: 0,
            protocol_fee_collected: 0,
        };

        env.storage().instance().set(&DataKey::Config, &config);
        env.storage().instance().set(&DataKey::State, &state);
    }

    /// Set operator beacon commit before commit phase ends.
    pub fn set_operator_commitment(env: Env, commitment: BytesN<32>) {
        let config = Self::get_config(&env);
        config.admin.require_auth();

        if env.ledger().timestamp() > config.commit_deadline {
            panic!("commit phase ended");
        }

        let mut state = Self::get_state(&env);
        state.operator_commitment = commitment;
        env.storage().instance().set(&DataKey::State, &state);
    }

    /// Buy tickets and submit participant entropy commitment.
    pub fn buy_tickets(env: Env, buyer: Address, ticket_count: u32, commitment: BytesN<32>) {
        buyer.require_auth();
        let config = Self::get_config(&env);
        let mut state = Self::get_state(&env);

        if env.ledger().timestamp() > config.commit_deadline {
            panic!("late commitment rejected: commit deadline passed");
        }
        if ticket_count == 0 {
            panic!("ticket count must be at least 1");
        }

        let total_cost = config.ticket_price * (ticket_count as i128);

        // Transfer funds into contract
        token::TokenClient::new(&env, &config.token).transfer(
            &buyer,
            &env.current_contract_address(),
            &total_cost,
        );

        // Record participant tickets
        for _ in 0..ticket_count {
            state.participants.push_back(buyer.clone());
        }

        state.total_prize_pool += total_cost;

        let existing_count: u32 = env
            .storage()
            .instance()
            .get(&DataKey::UserTicketCount(buyer.clone()))
            .unwrap_or(0);
        env.storage()
            .instance()
            .set(&DataKey::UserTicketCount(buyer.clone()), &(existing_count + ticket_count));
        env.storage()
            .instance()
            .set(&DataKey::UserCommitment(buyer.clone()), &commitment);
        env.storage().instance().set(&DataKey::State, &state);

        env.events().publish(
            (symbol_short!("ticket"), buyer),
            (ticket_count, total_cost),
        );
    }

    /// Operator or beacon reveals random seed after commit deadline.
    /// Determines winning ticket index verifiably.
    pub fn reveal_and_distribute(env: Env, operator_secret: BytesN<32>, salt: BytesN<32>) {
        let config = Self::get_config(&env);
        let mut state = Self::get_state(&env);

        let now = env.ledger().timestamp();
        if now <= config.commit_deadline {
            panic!("commit phase still active");
        }
        if now > config.reveal_deadline {
            panic!("reveal deadline expired");
        }
        if state.status == LotteryStatus::Completed {
            panic!("lottery already completed");
        }
        if state.participants.is_empty() {
            panic!("no participants in lottery");
        }

        // Verify operator commitment: hash(secret || salt) == operator_commitment
        // Combine secret and salt
        let mut combined = [0u8; 64];
        let secret_bytes = operator_secret.to_array();
        let salt_bytes = salt.to_array();
        for i in 0..32 {
            combined[i] = secret_bytes[i];
            combined[32 + i] = salt_bytes[i];
        }
        let hash_check: BytesN<32> = env.crypto().sha256(&soroban_sdk::Bytes::from_slice(&env, &combined)).into();
        if hash_check != state.operator_commitment {
            panic!("invalid commitment reveal");
        }

        // Calculate winning index via verifiable pseudo-randomness
        // seed = sha256(secret || timestamp || total_pool)
        let mut entropy_input = soroban_sdk::Bytes::from_slice(&env, &secret_bytes);
        entropy_input.append(&soroban_sdk::Bytes::from_slice(&env, &salt_bytes));
        let seed: BytesN<32> = env.crypto().sha256(&entropy_input).into();
        let seed_bytes = seed.to_array();

        let num_participants = state.participants.len() as usize;
        let mut rand_u64: u64 = 0;
        for i in 0..8 {
            rand_u64 = (rand_u64 << 8) | (seed_bytes[i] as u64);
        }
        let winning_index = (rand_u64 as usize) % num_participants;
        let winner = state.participants.get(winning_index as u32).unwrap();

        // Calculate payout minus protocol fee
        let protocol_fee = (state.total_prize_pool * config.protocol_fee_bps) / BPS_DENOMINATOR;
        let winner_amount = state.total_prize_pool - protocol_fee;

        state.winner = Some(winner.clone());
        state.winner_payout = winner_amount;
        state.protocol_fee_collected = protocol_fee;
        state.operator_revealed_secret = operator_secret;
        state.status = LotteryStatus::Completed;

        env.storage().instance().set(&DataKey::State, &state);

        // Transfer prize to winner
        if winner_amount > 0 {
            token::TokenClient::new(&env, &config.token).transfer(
                &env.current_contract_address(),
                &winner,
                &winner_amount,
            );
        }

        // Transfer protocol fee to admin
        if protocol_fee > 0 {
            token::TokenClient::new(&env, &config.token).transfer(
                &env.current_contract_address(),
                &config.admin,
                &protocol_fee,
            );
        }

        env.events().publish(
            (symbol_short!("winner"), winner),
            (winner_amount, protocol_fee),
        );
    }

    /// Retrieve the lottery configuration.
    pub fn get_config(env: &Env) -> LotteryConfig {
        env.storage().instance().get(&DataKey::Config).unwrap()
    }

    /// Retrieve the current lottery state.
    pub fn get_state(env: &Env) -> LotteryState {
        env.storage().instance().get(&DataKey::State).unwrap()
    }
}

#[cfg(test)]
mod test;
