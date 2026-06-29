//! Cross-contract communication example.
//!
//! Demonstrates how Soroban contracts call each other:
//!
//! - [`Counter`] — a simple counter that can be incremented by anyone.
//! - [`Router`] — calls `Counter` and a token contract; batches operations
//!   across multiple contracts in a single transaction.
//! - [`Aggregator`] — calls `Router` to orchestrate multi-step workflows,
//!   showing a two-level call chain.
#![no_std]

use soroban_sdk::{
    contract, contractimpl, contracttype, symbol_short, token, Address, Env,
};

// ---------------------------------------------------------------------------
// Counter — a simple counter callable by other contracts
// ---------------------------------------------------------------------------

#[contracttype]
enum CounterKey {
    Count,
}

/// A simple counter contract.
///
/// Any contract (or user) can increment it. The current value is readable
/// by anyone. Used by `Router` to demonstrate cross-contract state mutation.
#[contract]
#[derive(Default)]
pub struct Counter;

#[contractimpl]
impl Counter {
    /// Increment the counter by 1 and return the new value.
    pub fn increment(env: Env) -> u32 {
        let count: u32 = env
            .storage()
            .instance()
            .get(&CounterKey::Count)
            .unwrap_or(0);
        let new_count = count + 1;
        env.storage().instance().set(&CounterKey::Count, &new_count);
        env.events().publish((symbol_short!("incr"),), new_count);
        new_count
    }

    /// Return the current counter value.
    pub fn get(env: Env) -> u32 {
        env.storage()
            .instance()
            .get(&CounterKey::Count)
            .unwrap_or(0)
    }
}

// ---------------------------------------------------------------------------
// Router — calls Counter and a token contract
// ---------------------------------------------------------------------------

#[contracttype]
enum RouterKey {
    Counter,
    Token,
}

/// A router contract that orchestrates calls to `Counter` and a token.
///
/// Demonstrates:
/// - Calling another contract via a generated client (`CounterClient`).
/// - Calling a token contract via `token::TokenClient`.
/// - Storing cross-contract addresses in instance storage.
#[contract]
#[derive(Default)]
pub struct Router;

#[contractimpl]
impl Router {
    /// Initialize the router with the addresses of the counter and token contracts.
    pub fn initialize(env: Env, counter: Address, token: Address) {
        if env.storage().instance().has(&RouterKey::Counter) {
            panic!("already initialized");
        }
        env.storage().instance().set(&RouterKey::Counter, &counter);
        env.storage().instance().set(&RouterKey::Token, &token);
    }

    /// Increment the counter contract and return the new value.
    ///
    /// This is a cross-contract call: `Router` → `Counter`.
    pub fn ping_counter(env: Env) -> u32 {
        let counter: Address = env.storage().instance().get(&RouterKey::Counter).unwrap();
        let client = CounterClient::new(&env, &counter);
        let value = client.increment();
        env.events().publish((symbol_short!("pinged"),), value);
        value
    }

    /// Transfer `amount` tokens from `from` to `to` via the stored token contract.
    ///
    /// This is a cross-contract call: `Router` → `Token`.
    pub fn route_transfer(env: Env, from: Address, to: Address, amount: i128) {
        from.require_auth();
        let token_addr: Address = env.storage().instance().get(&RouterKey::Token).unwrap();
        token::TokenClient::new(&env, &token_addr).transfer(&from, &to, &amount);
        env.events().publish((symbol_short!("routed"),), amount);
    }

    /// Return the current counter value by calling the counter contract.
    pub fn counter_value(env: Env) -> u32 {
        let counter: Address = env.storage().instance().get(&RouterKey::Counter).unwrap();
        CounterClient::new(&env, &counter).get()
    }
}

// ---------------------------------------------------------------------------
// Aggregator — calls Router (two-level cross-contract chain)
// ---------------------------------------------------------------------------

#[contracttype]
enum AggKey {
    Router,
    TotalRouted,
}

/// An aggregator that calls `Router`, forming a two-level call chain:
/// `Aggregator` → `Router` → `Counter` / `Token`.
///
/// Tracks the cumulative amount routed through it.
#[contract]
#[derive(Default)]
pub struct Aggregator;

#[contractimpl]
impl Aggregator {
    /// Initialize the aggregator with the address of the router contract.
    pub fn initialize(env: Env, router: Address) {
        if env.storage().instance().has(&AggKey::Router) {
            panic!("already initialized");
        }
        env.storage().instance().set(&AggKey::Router, &router);
        env.storage().instance().set(&AggKey::TotalRouted, &0_i128);
    }

    /// Ping the counter through the router and return the counter value.
    ///
    /// Call chain: `Aggregator` → `Router::ping_counter` → `Counter::increment`.
    pub fn aggregate_ping(env: Env) -> u32 {
        let router: Address = env.storage().instance().get(&AggKey::Router).unwrap();
        let value = RouterClient::new(&env, &router).ping_counter();
        env.events().publish((symbol_short!("aggping"),), value);
        value
    }

    /// Route a transfer through the router and track the cumulative total.
    ///
    /// Call chain: `Aggregator` → `Router::route_transfer` → `Token::transfer`.
    pub fn aggregate_transfer(env: Env, from: Address, to: Address, amount: i128) {
        from.require_auth();
        let router: Address = env.storage().instance().get(&AggKey::Router).unwrap();
        RouterClient::new(&env, &router).route_transfer(&from, &to, &amount);

        let total: i128 = env
            .storage()
            .instance()
            .get(&AggKey::TotalRouted)
            .unwrap_or(0);
        env.storage()
            .instance()
            .set(&AggKey::TotalRouted, &(total + amount));
        env.events()
            .publish((symbol_short!("aggtxfr"),), total + amount);
    }

    /// Return the cumulative amount routed through this aggregator.
    pub fn total_routed(env: Env) -> i128 {
        env.storage()
            .instance()
            .get(&AggKey::TotalRouted)
            .unwrap_or(0)
    }

    /// Return the current counter value via the router.
    pub fn counter_value(env: Env) -> u32 {
        let router: Address = env.storage().instance().get(&AggKey::Router).unwrap();
        RouterClient::new(&env, &router).counter_value()
    }
}

#[cfg(test)]
mod test;
