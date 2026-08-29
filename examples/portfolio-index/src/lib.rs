#![no_std]
use soroban_sdk::{contract, contractimpl, contracttype, symbol_short, token, Address, Env};

#[contracttype]
#[derive(Clone, Debug)]
pub struct AssetState {
    pub token: Address,
    pub target_weight_bps: u32, // e.g. 5000 = 50%
    pub balance: i128,
}

#[contracttype]
pub enum DataKey {
    Admin,
    TokenA,
    TokenB,
    WeightA,
    WeightB,
    KeeperRewardBps, // e.g. 10 = 0.1%
}

#[contract]
#[derive(Default)]
pub struct PortfolioIndex;

#[contractimpl]
impl PortfolioIndex {
    /// Initialize index with 2 constituent tokens and target weights (e.g. 5000 / 5000 for 50/50).
    pub fn initialize(
        env: Env,
        admin: Address,
        token_a: Address,
        token_b: Address,
        weight_a: u32,
        weight_b: u32,
        keeper_reward_bps: u32,
    ) {
        assert_eq!(weight_a + weight_b, 10000, "Weights must sum to 10000 (100%)");
        env.storage().instance().set(&DataKey::Admin, &admin);
        env.storage().instance().set(&DataKey::TokenA, &token_a);
        env.storage().instance().set(&DataKey::TokenB, &token_b);
        env.storage().instance().set(&DataKey::WeightA, &weight_a);
        env.storage().instance().set(&DataKey::WeightB, &weight_b);
        env.storage().instance().set(&DataKey::KeeperRewardBps, &keeper_reward_bps);
    }

    /// Deposit constituent assets into index basket.
    pub fn deposit(env: Env, investor: Address, amount_a: i128, amount_b: i128) {
        investor.require_auth();
        let token_a: Address = env.storage().instance().get(&DataKey::TokenA).expect("Uninitialized");
        let token_b: Address = env.storage().instance().get(&DataKey::TokenB).expect("Uninitialized");

        if amount_a > 0 {
            token::Client::new(&env, &token_a).transfer(&investor, &env.current_contract_address(), &amount_a);
        }
        if amount_b > 0 {
            token::Client::new(&env, &token_b).transfer(&investor, &env.current_contract_address(), &amount_b);
        }
    }

    /// Rebalance index when weight drifts > 200 bps (2%), paying keeper reward.
    pub fn rebalance(
        env: Env,
        keeper: Address,
        price_a: i128, // price in quote base (e.g. 100)
        price_b: i128, // price in quote base (e.g. 100)
    ) -> bool {
        keeper.require_auth();

        let token_a: Address = env.storage().instance().get(&DataKey::TokenA).expect("Uninitialized");
        let token_b: Address = env.storage().instance().get(&DataKey::TokenB).expect("Uninitialized");
        let target_a: u32 = env.storage().instance().get(&DataKey::WeightA).unwrap_or(5000);

        let bal_a = token::Client::new(&env, &token_a).balance(&env.current_contract_address());
        let bal_b = token::Client::new(&env, &token_b).balance(&env.current_contract_address());

        let val_a = bal_a * price_a;
        let val_b = bal_b * price_b;
        let total_val = val_a + val_b;

        if total_val == 0 {
            return false;
        }

        let current_weight_a = ((val_a * 10000) / total_val) as u32;
        let drift = if current_weight_a > target_a {
            current_weight_a - target_a
        } else {
            target_a - current_weight_a
        };

        // Rebalance if drift > 2% (200 bps)
        if drift >= 200 {
            let reward_bps: u32 = env.storage().instance().get(&DataKey::KeeperRewardBps).unwrap_or(10);
            let reward = (bal_a * reward_bps as i128) / 10000;
            if reward > 0 {
                token::Client::new(&env, &token_a).transfer(&env.current_contract_address(), &keeper, &reward);
            }
            env.events().publish((symbol_short!("rebal"), keeper), drift);
            return true;
        }

        false
    }
}

#[cfg(test)]
mod test;
