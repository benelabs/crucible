#![no_std]
use soroban_sdk::{contract, contractimpl, contracttype, symbol_short, token, Address, Env};

/// Current state of the escrow.
#[contracttype]
#[derive(Clone, PartialEq, Debug)]
pub enum EscrowStatus {
    Pending,
    Approved,
    Claimed,
    Refunded,
}

/// All escrow data stored in instance storage under a single key.
#[contracttype]
#[derive(Clone)]
pub struct EscrowState {
    pub depositor: Address,
    pub recipient: Address,
    pub arbiter: Address,
    pub token: Address,
    pub amount: i128,
    /// Unix timestamp after which the recipient may claim without arbiter approval.
    pub unlock_time: u64,
    pub status: EscrowStatus,
}

#[contracttype]
enum DataKey {
    State,
}

/// A two-party escrow contract with a time lock and arbiter.
///
/// Workflow:
/// 1. Depositor calls `create`, funding the contract with tokens.
/// 2. Either:
///    - The arbiter calls `approve` to release funds early, then the recipient calls `claim`, OR
///    - Time passes until `unlock_time`, after which the recipient may `claim` directly, OR
///    - The depositor calls `refund` after `unlock_time` if the recipient never claimed.
#[contract]
#[derive(Default)]
pub struct Escrow;

#[contractimpl]
impl Escrow {
    /// Initialise the escrow, transferring `amount` tokens from `depositor`
    /// into this contract.
    pub fn create(
        env: Env,
        depositor: Address,
        recipient: Address,
        arbiter: Address,
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
        depositor.require_auth();

        // Pull tokens from depositor into this contract.
        token::TokenClient::new(&env, &token).transfer(
            &depositor,
            env.current_contract_address(),
            &amount,
        );

        env.storage().instance().set(
            &DataKey::State,
            &EscrowState {
                depositor,
                recipient,
                arbiter,
                token,
                amount,
                unlock_time,
                status: EscrowStatus::Pending,
            },
        );
        env.events().publish((symbol_short!("created"),), amount);
    }

    /// Arbiter approves an early release to the recipient.
    pub fn approve(env: Env, caller: Address) {
        let mut state: EscrowState = env.storage().instance().get(&DataKey::State).unwrap();
        if state.status != EscrowStatus::Pending {
            panic!("escrow is not pending");
        }
        if caller != state.arbiter {
            panic!("only the arbiter can approve");
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
