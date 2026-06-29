#![no_std]
use soroban_sdk::{contract, contractimpl, contracttype, symbol_short, token, Address, Env};

/// Per-staker record.
#[contracttype]
#[derive(Clone)]
pub struct StakeInfo {
    /// Amount of tokens staked by this address.
    pub amount: i128,
    /// Address this staker has delegated their voting power to.
    /// `None` means the staker votes for themselves.
    pub delegate: Option<Address>,
}

#[contracttype]
enum DataKey {
    /// Admin address (set at initialization).
    Admin,
    /// Staking token address.
    Token,
    /// StakeInfo keyed by staker address.
    Stake(Address),
    /// Accumulated voting power keyed by delegate address.
    VotingPower(Address),
}

/// A staking contract with delegation.
///
/// Workflow:
/// 1. Admin calls `initialize` with the staking token address.
/// 2. Users call `stake` to lock tokens and optionally delegate voting power.
/// 3. Users call `delegate` to change their delegate at any time.
/// 4. Users call `unstake` to withdraw their tokens (removes delegation too).
/// 5. Anyone can query `voting_power` to read the delegated power of an address.
#[contract]
#[derive(Default)]
pub struct Staking;

#[contractimpl]
impl Staking {
    /// Initialize the contract with the staking token.
    pub fn initialize(env: Env, admin: Address, token: Address) {
        if env.storage().instance().has(&DataKey::Admin) {
            panic!("already initialized");
        }
        admin.require_auth();
        env.storage().instance().set(&DataKey::Admin, &admin);
        env.storage().instance().set(&DataKey::Token, &token);
    }

    /// Stake `amount` tokens, optionally delegating voting power to `delegate`.
    ///
    /// If the caller already has a stake, the new amount is added on top.
    pub fn stake(env: Env, staker: Address, amount: i128, delegate: Option<Address>) {
        if amount <= 0 {
            panic!("amount must be positive");
        }
        staker.require_auth();

        let token_addr: Address = env.storage().instance().get(&DataKey::Token).unwrap();
        token::TokenClient::new(&env, &token_addr).transfer(
            &staker,
            &env.current_contract_address(),
            &amount,
        );

        let mut info: StakeInfo = env
            .storage()
            .instance()
            .get(&DataKey::Stake(staker.clone()))
            .unwrap_or(StakeInfo {
                amount: 0,
                delegate: None,
            });

        // Remove old voting power before updating.
        let old_delegate = info.delegate.clone().unwrap_or(staker.clone());
        Self::adjust_voting_power(&env, &old_delegate, -(info.amount));

        info.amount += amount;
        info.delegate = delegate.clone();

        let new_delegate = delegate.unwrap_or(staker.clone());
        Self::adjust_voting_power(&env, &new_delegate, info.amount);

        env.storage()
            .instance()
            .set(&DataKey::Stake(staker.clone()), &info);

        env.events()
            .publish((symbol_short!("staked"),), (staker, amount));
    }

    /// Change the delegate for the caller's stake.
    pub fn delegate(env: Env, staker: Address, new_delegate: Address) {
        staker.require_auth();

        let mut info: StakeInfo = env
            .storage()
            .instance()
            .get(&DataKey::Stake(staker.clone()))
            .expect("no stake found");

        if info.amount == 0 {
            panic!("no active stake");
        }

        // Remove power from old delegate.
        let old_delegate = info.delegate.clone().unwrap_or(staker.clone());
        Self::adjust_voting_power(&env, &old_delegate, -(info.amount));

        // Add power to new delegate.
        Self::adjust_voting_power(&env, &new_delegate, info.amount);

        info.delegate = Some(new_delegate.clone());
        env.storage()
            .instance()
            .set(&DataKey::Stake(staker.clone()), &info);

        env.events()
            .publish((symbol_short!("delegated"),), (staker, new_delegate));
    }

    /// Unstake all tokens and remove delegation.
    pub fn unstake(env: Env, staker: Address) {
        staker.require_auth();

        let info: StakeInfo = env
            .storage()
            .instance()
            .get(&DataKey::Stake(staker.clone()))
            .expect("no stake found");

        if info.amount == 0 {
            panic!("no active stake");
        }

        // Remove voting power from delegate.
        let delegate = info.delegate.clone().unwrap_or(staker.clone());
        Self::adjust_voting_power(&env, &delegate, -(info.amount));

        env.storage()
            .instance()
            .remove(&DataKey::Stake(staker.clone()));

        let token_addr: Address = env.storage().instance().get(&DataKey::Token).unwrap();
        token::TokenClient::new(&env, &token_addr).transfer(
            &env.current_contract_address(),
            &staker,
            &info.amount,
        );

        env.events()
            .publish((symbol_short!("unstaked"),), (staker, info.amount));
    }

    /// Return the stake info for `staker`.
    pub fn get_stake(env: Env, staker: Address) -> StakeInfo {
        env.storage()
            .instance()
            .get(&DataKey::Stake(staker))
            .unwrap_or(StakeInfo {
                amount: 0,
                delegate: None,
            })
    }

    /// Return the accumulated voting power of `account`.
    pub fn voting_power(env: Env, account: Address) -> i128 {
        env.storage()
            .instance()
            .get(&DataKey::VotingPower(account))
            .unwrap_or(0)
    }

    // -----------------------------------------------------------------------
    // Internal helpers
    // -----------------------------------------------------------------------

    fn adjust_voting_power(env: &Env, account: &Address, delta: i128) {
        if delta == 0 {
            return;
        }
        let current: i128 = env
            .storage()
            .instance()
            .get(&DataKey::VotingPower(account.clone()))
            .unwrap_or(0);
        let new_power = (current + delta).max(0);
        env.storage()
            .instance()
            .set(&DataKey::VotingPower(account.clone()), &new_power);
    }
}

#[cfg(test)]
mod test;
