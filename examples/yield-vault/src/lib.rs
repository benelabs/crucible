#![no_std]
#![allow(deprecated)]

use soroban_sdk::{contract, contractimpl, contracttype, symbol_short, token, Address, Env};

const DEAD_SHARES: i128 = 1_000;

#[contracttype]
enum DataKey {
    AssetToken,
    TotalAssets,
    TotalShares,
    Shares(Address),
    Initialized,
}

/// ERC-4626 style automated yield compounding vault contract on Soroban.
#[contract]
#[derive(Default)]
pub struct YieldVault;

#[contractimpl]
impl YieldVault {
    /// Initialize the vault with the underlying asset token and dead-shares minting protection.
    pub fn initialize(env: Env, asset_token: Address) {
        if env.storage().instance().has(&DataKey::Initialized) {
            panic!("already initialized");
        }

        env.storage().instance().set(&DataKey::AssetToken, &asset_token);
        env.storage().instance().set(&DataKey::TotalAssets, &DEAD_SHARES);
        env.storage().instance().set(&DataKey::TotalShares, &DEAD_SHARES);
        
        // Mint dead shares to contract address to prevent donation/inflation attacks
        let contract_addr = env.current_contract_address();
        env.storage().instance().set(&DataKey::Shares(contract_addr), &DEAD_SHARES);
        env.storage().instance().set(&DataKey::Initialized, &true);

        env.events().publish((symbol_short!("init"),), (asset_token, DEAD_SHARES));
    }

    pub fn total_assets(env: Env) -> i128 {
        env.storage()
            .instance()
            .get(&DataKey::TotalAssets)
            .unwrap_or(0)
    }

    pub fn total_shares(env: Env) -> i128 {
        env.storage()
            .instance()
            .get(&DataKey::TotalShares)
            .unwrap_or(0)
    }

    pub fn balance_of(env: Env, account: Address) -> i128 {
        env.storage()
            .instance()
            .get(&DataKey::Shares(account))
            .unwrap_or(0)
    }

    pub fn convert_to_shares(env: Env, assets: i128) -> i128 {
        let total_assets = Self::total_assets(env.clone());
        let total_shares = Self::total_shares(env);
        if total_assets == 0 || total_shares == 0 {
            assets
        } else {
            (assets * total_shares) / total_assets
        }
    }

    pub fn convert_to_assets(env: Env, shares: i128) -> i128 {
        let total_assets = Self::total_assets(env.clone());
        let total_shares = Self::total_shares(env);
        if total_shares == 0 {
            shares
        } else {
            (shares * total_assets) / total_shares
        }
    }

    /// Deposit assets into the vault and mint corresponding vault shares.
    pub fn deposit(env: Env, caller: Address, assets: i128, receiver: Address) -> i128 {
        caller.require_auth();
        if assets <= 0 {
            panic!("assets must be positive");
        }

        let shares = Self::convert_to_shares(env.clone(), assets);
        if shares <= 0 {
            panic!("zero shares minted");
        }

        let asset_token: Address = env.storage().instance().get(&DataKey::AssetToken).unwrap();
        token::Client::new(&env, &asset_token).transfer(&caller, env.current_contract_address(), &assets);

        let cur_assets = Self::total_assets(env.clone());
        let cur_shares = Self::total_shares(env.clone());
        let receiver_shares = Self::balance_of(env.clone(), receiver.clone());

        env.storage().instance().set(&DataKey::TotalAssets, &(cur_assets + assets));
        env.storage().instance().set(&DataKey::TotalShares, &(cur_shares + shares));
        env.storage().instance().set(&DataKey::Shares(receiver.clone()), &(receiver_shares + shares));

        env.events().publish((symbol_short!("deposit"),), (caller, receiver, assets, shares));
        shares
    }

    /// Mint exact vault shares by depositing the necessary underlying assets.
    pub fn mint(env: Env, caller: Address, shares: i128, receiver: Address) -> i128 {
        caller.require_auth();
        if shares <= 0 {
            panic!("shares must be positive");
        }

        let assets = Self::convert_to_assets(env.clone(), shares);
        let asset_token: Address = env.storage().instance().get(&DataKey::AssetToken).unwrap();
        token::Client::new(&env, &asset_token).transfer(&caller, env.current_contract_address(), &assets);

        let cur_assets = Self::total_assets(env.clone());
        let cur_shares = Self::total_shares(env.clone());
        let receiver_shares = Self::balance_of(env.clone(), receiver.clone());

        env.storage().instance().set(&DataKey::TotalAssets, &(cur_assets + assets));
        env.storage().instance().set(&DataKey::TotalShares, &(cur_shares + shares));
        env.storage().instance().set(&DataKey::Shares(receiver.clone()), &(receiver_shares + shares));

        env.events().publish((symbol_short!("mint"),), (caller, receiver, assets, shares));
        assets
    }

    /// Withdraw assets from vault by burning equivalent shares from owner.
    pub fn withdraw(env: Env, caller: Address, assets: i128, receiver: Address, owner: Address) -> i128 {
        owner.require_auth();
        if assets <= 0 {
            panic!("assets must be positive");
        }

        let shares = Self::convert_to_shares(env.clone(), assets);
        let owner_shares = Self::balance_of(env.clone(), owner.clone());
        if owner_shares < shares {
            panic!("insufficient shares balance");
        }

        let asset_token: Address = env.storage().instance().get(&DataKey::AssetToken).unwrap();
        token::Client::new(&env, &asset_token).transfer(&env.current_contract_address(), &receiver, &assets);

        let cur_assets = Self::total_assets(env.clone());
        let cur_shares = Self::total_shares(env.clone());

        env.storage().instance().set(&DataKey::TotalAssets, &(cur_assets - assets));
        env.storage().instance().set(&DataKey::TotalShares, &(cur_shares - shares));
        env.storage().instance().set(&DataKey::Shares(owner.clone()), &(owner_shares - shares));

        env.events().publish((symbol_short!("withdraw"),), (caller, owner, receiver, assets, shares));
        shares
    }

    /// Redeem vault shares for underlying assets.
    pub fn redeem(env: Env, caller: Address, shares: i128, receiver: Address, owner: Address) -> i128 {
        owner.require_auth();
        if shares <= 0 {
            panic!("shares must be positive");
        }

        let owner_shares = Self::balance_of(env.clone(), owner.clone());
        if owner_shares < shares {
            panic!("insufficient shares balance");
        }

        let assets = Self::convert_to_assets(env.clone(), shares);
        let asset_token: Address = env.storage().instance().get(&DataKey::AssetToken).unwrap();
        token::Client::new(&env, &asset_token).transfer(&env.current_contract_address(), &receiver, &assets);

        let cur_assets = Self::total_assets(env.clone());
        let cur_shares = Self::total_shares(env.clone());

        env.storage().instance().set(&DataKey::TotalAssets, &(cur_assets - assets));
        env.storage().instance().set(&DataKey::TotalShares, &(cur_shares - shares));
        env.storage().instance().set(&DataKey::Shares(owner.clone()), &(owner_shares - shares));

        env.events().publish((symbol_short!("redeem"),), (caller, owner, receiver, assets, shares));
        assets
    }

    /// Auto-compounds yield generated from underlying staking protocols.
    pub fn compound_yield(env: Env, yield_amount: i128) {
        if yield_amount <= 0 {
            panic!("yield must be positive");
        }

        let cur_assets = Self::total_assets(env.clone());
        env.storage().instance().set(&DataKey::TotalAssets, &(cur_assets + yield_amount));
        env.events().publish((symbol_short!("compound"),), (yield_amount,));
    }
}

#[cfg(test)]
mod test;
