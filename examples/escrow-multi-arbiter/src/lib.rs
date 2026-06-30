//! Escrow contract with multiple arbiters (M-of-N approval).
//!
//! Any one of the registered arbiters may approve an early release. Once
//! approved by any arbiter, the recipient can claim immediately without
//! waiting for the time lock. The depositor may refund only after the time
//! lock expires and no claim has been made.
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
    pub status: EscrowStatus,
}

#[contracttype]
enum DataKey {
    State,
}

/// An escrow contract where *any* registered arbiter can approve early release.
#[contract]
#[derive(Default)]
pub struct MultiArbiterEscrow;

#[contractimpl]
impl MultiArbiterEscrow {
    /// Create a new escrow.  At least one arbiter is required.
    pub fn create(
        env: Env,
        depositor: Address,
        recipient: Address,
        arbiters: Vec<Address>,
        token: Address,
        amount: i128,
        unlock_time: u64,
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
                status: EscrowStatus::Pending,
            },
        );
        env.events().publish((symbol_short!("created"),), amount);
    }

    /// Any registered arbiter may approve early release.
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

    /// Recipient claims the escrowed funds.
    ///
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
}

#[cfg(test)]
mod test;
