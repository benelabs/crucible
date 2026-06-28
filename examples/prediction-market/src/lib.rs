#![no_std]
use soroban_sdk::{contract, contractimpl, contracttype, symbol_short, token, Address, Env};

/// Binary outcome supported by the prediction market.
#[contracttype]
#[derive(Clone, PartialEq, Debug)]
pub enum Outcome {
    Yes,
    No,
}

/// Lifecycle state for a market.
#[contracttype]
#[derive(Clone, PartialEq, Debug)]
pub enum MarketStatus {
    Open,
    Resolved,
}

/// Market-level state stored under a single instance key.
#[contracttype]
#[derive(Clone, Debug)]
pub struct MarketState {
    pub admin: Address,
    pub token: Address,
    pub close_time: u64,
    pub status: MarketStatus,
    pub winning_outcome: Outcome,
    pub yes_total: i128,
    pub no_total: i128,
}

#[contracttype]
#[derive(Clone)]
struct PositionKey {
    trader: Address,
    outcome: Outcome,
}

#[contracttype]
enum DataKey {
    State,
    Position(PositionKey),
}

/// A minimal binary prediction market with escrowed collateral.
///
/// Traders buy YES or NO exposure before `close_time`. After the market closes,
/// the admin resolves the winning outcome and winners claim a proportional share
/// of the full collateral pool.
#[contract]
#[derive(Default)]
pub struct PredictionMarket;

#[contractimpl]
impl PredictionMarket {
    /// Initialize the market.
    ///
    /// `admin` acts as the resolver/oracle, `token` is the collateral asset,
    /// and `close_time` is the earliest timestamp at which resolution is valid.
    pub fn initialize(env: Env, admin: Address, token: Address, close_time: u64) {
        if env.storage().instance().has(&DataKey::State) {
            panic!("market already initialized");
        }
        if close_time <= env.ledger().timestamp() {
            panic!("close time must be in the future");
        }
        admin.require_auth();

        env.storage().instance().set(
            &DataKey::State,
            &MarketState {
                admin,
                token,
                close_time,
                status: MarketStatus::Open,
                winning_outcome: Outcome::No,
                yes_total: 0,
                no_total: 0,
            },
        );
        env.events().publish((symbol_short!("init"),), close_time);
    }

    /// Buy exposure to an outcome before the market closes.
    ///
    /// The caller's collateral is transferred into the contract and their
    /// position for the selected outcome is increased by `amount`.
    pub fn buy(env: Env, trader: Address, outcome: Outcome, amount: i128) {
        let mut state = Self::require_state(&env);
        if state.status != MarketStatus::Open {
            panic!("market is not open");
        }
        if env.ledger().timestamp() >= state.close_time {
            panic!("market is closed");
        }
        if amount <= 0 {
            panic!("amount must be positive");
        }
        trader.require_auth();

        token::TokenClient::new(&env, &state.token).transfer(
            &trader,
            env.current_contract_address(),
            &amount,
        );

        let key = DataKey::Position(PositionKey {
            trader: trader.clone(),
            outcome: outcome.clone(),
        });
        let position: i128 = env.storage().instance().get(&key).unwrap_or(0);
        env.storage()
            .instance()
            .set(&key, &Self::checked_add(position, amount));

        match outcome {
            Outcome::Yes => state.yes_total = Self::checked_add(state.yes_total, amount),
            Outcome::No => state.no_total = Self::checked_add(state.no_total, amount),
        }
        env.storage().instance().set(&DataKey::State, &state);
        env.events().publish((symbol_short!("buy"), trader), amount);
    }

    /// Resolve the market after close. Admin only.
    pub fn resolve(env: Env, admin: Address, winning_outcome: Outcome) {
        let mut state = Self::require_state(&env);
        if state.status != MarketStatus::Open {
            panic!("market already resolved");
        }
        if admin != state.admin {
            panic!("only the admin can resolve");
        }
        if env.ledger().timestamp() < state.close_time {
            panic!("market is still open");
        }
        admin.require_auth();

        state.status = MarketStatus::Resolved;
        state.winning_outcome = winning_outcome.clone();
        env.storage().instance().set(&DataKey::State, &state);
        env.events()
            .publish((symbol_short!("resolved"),), winning_outcome);
    }

    /// Claim the caller's proportional payout after resolution.
    pub fn claim(env: Env, trader: Address) -> i128 {
        let state = Self::require_state(&env);
        if state.status != MarketStatus::Resolved {
            panic!("market is not resolved");
        }
        trader.require_auth();

        let key = DataKey::Position(PositionKey {
            trader: trader.clone(),
            outcome: state.winning_outcome.clone(),
        });
        let position: i128 = env.storage().instance().get(&key).unwrap_or(0);
        if position <= 0 {
            panic!("no winning position");
        }

        let payout = Self::payout(&state, position);
        env.storage().instance().set(&key, &0_i128);
        token::TokenClient::new(&env, &state.token).transfer(
            &env.current_contract_address(),
            &trader,
            &payout,
        );
        env.events()
            .publish((symbol_short!("claim"), trader), payout);
        payout
    }

    /// Return the complete market state.
    pub fn get_state(env: Env) -> MarketState {
        Self::require_state(&env)
    }

    /// Return a trader's position for one outcome.
    pub fn position(env: Env, trader: Address, outcome: Outcome) -> i128 {
        env.storage()
            .instance()
            .get(&DataKey::Position(PositionKey { trader, outcome }))
            .unwrap_or(0)
    }

    /// Return total collateral escrowed across both sides.
    pub fn pool_total(env: Env) -> i128 {
        let state = Self::require_state(&env);
        Self::checked_add(state.yes_total, state.no_total)
    }

    fn require_state(env: &Env) -> MarketState {
        env.storage()
            .instance()
            .get(&DataKey::State)
            .unwrap_or_else(|| panic!("market is not initialized"))
    }

    fn payout(state: &MarketState, position: i128) -> i128 {
        let winning_total = match state.winning_outcome {
            Outcome::Yes => state.yes_total,
            Outcome::No => state.no_total,
        };
        if winning_total <= 0 {
            panic!("no winning liquidity");
        }
        let pool = Self::checked_add(state.yes_total, state.no_total);
        position
            .checked_mul(pool)
            .unwrap_or_else(|| panic!("payout overflow"))
            / winning_total
    }

    fn checked_add(left: i128, right: i128) -> i128 {
        left.checked_add(right)
            .unwrap_or_else(|| panic!("amount overflow"))
    }
}

#[cfg(test)]
mod test;
