#![no_std]
use soroban_sdk::{contract, contractimpl, contracttype, symbol_short, Address, Env};

#[contracttype]
enum DataKey {
    LiquidityPool(Address, Address),
    UserLiquidity(Address, Address, Address),
}

/// Minimal DEX (Decentralized Exchange) contract.
#[contract]
pub struct DEX;

#[contractimpl]
impl DEX {
    /// Add liquidity to a trading pair. Returns liquidity tokens received.
    pub fn add_liquidity(
        env: Env,
        token_a: Address,
        token_b: Address,
        amount_a: i128,
        amount_b: i128,
        user: Address,
    ) -> i128 {
        user.require_auth();
        if amount_a <= 0 || amount_b <= 0 {
            panic!("amounts must be positive");
        }

        let pool_key = DataKey::LiquidityPool(token_a.clone(), token_b.clone());
        let (pool_a, pool_b): (i128, i128) = env
            .storage()
            .instance()
            .get(&pool_key)
            .unwrap_or((0, 0));

        let liquidity = if pool_a == 0 {
            (amount_a * amount_b) as i128
        } else {
            ((amount_a * pool_b) / pool_a).min(amount_b)
        };

        env.storage()
            .instance()
            .set(&pool_key, &(pool_a + amount_a, pool_b + amount_b));

        let user_key = DataKey::UserLiquidity(user.clone(), token_a, token_b);
        let user_liq: i128 = env.storage().instance().get(&user_key).unwrap_or(0);
        env.storage()
            .instance()
            .set(&user_key, &(user_liq + liquidity));

        env.events()
            .publish((symbol_short!("addliq"), user), (amount_a, amount_b, liquidity));

        liquidity
    }

    /// Swap token_in for token_out. Returns amount of token_out.
    pub fn swap(env: Env, token_in: Address, token_out: Address, amount_in: i128, user: Address) -> i128 {
        user.require_auth();
        if amount_in <= 0 {
            panic!("amount must be positive");
        }

        let pool_key = DataKey::LiquidityPool(token_in.clone(), token_out.clone());
        let (pool_in, pool_out): (i128, i128) = env
            .storage()
            .instance()
            .get(&pool_key)
            .unwrap_or_else(|| panic!("pool does not exist"));

        let amount_out = (amount_in * pool_out) / (pool_in + amount_in);

        env.storage()
            .instance()
            .set(&pool_key, &(pool_in + amount_in, pool_out - amount_out));

        env.events()
            .publish((symbol_short!("swap"), user), (amount_in, amount_out));

        amount_out
    }

    /// Get current price of token_out in terms of token_in.
    pub fn get_price(env: Env, token_in: Address, token_out: Address) -> i128 {
        let pool_key = DataKey::LiquidityPool(token_in, token_out);
        let (pool_in, pool_out): (i128, i128) = env
            .storage()
            .instance()
            .get(&pool_key)
            .unwrap_or_else(|| panic!("pool does not exist"));

        if pool_in == 0 {
            panic!("pool is empty");
        }

        pool_out / pool_in
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_dex_creation() {
        let dex = DEX;
        assert_eq!(std::mem::size_of_val(&dex), 0);
    }
}
