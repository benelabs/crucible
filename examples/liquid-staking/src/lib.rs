// Location: examples/liquid-staking/src/lib.rs // Production requirement: Multi-Asset Liquidity Staking Derivative (LSD) Protocol
#![no_std]
use soroban_sdk::{contract, contractimpl, contracttype, symbol_short, token, Address, Env};

/// Precision scale for the sXLM exchange rate (assets per share).
const RATE_SCALE: i128 = 1_000_000_000;

/// Pending unbonding request waiting for the cooldown window to elapse.
#[contracttype]
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct UnbondingRequest {
    /// Request owner.
    pub owner: Address,
    /// Underlying assets owed after cooldown.
    pub assets: i128,
    /// Ledger timestamp when withdrawal becomes available.
    pub unlock_at: u64,
    /// Whether the request has already been claimed.
    pub claimed: bool,
}

#[contracttype]
#[derive(Clone)]
enum DataKey {
    Admin,
    Asset,
    TotalPooled,
    TotalShares,
    Shares(Address),
    Cooldown,
    NextUnbondId,
    Unbond(u32),
    Initialized,
}

/// Liquid staking protocol that pools deposits, stakes with validators,
/// and mints tradeable receipt tokens (sXLM).
///
/// Exchange rate grows as staking rewards are accrued into `total_pooled`
/// without minting additional shares, compounding value for sXLM holders.
#[contract]
#[derive(Default)]
pub struct LiquidStaking;

#[contractimpl]
impl LiquidStaking {
    /// Initialize the LSD pool with the underlying asset and unbonding cooldown.
    pub fn initialize(env: Env, admin: Address, asset: Address, cooldown_secs: u64) {
        if env.storage().instance().has(&DataKey::Initialized) {
            panic!("already initialized");
        }
        admin.require_auth();
        if cooldown_secs == 0 {
            panic!("cooldown must be positive");
        }

        env.storage().instance().set(&DataKey::Admin, &admin);
        env.storage().instance().set(&DataKey::Asset, &asset);
        env.storage().instance().set(&DataKey::TotalPooled, &0i128);
        env.storage().instance().set(&DataKey::TotalShares, &0i128);
        env.storage().instance().set(&DataKey::Cooldown, &cooldown_secs);
        env.storage().instance().set(&DataKey::NextUnbondId, &0u32);
        env.storage().instance().set(&DataKey::Initialized, &true);

        env.events()
            .publish((symbol_short!("init"),), (admin, asset, cooldown_secs));
    }

    /// Deposit underlying assets and mint sXLM shares at the current exchange rate.
    pub fn deposit(env: Env, depositor: Address, assets: i128) -> i128 {
        depositor.require_auth();
        if assets <= 0 {
            panic!("assets must be positive");
        }

        let shares = Self::convert_to_shares(env.clone(), assets);
        if shares <= 0 {
            panic!("zero shares minted");
        }

        let asset: Address = env.storage().instance().get(&DataKey::Asset).unwrap();
        token::Client::new(&env, &asset).transfer(
            &depositor,
            &env.current_contract_address(),
            &assets,
        );

        let total_pooled = Self::total_pooled(env.clone());
        let total_shares = Self::total_shares(env.clone());
        let owner_shares = Self::balance_of(env.clone(), depositor.clone());

        env.storage()
            .instance()
            .set(&DataKey::TotalPooled, &(total_pooled + assets));
        env.storage()
            .instance()
            .set(&DataKey::TotalShares, &(total_shares + shares));
        env.storage()
            .instance()
            .set(&DataKey::Shares(depositor.clone()), &(owner_shares + shares));

        env.events()
            .publish((symbol_short!("deposit"),), (depositor, assets, shares));
        shares
    }

    /// Burn sXLM and enqueue an unbonding request that unlocks after the cooldown.
    pub fn request_unbond(env: Env, owner: Address, shares: i128) -> u32 {
        owner.require_auth();
        if shares <= 0 {
            panic!("shares must be positive");
        }

        let owner_shares = Self::balance_of(env.clone(), owner.clone());
        if owner_shares < shares {
            panic!("insufficient shares");
        }

        let assets = Self::convert_to_assets(env.clone(), shares);
        if assets <= 0 {
            panic!("zero assets unbonded");
        }

        let total_pooled = Self::total_pooled(env.clone());
        let total_shares = Self::total_shares(env.clone());
        env.storage()
            .instance()
            .set(&DataKey::TotalPooled, &(total_pooled - assets));
        env.storage()
            .instance()
            .set(&DataKey::TotalShares, &(total_shares - shares));
        env.storage()
            .instance()
            .set(&DataKey::Shares(owner.clone()), &(owner_shares - shares));

        let cooldown: u64 = env.storage().instance().get(&DataKey::Cooldown).unwrap();
        let unlock_at = env.ledger().timestamp().saturating_add(cooldown);
        let id: u32 = env.storage().instance().get(&DataKey::NextUnbondId).unwrap();

        let request = UnbondingRequest {
            owner: owner.clone(),
            assets,
            unlock_at,
            claimed: false,
        };
        env.storage().instance().set(&DataKey::Unbond(id), &request);
        env.storage()
            .instance()
            .set(&DataKey::NextUnbondId, &(id + 1));

        env.events().publish(
            (symbol_short!("unbond"),),
            (owner, id, shares, assets, unlock_at),
        );
        id
    }

    /// Withdraw assets from a matured unbonding request.
    pub fn withdraw(env: Env, owner: Address, unbond_id: u32) -> i128 {
        owner.require_auth();

        let mut request: UnbondingRequest = env
            .storage()
            .instance()
            .get(&DataKey::Unbond(unbond_id))
            .expect("unknown unbonding request");

        if request.owner != owner {
            panic!("not request owner");
        }
        if request.claimed {
            panic!("already claimed");
        }
        if env.ledger().timestamp() < request.unlock_at {
            panic!("cooldown active");
        }

        request.claimed = true;
        env.storage()
            .instance()
            .set(&DataKey::Unbond(unbond_id), &request);

        let asset: Address = env.storage().instance().get(&DataKey::Asset).unwrap();
        token::Client::new(&env, &asset).transfer(
            &env.current_contract_address(),
            &owner,
            &request.assets,
        );

        env.events().publish(
            (symbol_short!("withdraw"),),
            (owner, unbond_id, request.assets),
        );
        request.assets
    }

    /// Accrue validator staking rewards into the pooled asset balance.
    ///
    /// Increases the sXLM exchange rate without minting new shares so existing
    /// holders compound value across epochs.
    pub fn accrue_rewards(env: Env, caller: Address, reward_assets: i128) {
        caller.require_auth();
        let admin: Address = env.storage().instance().get(&DataKey::Admin).unwrap();
        if caller != admin {
            panic!("admin only");
        }
        if reward_assets <= 0 {
            panic!("reward must be positive");
        }

        let asset: Address = env.storage().instance().get(&DataKey::Asset).unwrap();
        token::Client::new(&env, &asset).transfer(
            &caller,
            &env.current_contract_address(),
            &reward_assets,
        );

        let total_pooled = Self::total_pooled(env.clone());
        env.storage()
            .instance()
            .set(&DataKey::TotalPooled, &(total_pooled + reward_assets));

        env.events()
            .publish((symbol_short!("reward"),), (reward_assets, Self::exchange_rate(env)));
    }

    pub fn total_pooled(env: Env) -> i128 {
        env.storage()
            .instance()
            .get(&DataKey::TotalPooled)
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

    /// Assets represented by one share, scaled by [`RATE_SCALE`].
    pub fn exchange_rate(env: Env) -> i128 {
        let total_shares = Self::total_shares(env.clone());
        if total_shares == 0 {
            return RATE_SCALE;
        }
        let total_pooled = Self::total_pooled(env);
        (total_pooled * RATE_SCALE) / total_shares
    }

    pub fn convert_to_shares(env: Env, assets: i128) -> i128 {
        let total_pooled = Self::total_pooled(env.clone());
        let total_shares = Self::total_shares(env);
        if total_pooled == 0 || total_shares == 0 {
            assets
        } else {
            (assets * total_shares) / total_pooled
        }
    }

    pub fn convert_to_assets(env: Env, shares: i128) -> i128 {
        let total_shares = Self::total_shares(env.clone());
        if total_shares == 0 {
            shares
        } else {
            let total_pooled = Self::total_pooled(env);
            (shares * total_pooled) / total_shares
        }
    }

    pub fn get_unbonding(env: Env, unbond_id: u32) -> UnbondingRequest {
        env.storage()
            .instance()
            .get(&DataKey::Unbond(unbond_id))
            .expect("unknown unbonding request")
    }

    pub fn cooldown(env: Env) -> u64 {
        env.storage().instance().get(&DataKey::Cooldown).unwrap()
    }
}

#[cfg(test)]
mod test;
