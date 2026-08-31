#![no_std]

use soroban_sdk::{
    contract, contractimpl, contracttype, Address, Env,
};

#[contracttype]
#[derive(Clone)]
pub enum DataKey {
    TokenA,
    TokenB,
    ReserveA,
    ReserveB,
    TotalShares,
    Shares(Address),
    CumulativePriceA,
    CumulativePriceB,
    LastTimestamp,
}

#[contract]
pub struct AmmPoolContract;

#[contractimpl]
impl AmmPoolContract {
    /// Initialize the AMM liquidity pool with pair tokens.
    pub fn initialize(env: Env, token_a: Address, token_b: Address) {
        if env.storage().instance().has(&DataKey::TokenA) {
            panic!("already initialized");
        }
        env.storage().instance().set(&DataKey::TokenA, &token_a);
        env.storage().instance().set(&DataKey::TokenB, &token_b);
        env.storage().instance().set(&DataKey::ReserveA, &0i128);
        env.storage().instance().set(&DataKey::ReserveB, &0i128);
        env.storage().instance().set(&DataKey::TotalShares, &0i128);
        env.storage().instance().set(&DataKey::CumulativePriceA, &0i128);
        env.storage().instance().set(&DataKey::CumulativePriceB, &0i128);
        env.storage().instance().set(&DataKey::LastTimestamp, &env.ledger().timestamp());
    }

    /// Deposit liquidity into the constant-product pool.
    pub fn deposit(env: Env, to: Address, amount_a: i128, amount_b: i128) -> i128 {
        to.require_auth();
        if amount_a <= 0 || amount_b <= 0 {
            panic!("invalid deposit amount");
        }

        Self::update_twap(&env);

        let reserve_a: i128 = env.storage().instance().get(&DataKey::ReserveA).unwrap_or(0);
        let reserve_b: i128 = env.storage().instance().get(&DataKey::ReserveB).unwrap_or(0);
        let total_shares: i128 = env.storage().instance().get(&DataKey::TotalShares).unwrap_or(0);

        let shares: i128 = if total_shares == 0 {
            (amount_a * amount_b).isqrt()
        } else {
            let share_a = (amount_a * total_shares) / reserve_a;
            let share_b = (amount_b * total_shares) / reserve_b;
            if share_a < share_b { share_a } else { share_b }
        };

        if shares <= 0 {
            panic!("insufficient liquidity minted");
        }

        let new_reserve_a = reserve_a + amount_a;
        let new_reserve_b = reserve_b + amount_b;
        let new_total_shares = total_shares + shares;

        let user_shares: i128 = env.storage().instance().get(&DataKey::Shares(to.clone())).unwrap_or(0);
        env.storage().instance().set(&DataKey::Shares(to), &(user_shares + shares));
        env.storage().instance().set(&DataKey::ReserveA, &new_reserve_a);
        env.storage().instance().set(&DataKey::ReserveB, &new_reserve_b);
        env.storage().instance().set(&DataKey::TotalShares, &new_total_shares);

        shares
    }

    /// Execute constant-product swap with deadline & slippage protection.
    pub fn swap(
        env: Env,
        to: Address,
        buy_a: bool,
        amount_in: i128,
        min_amount_out: i128,
        deadline: u64,
    ) -> i128 {
        to.require_auth();

        if env.ledger().timestamp() > deadline {
            panic!("transaction expired past deadline");
        }
        if amount_in <= 0 {
            panic!("amount_in must be positive");
        }

        Self::update_twap(&env);

        let reserve_a: i128 = env.storage().instance().get(&DataKey::ReserveA).unwrap();
        let reserve_b: i128 = env.storage().instance().get(&DataKey::ReserveB).unwrap();

        // 0.3% fee constant-product swap formula
        let fee_multiplier = 997i128;
        let fee_denominator = 1000i128;

        let (amount_out, new_reserve_a, new_reserve_b) = if buy_a {
            // Pay B, receive A
            let amount_in_with_fee = amount_in * fee_multiplier;
            let numerator = amount_in_with_fee * reserve_a;
            let denominator = (reserve_b * fee_denominator) + amount_in_with_fee;
            let out = numerator / denominator;
            (out, reserve_a - out, reserve_b + amount_in)
        } else {
            // Pay A, receive B
            let amount_in_with_fee = amount_in * fee_multiplier;
            let numerator = amount_in_with_fee * reserve_b;
            let denominator = (reserve_a * fee_denominator) + amount_in_with_fee;
            let out = numerator / denominator;
            (out, reserve_a + amount_in, reserve_b - out)
        };

        if amount_out < min_amount_out {
            panic!("slippage limit exceeded");
        }

        env.storage().instance().set(&DataKey::ReserveA, &new_reserve_a);
        env.storage().instance().set(&DataKey::ReserveB, &new_reserve_b);

        amount_out
    }

    /// Retrieve accumulated TWAP prices.
    pub fn get_cumulative_prices(env: Env) -> (i128, i128) {
        let price_a: i128 = env.storage().instance().get(&DataKey::CumulativePriceA).unwrap_or(0);
        let price_b: i128 = env.storage().instance().get(&DataKey::CumulativePriceB).unwrap_or(0);
        (price_a, price_b)
    }

    /// Retrieve current pool reserves.
    pub fn get_reserves(env: Env) -> (i128, i128) {
        let reserve_a: i128 = env.storage().instance().get(&DataKey::ReserveA).unwrap_or(0);
        let reserve_b: i128 = env.storage().instance().get(&DataKey::ReserveB).unwrap_or(0);
        (reserve_a, reserve_b)
    }

    fn update_twap(env: &Env) {
        let last_time: u64 = env.storage().instance().get(&DataKey::LastTimestamp).unwrap_or(0);
        let current_time = env.ledger().timestamp();
        let time_elapsed = current_time.saturating_sub(last_time);

        if time_elapsed > 0 {
            let reserve_a: i128 = env.storage().instance().get(&DataKey::ReserveA).unwrap_or(0);
            let reserve_b: i128 = env.storage().instance().get(&DataKey::ReserveB).unwrap_or(0);

            if reserve_a > 0 && reserve_b > 0 {
                let price_a = (reserve_b * 1000) / reserve_a;
                let price_b = (reserve_a * 1000) / reserve_b;

                let cum_a: i128 = env.storage().instance().get(&DataKey::CumulativePriceA).unwrap_or(0);
                let cum_b: i128 = env.storage().instance().get(&DataKey::CumulativePriceB).unwrap_or(0);

                env.storage().instance().set(&DataKey::CumulativePriceA, &(cum_a + price_a * (time_elapsed as i128)));
                env.storage().instance().set(&DataKey::CumulativePriceB, &(cum_b + price_b * (time_elapsed as i128)));
                env.storage().instance().set(&DataKey::LastTimestamp, &current_time);
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use soroban_sdk::{testutils::Address as _, Address, Env};

    #[test]
    fn test_amm_deposit_and_swap_maintains_k_invariant() {
        let env = Env::default();
        let contract_id = env.register(AmmPoolContract, ());
        let client = AmmPoolContractClient::new(&env, &contract_id);

        let user = Address::generate(&env);
        let token_a = Address::generate(&env);
        let token_b = Address::generate(&env);

        env.mock_all_auths();

        client.initialize(&token_a, &token_b);

        // Initial deposit: 1000 A, 1000 B
        let shares = client.deposit(&user, &1000, &1000);
        assert_eq!(shares, 1000);

        let (res_a_start, res_b_start) = client.get_reserves();
        let k_start = res_a_start * res_b_start;
        assert_eq!(k_start, 1_000_000);

        // Swap: Swap 100 B for A with min_amount_out = 80 and deadline = 1000
        let amount_out = client.swap(&user, &true, &100, &80, &1000);
        assert!(amount_out >= 80);

        let (res_a_end, res_b_end) = client.get_reserves();
        let k_end = res_a_end * res_b_end;

        // K invariant must be maintained or increased due to 0.3% LP fees
        assert!(k_end >= k_start);
    }
}
