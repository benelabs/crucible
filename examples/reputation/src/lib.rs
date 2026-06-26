#![no_std]
#![allow(deprecated)]
//! Reputation contract example.
//!
//! Demonstrates an admin-gated on-chain reputation system and how to test
//! it with the crucible testing toolkit.

use soroban_sdk::{contract, contractimpl, contracttype, symbol_short, Address, Env};

#[contracttype]
enum DataKey {
    Admin,
    Reputation(Address),
}

/// A simple admin-gated reputation contract.
///
/// The admin is set at initialization and is the only address allowed to
/// modify reputation scores.
#[contract]
#[derive(Default)]
pub struct ReputationContract;

#[contractimpl]
impl ReputationContract {
    /// Initialize the contract with an admin address.
    ///
    /// Panics if the contract has already been initialized.
    pub fn initialize(env: Env, admin: Address) {
        if env.storage().instance().has(&DataKey::Admin) {
            panic!("already initialized");
        }
        env.storage().instance().set(&DataKey::Admin, &admin);
    }

    /// Set the reputation score for `account` to `score`. Admin only.
    pub fn set_reputation(env: Env, caller: Address, account: Address, score: i32) {
        caller.require_auth();
        let admin: Address = env
            .storage()
            .instance()
            .get(&DataKey::Admin)
            .expect("not initialized");
        assert_eq!(caller, admin, "not admin");
        env.storage()
            .instance()
            .set(&DataKey::Reputation(account.clone()), &score);
        env.events()
            .publish((symbol_short!("rep_set"), account), score);
    }

    /// Increase the reputation of `account` by `amount`. Admin only.
    pub fn increase_reputation(env: Env, caller: Address, account: Address, amount: i32) {
        caller.require_auth();
        let admin: Address = env
            .storage()
            .instance()
            .get(&DataKey::Admin)
            .expect("not initialized");
        assert_eq!(caller, admin, "not admin");
        let current: i32 = env
            .storage()
            .instance()
            .get(&DataKey::Reputation(account.clone()))
            .unwrap_or(0);
        let new_score = current + amount;
        env.storage()
            .instance()
            .set(&DataKey::Reputation(account.clone()), &new_score);
        env.events()
            .publish((symbol_short!("rep_inc"), account), amount);
    }

    /// Decrease the reputation of `account` by `amount`. Admin only.
    pub fn decrease_reputation(env: Env, caller: Address, account: Address, amount: i32) {
        caller.require_auth();
        let admin: Address = env
            .storage()
            .instance()
            .get(&DataKey::Admin)
            .expect("not initialized");
        assert_eq!(caller, admin, "not admin");
        let current: i32 = env
            .storage()
            .instance()
            .get(&DataKey::Reputation(account.clone()))
            .unwrap_or(0);
        let new_score = current - amount;
        env.storage()
            .instance()
            .set(&DataKey::Reputation(account.clone()), &new_score);
        env.events()
            .publish((symbol_short!("rep_dec"), account), amount);
    }

    /// Return the current reputation score for `account` (defaults to 0).
    pub fn get_reputation(env: Env, account: Address) -> i32 {
        env.storage()
            .instance()
            .get(&DataKey::Reputation(account))
            .unwrap_or(0)
    }
}

#[cfg(test)]
mod test;
