#![no_std]
use soroban_sdk::{contract, contractimpl, contracttype, symbol_short, Env};

#[contracttype]
enum DataKey {
    Count,
}

/// A simple on-chain counter contract.
#[contract]
#[derive(Default)]
pub struct Counter;

#[contractimpl]
impl Counter {
    /// Initialize the counter with a starting value.
    ///
    /// Panics if the counter has already been initialized.
    pub fn initialize(env: Env, value: u32) {
        if env.storage().instance().has(&DataKey::Count) {
            panic!("already initialized");
        }
        env.storage().instance().set(&DataKey::Count, &value);
    }

    /// Increment the counter by 1 and return the new value.
    pub fn increment(env: Env) -> u32 {
        let count: u32 = env.storage().instance().get(&DataKey::Count).unwrap_or(0);
        let new_count = count + 1;
        env.storage().instance().set(&DataKey::Count, &new_count);
        env.events().publish((symbol_short!("incr"),), new_count);
        new_count
    }

    /// Decrement the counter by 1 and return the new value.
    ///
    /// Panics if the counter is already at zero.
    pub fn decrement(env: Env) -> u32 {
        let count: u32 = env.storage().instance().get(&DataKey::Count).unwrap_or(0);
        if count == 0 {
            panic!("underflow: counter is already at zero");
        }
        let new_count = count - 1;
        env.storage().instance().set(&DataKey::Count, &new_count);
        env.events().publish((symbol_short!("decr"),), new_count);
        new_count
    }

    /// Increment the counter by `amount` and return the new value.
    pub fn increment_by(env: Env, amount: u32) -> u32 {
        let count: u32 = env.storage().instance().get(&DataKey::Count).unwrap_or(0);
        let new_count = count + amount;
        env.storage().instance().set(&DataKey::Count, &new_count);
        new_count
    }

    /// Return the current counter value.
    pub fn get(env: Env) -> u32 {
        env.storage().instance().get(&DataKey::Count).unwrap_or(0)
    }

    /// Reset the counter to zero.
    pub fn reset(env: Env) {
        env.storage().instance().set(&DataKey::Count, &0u32);
        env.events().publish((symbol_short!("reset"),), ());
    }
}

#[cfg(test)]
mod test;
