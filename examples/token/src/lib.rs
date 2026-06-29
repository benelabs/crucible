#![no_std]
use soroban_sdk::{contract, contractimpl, contracttype, symbol_short, Address, Env};

#[contracttype]
#[derive(Clone)]
struct AllowanceKey {
    from: Address,
    spender: Address,
}

#[contracttype]
enum DataKey {
    Admin,
    Balance(Address),
    Allowance(AllowanceKey),
}

/// A simple mintable token contract.
///
/// Supports mint (admin-only), transfer, burn, and an allowance-based
/// `transfer_from` flow. Emits events for every mutating operation.
#[contract]
#[derive(Default)]
pub struct Token;

#[contractimpl]
impl Token {
    /// Initialize the token with an admin address.
    pub fn initialize(env: Env, admin: Address) {
        env.storage().instance().set(&DataKey::Admin, &admin);
    }

    /// Mint `amount` tokens to `to`. Admin only.
    pub fn mint(env: Env, to: Address, amount: i128) {
        let admin: Address = env.storage().instance().get(&DataKey::Admin).unwrap();
        admin.require_auth();
        if amount <= 0 {
            panic!("mint amount must be positive");
        }
        let bal: i128 = env
            .storage()
            .instance()
            .get(&DataKey::Balance(to.clone()))
            .unwrap_or(0);
        env.storage()
            .instance()
            .set(&DataKey::Balance(to.clone()), &(bal + amount));
        env.events().publish((symbol_short!("mint"), to), amount);
    }

    /// Transfer `amount` tokens from `from` to `to`. Requires `from` auth.
    pub fn transfer(env: Env, from: Address, to: Address, amount: i128) {
        from.require_auth();
        let from_bal: i128 = env
            .storage()
            .instance()
            .get(&DataKey::Balance(from.clone()))
            .unwrap_or(0);
        if from_bal < amount {
            panic!("insufficient balance");
        }
        let to_bal: i128 = env
            .storage()
            .instance()
            .get(&DataKey::Balance(to.clone()))
            .unwrap_or(0);
        env.storage()
            .instance()
            .set(&DataKey::Balance(from.clone()), &(from_bal - amount));
        env.storage()
            .instance()
            .set(&DataKey::Balance(to.clone()), &(to_bal + amount));
        env.events()
            .publish((symbol_short!("xfer"), from, to), amount);
    }

    /// Burn `amount` tokens from `from`. Requires `from` auth.
    pub fn burn(env: Env, from: Address, amount: i128) {
        from.require_auth();
        let bal: i128 = env
            .storage()
            .instance()
            .get(&DataKey::Balance(from.clone()))
            .unwrap_or(0);
        if bal < amount {
            panic!("insufficient balance");
        }
        env.storage()
            .instance()
            .set(&DataKey::Balance(from.clone()), &(bal - amount));
        env.events().publish((symbol_short!("burn"), from), amount);
    }

    /// Approve `spender` to spend up to `amount` tokens on behalf of `from`.
    pub fn approve(env: Env, from: Address, spender: Address, amount: i128) {
        from.require_auth();
        env.storage().instance().set(
            &DataKey::Allowance(AllowanceKey {
                from: from.clone(),
                spender: spender.clone(),
            }),
            &amount,
        );
    }

    /// Transfer `amount` from `from` to `to` using `spender`'s allowance.
    pub fn transfer_from(env: Env, spender: Address, from: Address, to: Address, amount: i128) {
        spender.require_auth();
        let key = DataKey::Allowance(AllowanceKey {
            from: from.clone(),
            spender: spender.clone(),
        });
        let allowance: i128 = env.storage().instance().get(&key).unwrap_or(0);
        if allowance < amount {
            panic!("insufficient allowance");
        }
        env.storage().instance().set(&key, &(allowance - amount));
        let from_bal: i128 = env
            .storage()
            .instance()
            .get(&DataKey::Balance(from.clone()))
            .unwrap_or(0);
        if from_bal < amount {
            panic!("insufficient balance");
        }
        let to_bal: i128 = env
            .storage()
            .instance()
            .get(&DataKey::Balance(to.clone()))
            .unwrap_or(0);
        env.storage()
            .instance()
            .set(&DataKey::Balance(from.clone()), &(from_bal - amount));
        env.storage()
            .instance()
            .set(&DataKey::Balance(to.clone()), &(to_bal + amount));
    }

    /// Return the token balance of `account`.
    pub fn balance(env: Env, account: Address) -> i128 {
        env.storage()
            .instance()
            .get(&DataKey::Balance(account))
            .unwrap_or(0)
    }

    /// Return the allowance of `spender` on behalf of `from`.
    pub fn allowance(env: Env, from: Address, spender: Address) -> i128 {
        env.storage()
            .instance()
            .get(&DataKey::Allowance(AllowanceKey { from, spender }))
            .unwrap_or(0)
    }
}

#[cfg(test)]
mod test;
