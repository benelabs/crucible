#![no_std]
use soroban_sdk::{contract, contractimpl, contracttype, symbol_short, token, Address, Env};

/// A single deposit held in the vault.
#[contracttype]
#[derive(Clone)]
pub struct Deposit {
    /// Owner who deposited and may withdraw.
    pub owner: Address,
    /// Token address.
    pub token: Address,
    /// Amount deposited.
    pub amount: i128,
    /// Unix timestamp after which the owner may withdraw.
    pub unlock_time: u64,
    /// Whether the deposit has been withdrawn.
    pub withdrawn: bool,
}

#[contracttype]
enum DataKey {
    /// Next deposit ID counter.
    NextId,
    /// Deposit keyed by u64 ID.
    Deposit(u64),
}

/// A time-locked vault contract.
///
/// Users deposit tokens that are locked until a specified unlock time.
/// After the unlock time passes, only the original depositor may withdraw.
#[contract]
#[derive(Default)]
pub struct TimeLockedVault;

#[contractimpl]
impl TimeLockedVault {
    /// Deposit `amount` of `token` locked until `unlock_time`.
    ///
    /// Returns the deposit ID.
    pub fn deposit(
        env: Env,
        owner: Address,
        token: Address,
        amount: i128,
        unlock_time: u64,
    ) -> u64 {
        if amount <= 0 {
            panic!("amount must be positive");
        }
        let now = env.ledger().timestamp();
        if unlock_time <= now {
            panic!("unlock_time must be in the future");
        }
        owner.require_auth();

        token::TokenClient::new(&env, &token).transfer(&owner, &env.current_contract_address(), &amount);

        let id: u64 = env
            .storage()
            .instance()
            .get(&DataKey::NextId)
            .unwrap_or(0u64);

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
        env.storage().instance().set(&DataKey::NextId, &(id + 1));

        env.events()
            .publish((symbol_short!("deposited"),), (owner, id, amount));

        id
    }

    /// Withdraw a deposit after its unlock time.
    ///
    /// Only the original depositor may call this.
    pub fn withdraw(env: Env, id: u64) {
        let mut dep: Deposit = env
            .storage()
            .instance()
            .get(&DataKey::Deposit(id))
            .expect("deposit not found");

        if dep.withdrawn {
            panic!("already withdrawn");
        }
        let now = env.ledger().timestamp();
        if now < dep.unlock_time {
            panic!("time lock has not expired");
        }
        dep.owner.require_auth();

        dep.withdrawn = true;
        env.storage().instance().set(&DataKey::Deposit(id), &dep);

        token::TokenClient::new(&env, &dep.token).transfer(
            &env.current_contract_address(),
            &dep.owner,
            &dep.amount,
        );

        env.events()
            .publish((symbol_short!("withdrew"),), (dep.owner, id, dep.amount));
    }

    /// Return the deposit record for `id`.
    pub fn get_deposit(env: Env, id: u64) -> Deposit {
        env.storage()
            .instance()
            .get(&DataKey::Deposit(id))
            .expect("deposit not found")
    }
}

#[cfg(test)]
mod test;
