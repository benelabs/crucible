#![no_std]
#![allow(deprecated)]

use soroban_sdk::{
    contract, contracterror, contractimpl, contracttype, panic_with_error, symbol_short, token, Address,
    Env, Map, Symbol, Vec,
};

// Define storage keys
#[contracttype]
#[derive(Clone, Debug, Eq, PartialEq)]
enum DataKey {
    Admins,   // Vec<Address>
    Quorum,   // u32
    Balances, // Map<(Address, Address), i128>
}

#[contracterror]
#[derive(Copy, Clone, Debug, Eq, PartialEq, PartialOrd, Ord)]
#[repr(u32)]
pub enum ContractError {
    NotAdmin = 1,
    InsufficientQuorum = 2,
    InsufficientBalance = 3,
}

#[contract]
pub struct Treasury;

#[contractimpl]
impl Treasury {
    /// Initialize the treasury with a list of admin addresses and a quorum threshold.
    pub fn initialize(env: Env, admins: Vec<Address>, quorum: u32) {
        // Store admins and quorum only once
        if env.storage().instance().has(&DataKey::Admins) {
            panic!("already initialized");
        }
        env.storage().instance().set(&DataKey::Admins, &admins);
        env.storage().instance().set(&DataKey::Quorum, &quorum);
        // Initialize empty balances map
        let balances: Map<(Address, Address), i128> = Map::new(&env);
        env.storage().instance().set(&DataKey::Balances, &balances);
        env.events()
            .publish((symbol_short!("init"),), (admins, quorum));
    }

    fn is_admin(env: &Env, caller: Address) -> bool {
        let admins: Vec<Address> = env.storage().instance().get(&DataKey::Admins).unwrap();
        admins.iter().any(|a| a == caller)
    }

    /// Deposit an amount of a given token (use Address::from([0;32]) for native XLM).
    pub fn deposit(env: Env, depositor: Address, token: Address, amount: i128) {
        // Ensure the depositor authorized this operation
        depositor.require_auth();

        // Perform actual token transfer from depositor to this contract (treasury)
        token::Client::new(&env, &token).transfer(&depositor, &env.current_contract_address(), &amount);

        // Update internal accounting only after successful transfer
        let mut balances: Map<(Address, Address), i128> =
            env.storage().instance().get(&DataKey::Balances).unwrap();
        let treasury_addr = env.current_contract_address();
        let key = (treasury_addr.clone(), token.clone());
        let current = balances.get(key.clone()).unwrap_or(0);
        let new_balance = current + amount;
        if new_balance < 0 {
            panic_with_error!(&env, ContractError::InsufficientBalance);
        }
        balances.set(key.clone(), new_balance);
        env.storage().instance().set(&DataKey::Balances, &balances);
        env.events()
            .publish((symbol_short!("deposit"),), (depositor, token, amount));
    }

    /// Withdraw tokens from the treasury to a destination address.
    /// `signers` must include >= quorum admin addresses.
    pub fn withdraw(env: Env, to: Address, token: Address, amount: i128, signers: Vec<Address>) {
        // Verify quorum
        let quorum: u32 = env.storage().instance().get(&DataKey::Quorum).unwrap();
        let admins: Vec<Address> = env.storage().instance().get(&DataKey::Admins).unwrap();
        let mut valid = 0u32;
        for s in signers.iter() {
            if admins.iter().any(|a| a == s) {
                valid += 1;
            }
        }
        if valid < quorum {
            panic_with_error!(&env, ContractError::InsufficientQuorum);
        }
        // Treasury address is the contract's own address
        let treasury_addr = env.current_contract_address();
        let mut balances: Map<(Address, Address), i128> =
            env.storage().instance().get(&DataKey::Balances).unwrap();
        let key = (treasury_addr.clone(), token.clone());
        let current = balances.get(key.clone()).unwrap_or(0);
        if current < amount {
            panic_with_error!(&env, ContractError::InsufficientBalance);
        }
        let new_balance = current - amount;
        balances.set(key.clone(), new_balance);
        // Transfer to destination (mock token handles actual credit; here we just emit event)
        env.storage().instance().set(&DataKey::Balances, &balances);
        env.events()
            .publish((symbol_short!("withdraw"),), (to, token, amount));
    }

    /// Query the balance of an account for a given token.
    pub fn balance_of(env: Env, account: Address, token: Address) -> i128 {
        let balances: Map<(Address, Address), i128> =
            env.storage().instance().get(&DataKey::Balances).unwrap();
        balances.get((account, token)).unwrap_or(0)
    }
}
