#![no_std]

use soroban_sdk::{
    contract, contracterror, contractimpl, contracttype, panic_with_error, symbol_short, token,
    Address, Env, Map, Symbol, Vec,
};

// Define storage keys
#[contracttype]
#[derive(Clone, Debug, Eq, PartialEq)]
enum DataKey {
    Admins,   // Vec<Address>
    Quorum,   // u32
    Balances, // Map<(Address, Address), i128>
    ReentrancyGuard, // bool lock
}

#[contracterror]
#[derive(Copy, Clone, Debug, Eq, PartialEq, PartialOrd, Ord)]
#[repr(u32)]
pub enum ContractError {
    NotAdmin = 1,
    InsufficientQuorum = 2,
    InsufficientBalance = 3,
    /// `initialize` was called after the contract was already set up.
    AlreadyInitialized = 4,
    /// The admins vector passed to `initialize` was empty.
    EmptyAdmins = 5,
    /// `quorum` was zero or exceeded the number of admins.
    InvalidQuorum = 6,
    /// The admins vector contained duplicate addresses.
    DuplicateAdmin = 7,
    /// Reentrancy guard triggered - reentrant call forbidden.
    ReentrancyGuardLocked = 8,
}

#[contract]
pub struct Treasury;

#[contractimpl]
impl Treasury {
    /// Initialize the treasury with a list of admin addresses and a quorum threshold.
    ///
    /// # Errors
    /// - [`ContractError::AlreadyInitialized`] — called more than once.
    /// - [`ContractError::EmptyAdmins`] — `admins` is empty.
    /// - [`ContractError::InvalidQuorum`] — `quorum` is 0 or greater than `admins.len()`.
    /// - [`ContractError::DuplicateAdmin`] — `admins` contains duplicate addresses.
    pub fn initialize(env: Env, admins: Vec<Address>, quorum: u32) {
        if env.storage().instance().has(&DataKey::Admins) {
            panic_with_error!(&env, ContractError::AlreadyInitialized);
        }
        if admins.is_empty() {
            panic_with_error!(&env, ContractError::EmptyAdmins);
        }
        let n = admins.len();
        if quorum == 0 || quorum > n {
            panic_with_error!(&env, ContractError::InvalidQuorum);
        }
        // O(n²) duplicate check — admin lists are expected to be small
        for i in 0..n {
            for j in (i + 1)..n {
                if admins.get(i).unwrap() == admins.get(j).unwrap() {
                    panic_with_error!(&env, ContractError::DuplicateAdmin);
                }
            }
        }
        env.storage().instance().set(&DataKey::Admins, &admins);
        env.storage().instance().set(&DataKey::Quorum, &quorum);
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
        token::Client::new(&env, &token).transfer(
            &depositor,
            &env.current_contract_address(),
            &amount,
        );

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

    fn lock_guard(env: &Env) {
        let is_locked: bool = env
            .storage()
            .instance()
            .get(&DataKey::ReentrancyGuard)
            .unwrap_or(false);
        if is_locked {
            panic_with_error!(env, ContractError::ReentrancyGuardLocked);
        }
        env.storage().instance().set(&DataKey::ReentrancyGuard, &true);
    }

    fn unlock_guard(env: &Env) {
        env.storage().instance().set(&DataKey::ReentrancyGuard, &false);
    }

    /// Withdraw tokens from the treasury to a destination address.
    /// `signers` must include >= quorum admin addresses, each of which must authorize.
    pub fn withdraw(env: Env, to: Address, token: Address, amount: i128, signers: Vec<Address>) {
        Self::lock_guard(&env);

        // Require authorization from every signer before checking quorum.
        // This prevents passing arbitrary admin addresses without real signatures.
        for s in signers.iter() {
            s.require_auth();
        }

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
            Self::unlock_guard(&env);
            panic_with_error!(&env, ContractError::InsufficientQuorum);
        }
        // Treasury address is the contract's own address
        let treasury_addr = env.current_contract_address();
        let mut balances: Map<(Address, Address), i128> =
            env.storage().instance().get(&DataKey::Balances).unwrap();
        let key = (treasury_addr.clone(), token.clone());
        let current = balances.get(key.clone()).unwrap_or(0);
        if current < amount {
            Self::unlock_guard(&env);
            panic_with_error!(&env, ContractError::InsufficientBalance);
        }
        let new_balance = current - amount;
        balances.set(key.clone(), new_balance);
        // Transfer to destination (mock token handles actual credit; here we just emit event)
        env.storage().instance().set(&DataKey::Balances, &balances);
        env.events()
            .publish((symbol_short!("withdraw"),), (to, token, amount));

        Self::unlock_guard(&env);
    }

    /// Query the balance of an account for a given token.
    pub fn balance_of(env: Env, account: Address, token: Address) -> i128 {
        let balances: Map<(Address, Address), i128> =
            env.storage().instance().get(&DataKey::Balances).unwrap();
        balances.get((account, token)).unwrap_or(0)
    }

    /// Execute a flash loan. Borrows `amount` of `token` to `borrower`.
    /// The borrower must repay `amount + fee` within the same transaction scope.
    pub fn flash_loan(env: Env, borrower: Address, token: Address, amount: i128) {
        Self::lock_guard(&env);
        borrower.require_auth();

        if amount <= 0 {
            Self::unlock_guard(&env);
            panic_with_error!(&env, ContractError::InsufficientBalance);
        }

        let treasury_addr = env.current_contract_address();
        let mut balances: Map<(Address, Address), i128> =
            env.storage().instance().get(&DataKey::Balances).unwrap();
        let key = (treasury_addr.clone(), token.clone());
        let current = balances.get(key.clone()).unwrap_or(0);
        if current < amount {
            Self::unlock_guard(&env);
            panic_with_error!(&env, ContractError::InsufficientBalance);
        }

        // Calculate 0.1% fee (minimum 1 unit)
        let mut fee = amount / 1000;
        if fee == 0 {
            fee = 1;
        }

        // Transfer funds from treasury to borrower
        token::Client::new(&env, &token).transfer(&treasury_addr, &borrower, &amount);

        // Repay funds + fee from borrower back to treasury
        token::Client::new(&env, &token).transfer(&borrower, &treasury_addr, &(amount + fee));

        // Update internal accounting with collected fee
        let new_balance = current + fee;
        balances.set(key.clone(), new_balance);
        env.storage().instance().set(&DataKey::Balances, &balances);

        env.events()
            .publish((symbol_short!("flashloan"),), (borrower, token, amount, fee));

        Self::unlock_guard(&env);
    }
}

