#![no_std]
#![allow(deprecated)]
use soroban_sdk::{contract, contractimpl, contracttype, symbol_short, token, Address, Env};

/// A single deposit held in the vault.
#[contracttype]
#[derive(Clone)]
pub struct Deposit {
    /// Address that deposited the tokens.
    pub owner: Address,
    /// Token contract address.
    pub token: Address,
    /// Amount locked.
    pub amount: i128,
    /// Unix timestamp before which `withdraw` is rejected.
    pub unlock_time: u64,
    /// True once the deposit has been withdrawn.
    pub withdrawn: bool,
}

#[contracttype]
enum DataKey {
    /// Next deposit ID (monotonically increasing u64).
    NextId,
    /// Deposit(id)
    Deposit(u64),
}

/// A time-locked vault.
///
/// Owners deposit tokens along with a future `unlock_time`.  Funds cannot be
/// retrieved until the ledger timestamp has passed that threshold.
#[contract]
#[derive(Default)]
pub struct Vault;

#[contractimpl]
impl Vault {
    /// Lock `amount` tokens until `unlock_time`.
    ///
    /// Returns the deposit ID assigned to this deposit.
    pub fn deposit(env: Env, owner: Address, token: Address, amount: i128, unlock_time: u64) -> u64 {
        owner.require_auth();
        if amount <= 0 {
            panic!("amount must be positive");
        }
        if unlock_time <= env.ledger().timestamp() {
            panic!("unlock_time must be in the future");
        }

        token::Client::new(&env, &token).transfer(&owner, env.current_contract_address(), &amount);

        let id: u64 = env
            .storage()
            .instance()
            .get(&DataKey::NextId)
            .unwrap_or(0u64);
        env.storage()
            .instance()
            .set(&DataKey::NextId, &(id + 1));
        env.storage().instance().set(
            &DataKey::Deposit(id),
            &Deposit {
                owner: owner.clone(),
                token,
                amount,
                unlock_time,
                withdrawn: false,
            },
        );

        env.events().publish((symbol_short!("deposit"),), (id, owner, amount));
        id
    }

    /// Withdraw locked funds once `unlock_time` has passed.
    ///
    /// Only the original owner may call this.
    pub fn withdraw(env: Env, id: u64) {
        let mut dep: Deposit = env
            .storage()
            .instance()
            .get(&DataKey::Deposit(id))
            .unwrap_or_else(|| panic!("deposit not found"));

        if dep.withdrawn {
            panic!("already withdrawn");
        }
        if env.ledger().timestamp() < dep.unlock_time {
            panic!("funds are still locked");
        }

        dep.owner.require_auth();
        dep.withdrawn = true;
        env.storage().instance().set(&DataKey::Deposit(id), &dep);

        token::Client::new(&env, &dep.token).transfer(
            &env.current_contract_address(),
            &dep.owner,
            &dep.amount,
        );

        env.events()
            .publish((symbol_short!("withdraw"),), (id, dep.owner, dep.amount));
    }

    /// Return the deposit record for the given ID.
    pub fn get_deposit(env: Env, id: u64) -> Deposit {
        env.storage()
            .instance()
            .get(&DataKey::Deposit(id))
            .unwrap_or_else(|| panic!("deposit not found"))
    }
}

#[cfg(test)]
mod test;
