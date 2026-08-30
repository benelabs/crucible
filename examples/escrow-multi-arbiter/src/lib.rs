//! Escrow contract with multiple arbiters (M-of-N approval).
//!
//! In addition to the legacy single-arbiter approval flow, disputes can now be
//! settled by vote collection until an M-of-N quorum is reached. The contract
//! automatically executes the chosen settlement once quorum is satisfied.
#![no_std]
use soroban_sdk::{
    contract, contractimpl, contracttype, symbol_short, token, vec, Address, Env, Vec,
};

#[contracttype]
#[derive(Clone, PartialEq, Debug)]
pub enum EscrowStatus {
    Pending,
    Approved,
    Claimed,
    Refunded,
    ReleasedToBuyer,
    RefundedToSeller,
    SplitCustom,
}

#[contracttype]
#[derive(Clone, PartialEq, Debug)]
pub enum ArbiterVote {
    ReleaseToBuyer,
    RefundToSeller,
    SplitCustom(i128, i128),
}

#[contracttype]
#[derive(Clone, PartialEq, Debug)]
pub struct VoteRecord {
    pub arbiter: Address,
    pub vote: ArbiterVote,
}

#[contracttype]
#[derive(Clone)]
pub struct EscrowState {
    pub depositor: Address,
    pub recipient: Address,
    /// All registered arbiters; any one may approve.
    pub arbiters: Vec<Address>,
    pub token: Address,
    pub amount: i128,
    pub unlock_time: u64,
    pub quorum: u32,
    pub status: EscrowStatus,
    pub release_votes: u32,
    pub refund_votes: u32,
    pub split_buyer_share: i128,
    pub split_seller_share: i128,
    pub votes: Vec<VoteRecord>,
}

#[contracttype]
enum DataKey {
    State,
}

/// An escrow contract where arbiters vote toward a settlement outcome.
#[contract]
#[derive(Default)]
pub struct MultiArbiterEscrow;

#[contractimpl]
impl MultiArbiterEscrow {
    /// Create a new escrow. At least one arbiter is required.
    pub fn create(
        env: Env,
        depositor: Address,
        recipient: Address,
        arbiters: Vec<Address>,
        token: Address,
        amount: i128,
        unlock_time: u64,
    ) {
        Self::create_with_quorum(env, depositor, recipient, arbiters, token, amount, unlock_time, 1);
    }

    /// Create a new escrow that settles only after a configured M-of-N quorum.
    pub fn create_with_quorum(
        env: Env,
        depositor: Address,
        recipient: Address,
        arbiters: Vec<Address>,
        token: Address,
        amount: i128,
        unlock_time: u64,
        quorum: u32,
    ) {
        if env.storage().instance().has(&DataKey::State) {
            panic!("escrow already exists");
        }
        if amount <= 0 {
            panic!("amount must be positive");
        }
        if arbiters.is_empty() {
            panic!("at least one arbiter required");
        }
        let quorum = quorum.max(1).min(arbiters.len() as u32);
        depositor.require_auth();

        token::TokenClient::new(&env, &token).transfer(
            &depositor,
            &env.current_contract_address(),
            &amount,
        );

        env.storage().instance().set(
            &DataKey::State,
            &EscrowState {
                depositor,
                recipient,
                arbiters,
                token,
                amount,
                unlock_time,
                quorum,
                status: EscrowStatus::Pending,
                release_votes: 0,
                refund_votes: 0,
                split_buyer_share: 0,
                split_seller_share: 0,
                votes: vec![&env],
            },
        );
        env.events().publish((symbol_short!("created"),), amount);
    }

    /// Legacy any-one arbiter approval path.
    pub fn approve(env: Env, caller: Address) {
        let mut state: EscrowState = env.storage().instance().get(&DataKey::State).unwrap();
        if state.status != EscrowStatus::Pending {
            panic!("escrow is not pending");
        }
        if !state.arbiters.contains(&caller) {
            panic!("caller is not a registered arbiter");
        }
        caller.require_auth();
        state.status = EscrowStatus::Approved;
        env.storage().instance().set(&DataKey::State, &state);
        env.events().publish((symbol_short!("approved"),), ());
    }

    /// Vote to release the escrow to the buyer.
    pub fn release_to_buyer(env: Env, arbiter: Address) {
        Self::record_vote(&env, arbiter, ArbiterVote::ReleaseToBuyer);
    }

    /// Vote to refund the escrow to the seller.
    pub fn refund_to_seller(env: Env, arbiter: Address) {
        Self::record_vote(&env, arbiter, ArbiterVote::RefundToSeller);
    }

    /// Vote for a custom buyer/seller split, with the sum matching the escrow amount.
    pub fn split_custom(env: Env, arbiter: Address, buyer_share: i128, seller_share: i128) {
        if buyer_share + seller_share != 0 {
            let amount = env.storage().instance().get::<DataKey, EscrowState>(&DataKey::State).unwrap().amount;
            if buyer_share + seller_share != amount {
                panic!("split must total the escrow amount");
            }
        }
        Self::record_vote(&env, arbiter, ArbiterVote::SplitCustom(buyer_share, seller_share));
    }

    /// Recipient claims the escrowed funds.
    /// Requires either arbiter approval or that `unlock_time` has passed.
    pub fn claim(env: Env) {
        let mut state: EscrowState = env.storage().instance().get(&DataKey::State).unwrap();
        if state.status != EscrowStatus::Pending && state.status != EscrowStatus::Approved {
            panic!("escrow already settled");
        }
        let now = env.ledger().timestamp();
        if state.status != EscrowStatus::Approved && now < state.unlock_time {
            panic!("time lock has not expired");
        }
        state.recipient.require_auth();
        state.status = EscrowStatus::Claimed;
        env.storage().instance().set(&DataKey::State, &state);

        token::TokenClient::new(&env, &state.token).transfer(
            &env.current_contract_address(),
            &state.recipient,
            &state.amount,
        );
        env.events()
            .publish((symbol_short!("claimed"),), state.amount);
    }

    /// Depositor reclaims funds after the time lock expires (if unclaimed).
    pub fn refund(env: Env) {
        let mut state: EscrowState = env.storage().instance().get(&DataKey::State).unwrap();
        if state.status != EscrowStatus::Pending {
            panic!("can only refund a pending escrow");
        }
        let now = env.ledger().timestamp();
        if now < state.unlock_time {
            panic!("time lock has not expired");
        }
        state.depositor.require_auth();
        state.status = EscrowStatus::Refunded;
        env.storage().instance().set(&DataKey::State, &state);

        token::TokenClient::new(&env, &state.token).transfer(
            &env.current_contract_address(),
            &state.depositor,
            &state.amount,
        );
        env.events()
            .publish((symbol_short!("refunded"),), state.amount);
    }

    /// Return the current escrow state.
    pub fn get_state(env: Env) -> EscrowState {
        env.storage().instance().get(&DataKey::State).unwrap()
    }

    fn record_vote(env: &Env, arbiter: Address, decision: ArbiterVote) {
        let mut state: EscrowState = env.storage().instance().get(&DataKey::State).unwrap();
        if state.status != EscrowStatus::Pending {
            panic!("escrow is not pending");
        }
        if !state.arbiters.contains(&arbiter) {
            panic!("caller is not a registered arbiter");
        }
        if state.votes.iter().any(|vote| vote.arbiter == arbiter) {
            panic!("arbiter has already voted");
        }
        arbiter.require_auth();

        state.votes.push_back(VoteRecord { arbiter: arbiter.clone(), vote: decision.clone() });

        match decision {
            ArbiterVote::ReleaseToBuyer => state.release_votes += 1,
            ArbiterVote::RefundToSeller => state.refund_votes += 1,
            ArbiterVote::SplitCustom(buyer_share, seller_share) => {
                state.split_buyer_share = buyer_share;
                state.split_seller_share = seller_share;
            }
        }

        if state.release_votes >= state.quorum {
            state.status = EscrowStatus::ReleasedToBuyer;
            env.storage().instance().set(&DataKey::State, &state);
            token::TokenClient::new(env, &state.token).transfer(
                &env.current_contract_address(),
                &state.recipient,
                &state.amount,
            );
            env.events().publish((symbol_short!("released"),), state.amount);
            return;
        }

        if state.refund_votes >= state.quorum {
            state.status = EscrowStatus::RefundedToSeller;
            env.storage().instance().set(&DataKey::State, &state);
            token::TokenClient::new(env, &state.token).transfer(
                &env.current_contract_address(),
                &state.depositor,
                &state.amount,
            );
            env.events().publish((symbol_short!("refunded"),), state.amount);
            return;
        }

        if matches!(decision, ArbiterVote::SplitCustom(_, _)) && state.split_buyer_share + state.split_seller_share == state.amount {
            state.status = EscrowStatus::SplitCustom;
            env.storage().instance().set(&DataKey::State, &state);
            token::TokenClient::new(env, &state.token).transfer(
                &env.current_contract_address(),
                &state.recipient,
                &state.split_buyer_share,
            );
            token::TokenClient::new(env, &state.token).transfer(
                &env.current_contract_address(),
                &state.depositor,
                &state.split_seller_share,
            );
            env.events().publish((symbol_short!("split"),), (state.split_buyer_share, state.split_seller_share));
            return;
        }

        env.storage().instance().set(&DataKey::State, &state);
    }
}

#[cfg(test)]
mod test;
