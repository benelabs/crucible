#![no_std]

use soroban_sdk::{contract, contractimpl, contracttype, symbol_short, token, Address, Env};

const BPS: i128 = 10_000;
const INDEX_SCALE: i128 = 1_000_000_000_000;
const SECONDS_PER_YEAR: i128 = 31_536_000;

#[contracttype]
#[derive(Clone)]
pub struct ReserveConfig {
    pub admin: Address,
    pub asset: Address,
    pub collateral_asset: Address,
    pub base_rate_bps: i128,
    pub utilization_rate_bps: i128,
    pub collateral_factor_bps: i128,
}

#[contracttype]
#[derive(Clone)]
pub struct ReserveState {
    pub total_supplied: i128,
    pub total_borrowed: i128,
    pub total_collateral: i128,
    pub supply_index: i128,
    pub borrow_index: i128,
    pub last_accrual_time: u64,
}

#[contracttype]
#[derive(Clone)]
pub struct AccountPosition {
    pub supplied_scaled: i128,
    pub borrowed_scaled: i128,
    pub collateral: i128,
}

#[contracttype]
#[derive(Clone)]
pub struct AccountSnapshot {
    pub supplied: i128,
    pub borrowed: i128,
    pub collateral: i128,
}

#[contracttype]
enum DataKey {
    Config,
    State,
    Position(Address),
}

/// Single-reserve lending pool with linear utilization-based interest.
///
/// The contract keeps lender and borrower balances in scaled units. Interest
/// accrues globally by updating two indexes once per mutating call, so the pool
/// never iterates through accounts.
#[contract]
#[derive(Default)]
pub struct Lending;

#[contractimpl]
impl Lending {
    /// Initialize a lending reserve.
    ///
    /// `base_rate_bps` is always charged to borrowers. `utilization_rate_bps`
    /// is multiplied by current pool utilization and added to the base rate.
    /// `collateral_factor_bps` caps borrow value against deposited collateral.
    pub fn initialize(
        env: Env,
        admin: Address,
        asset: Address,
        collateral_asset: Address,
        base_rate_bps: i128,
        utilization_rate_bps: i128,
        collateral_factor_bps: i128,
    ) {
        if env.storage().instance().has(&DataKey::Config) {
            panic!("lending pool already initialized");
        }
        Self::require_bps("base rate", base_rate_bps);
        Self::require_bps("utilization rate", utilization_rate_bps);
        Self::require_bps("collateral factor", collateral_factor_bps);

        admin.require_auth();
        env.storage().instance().set(
            &DataKey::Config,
            &ReserveConfig {
                admin,
                asset,
                collateral_asset,
                base_rate_bps,
                utilization_rate_bps,
                collateral_factor_bps,
            },
        );
        env.storage().instance().set(
            &DataKey::State,
            &ReserveState {
                total_supplied: 0,
                total_borrowed: 0,
                total_collateral: 0,
                supply_index: INDEX_SCALE,
                borrow_index: INDEX_SCALE,
                last_accrual_time: env.ledger().timestamp(),
            },
        );
    }

    /// Deposit pool liquidity and begin earning borrower-paid interest.
    pub fn deposit(env: Env, lender: Address, amount: i128) {
        Self::require_positive("deposit amount", amount);
        lender.require_auth();
        let config = Self::config(&env);
        let mut state = Self::accrue(&env, &config);
        let mut position = Self::load_position(&env, lender.clone());
        let scaled = Self::scale_down(amount, state.supply_index);

        position.supplied_scaled = Self::checked_add(position.supplied_scaled, scaled);
        state.total_supplied = Self::checked_add(state.total_supplied, amount);

        token::TokenClient::new(&env, &config.asset).transfer(
            &lender,
            &env.current_contract_address(),
            &amount,
        );
        Self::save_position(&env, lender.clone(), &position);
        Self::save_state(&env, &state);
        env.events()
            .publish((symbol_short!("deposit"), lender), amount);
    }

    /// Withdraw available supplied liquidity.
    pub fn withdraw(env: Env, lender: Address, amount: i128) {
        Self::require_positive("withdraw amount", amount);
        lender.require_auth();
        let config = Self::config(&env);
        let mut state = Self::accrue(&env, &config);
        let mut position = Self::load_position(&env, lender.clone());
        let supplied = Self::scale_up(position.supplied_scaled, state.supply_index);

        if supplied < amount {
            panic!("insufficient supplied balance");
        }
        if Self::available_liquidity(&state) < amount {
            panic!("insufficient pool liquidity");
        }

        position.supplied_scaled = Self::scale_down(supplied - amount, state.supply_index);
        state.total_supplied = Self::checked_sub(state.total_supplied, amount);

        Self::save_position(&env, lender.clone(), &position);
        Self::save_state(&env, &state);
        token::TokenClient::new(&env, &config.asset).transfer(
            &env.current_contract_address(),
            &lender,
            &amount,
        );
        env.events()
            .publish((symbol_short!("withdraw"), lender), amount);
    }

    /// Deposit collateral for future borrows.
    pub fn deposit_collateral(env: Env, borrower: Address, amount: i128) {
        Self::require_positive("collateral amount", amount);
        borrower.require_auth();
        let config = Self::config(&env);
        let mut state = Self::accrue(&env, &config);
        let mut position = Self::load_position(&env, borrower.clone());

        position.collateral = Self::checked_add(position.collateral, amount);
        state.total_collateral = Self::checked_add(state.total_collateral, amount);

        token::TokenClient::new(&env, &config.collateral_asset).transfer(
            &borrower,
            &env.current_contract_address(),
            &amount,
        );
        Self::save_position(&env, borrower.clone(), &position);
        Self::save_state(&env, &state);
        env.events()
            .publish((symbol_short!("collat"), borrower), amount);
    }

    /// Withdraw collateral if the remaining position stays healthy.
    pub fn withdraw_collateral(env: Env, borrower: Address, amount: i128) {
        Self::require_positive("collateral amount", amount);
        borrower.require_auth();
        let config = Self::config(&env);
        let mut state = Self::accrue(&env, &config);
        let mut position = Self::load_position(&env, borrower.clone());

        if position.collateral < amount {
            panic!("insufficient collateral");
        }
        position.collateral = Self::checked_sub(position.collateral, amount);
        Self::require_healthy(&position, &state, &config);
        state.total_collateral = Self::checked_sub(state.total_collateral, amount);

        Self::save_position(&env, borrower.clone(), &position);
        Self::save_state(&env, &state);
        token::TokenClient::new(&env, &config.collateral_asset).transfer(
            &env.current_contract_address(),
            &borrower,
            &amount,
        );
        env.events()
            .publish((symbol_short!("uncollat"), borrower), amount);
    }

    /// Borrow pool liquidity against deposited collateral.
    pub fn borrow(env: Env, borrower: Address, amount: i128) {
        Self::require_positive("borrow amount", amount);
        borrower.require_auth();
        let config = Self::config(&env);
        let mut state = Self::accrue(&env, &config);
        let mut position = Self::load_position(&env, borrower.clone());

        if Self::available_liquidity(&state) < amount {
            panic!("insufficient pool liquidity");
        }

        let borrowed = Self::scale_up(position.borrowed_scaled, state.borrow_index);
        let next_borrowed = Self::checked_add(borrowed, amount);
        position.borrowed_scaled = Self::scale_down(next_borrowed, state.borrow_index);
        Self::require_healthy(&position, &state, &config);
        state.total_borrowed = Self::checked_add(state.total_borrowed, amount);

        Self::save_position(&env, borrower.clone(), &position);
        Self::save_state(&env, &state);
        token::TokenClient::new(&env, &config.asset).transfer(
            &env.current_contract_address(),
            &borrower,
            &amount,
        );
        env.events()
            .publish((symbol_short!("borrow"), borrower), amount);
    }

    /// Repay borrowed principal plus accrued interest.
    ///
    /// Overpayments are capped to the current debt and only the needed amount
    /// is pulled from `borrower`.
    pub fn repay(env: Env, borrower: Address, amount: i128) {
        Self::require_positive("repay amount", amount);
        borrower.require_auth();
        let config = Self::config(&env);
        let mut state = Self::accrue(&env, &config);
        let mut position = Self::load_position(&env, borrower.clone());
        let borrowed = Self::scale_up(position.borrowed_scaled, state.borrow_index);

        if borrowed == 0 {
            panic!("nothing to repay");
        }
        let paid = if amount > borrowed { borrowed } else { amount };
        let remaining = borrowed - paid;
        position.borrowed_scaled = Self::scale_down(remaining, state.borrow_index);
        state.total_borrowed = Self::checked_sub(state.total_borrowed, paid);

        token::TokenClient::new(&env, &config.asset).transfer(
            &borrower,
            &env.current_contract_address(),
            &paid,
        );
        Self::save_position(&env, borrower.clone(), &position);
        Self::save_state(&env, &state);
        env.events()
            .publish((symbol_short!("repay"), borrower), paid);
    }

    /// Return the current reserve state after applying pending interest.
    pub fn reserve(env: Env) -> ReserveState {
        let config = Self::config(&env);
        Self::accrue(&env, &config)
    }

    /// Return a user's balances after applying pending interest.
    pub fn position(env: Env, account: Address) -> AccountSnapshot {
        let config = Self::config(&env);
        let state = Self::accrue(&env, &config);
        let position = Self::load_position(&env, account);
        AccountSnapshot {
            supplied: Self::scale_up(position.supplied_scaled, state.supply_index),
            borrowed: Self::scale_up(position.borrowed_scaled, state.borrow_index),
            collateral: position.collateral,
        }
    }

    /// Return available, unborrowed asset liquidity.
    pub fn liquidity(env: Env) -> i128 {
        let config = Self::config(&env);
        let state = Self::accrue(&env, &config);
        Self::available_liquidity(&state)
    }

    fn accrue(env: &Env, config: &ReserveConfig) -> ReserveState {
        let mut state: ReserveState = env.storage().instance().get(&DataKey::State).unwrap();
        let now = env.ledger().timestamp();
        let elapsed = now - state.last_accrual_time;
        if elapsed == 0 || state.total_borrowed == 0 {
            state.last_accrual_time = now;
            env.storage().instance().set(&DataKey::State, &state);
            return state;
        }

        let rate_bps = Self::borrow_rate_bps(config, &state);
        let interest = Self::checked_mul(
            Self::checked_mul(state.total_borrowed, rate_bps),
            elapsed as i128,
        ) / BPS
            / SECONDS_PER_YEAR;
        if interest > 0 {
            let prior_supplied = state.total_supplied;
            let prior_borrowed = state.total_borrowed;
            state.total_supplied = Self::checked_add(state.total_supplied, interest);
            state.total_borrowed = Self::checked_add(state.total_borrowed, interest);
            if prior_supplied > 0 {
                state.supply_index =
                    Self::checked_mul(state.supply_index, state.total_supplied) / prior_supplied;
            }
            state.borrow_index =
                Self::checked_mul(state.borrow_index, state.total_borrowed) / prior_borrowed;
        }
        state.last_accrual_time = now;
        env.storage().instance().set(&DataKey::State, &state);
        state
    }

    fn borrow_rate_bps(config: &ReserveConfig, state: &ReserveState) -> i128 {
        if state.total_supplied == 0 {
            return config.base_rate_bps;
        }
        let utilization_bps = Self::checked_mul(state.total_borrowed, BPS) / state.total_supplied;
        Self::checked_add(
            config.base_rate_bps,
            Self::checked_mul(config.utilization_rate_bps, utilization_bps) / BPS,
        )
    }

    fn require_healthy(position: &AccountPosition, state: &ReserveState, config: &ReserveConfig) {
        let borrowed = Self::scale_up(position.borrowed_scaled, state.borrow_index);
        let borrow_limit =
            Self::checked_mul(position.collateral, config.collateral_factor_bps) / BPS;
        if borrowed > borrow_limit {
            panic!("insufficient collateral");
        }
    }

    fn available_liquidity(state: &ReserveState) -> i128 {
        Self::checked_sub(state.total_supplied, state.total_borrowed)
    }

    fn config(env: &Env) -> ReserveConfig {
        env.storage().instance().get(&DataKey::Config).unwrap()
    }

    fn load_position(env: &Env, account: Address) -> AccountPosition {
        env.storage()
            .instance()
            .get(&DataKey::Position(account))
            .unwrap_or(AccountPosition {
                supplied_scaled: 0,
                borrowed_scaled: 0,
                collateral: 0,
            })
    }

    fn save_state(env: &Env, state: &ReserveState) {
        env.storage().instance().set(&DataKey::State, state);
    }

    fn save_position(env: &Env, account: Address, position: &AccountPosition) {
        env.storage()
            .instance()
            .set(&DataKey::Position(account), position);
    }

    fn scale_down(amount: i128, index: i128) -> i128 {
        Self::checked_mul(amount, INDEX_SCALE) / index
    }

    fn scale_up(scaled: i128, index: i128) -> i128 {
        Self::checked_mul(scaled, index) / INDEX_SCALE
    }

    fn checked_add(left: i128, right: i128) -> i128 {
        left.checked_add(right)
            .unwrap_or_else(|| panic!("math overflow"))
    }

    fn checked_sub(left: i128, right: i128) -> i128 {
        left.checked_sub(right)
            .unwrap_or_else(|| panic!("math underflow"))
    }

    fn checked_mul(left: i128, right: i128) -> i128 {
        left.checked_mul(right)
            .unwrap_or_else(|| panic!("math overflow"))
    }

    fn require_positive(label: &str, amount: i128) {
        if amount <= 0 {
            panic!("{label} must be positive");
        }
    }

    fn require_bps(label: &str, value: i128) {
        if !(0..=BPS).contains(&value) {
            panic!("{label} must be between 0 and 10000 bps");
        }
    }
}

#[cfg(test)]
mod test;
