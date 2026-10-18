#![no_std]

use soroban_sdk::{
    contract, contracterror, contractimpl, contracttype, panic_with_error, symbol_short, token,
    Address, Env, Map, Vec,
};

// Define storage keys
#[contracttype]
#[derive(Clone, Debug, Eq, PartialEq)]
enum DataKey {
    Admins,          // Vec<Address>
    Quorum,          // u32
    Balances,        // Map<(Address, Address), i128>
    ReentrancyGuard, // bool lock
    DailyLimit,      // i128 — max withdrawable per period
    SpentInPeriod,   // i128 — amount withdrawn in current period
    /// Ledger sequence at which the current spending period started.
    LastResetSequence, // u32
}

/// Approximate ledgers per day (86_400s / 5s close). Period resets are tied to
/// sequence numbers so validators cannot prematurely reset limits via timestamp skew.
const LEDGERS_PER_DAY: u32 = 17_280;

#[contracterror]
#[derive(Copy, Clone, Debug, Eq, PartialEq, PartialOrd, Ord)]
#[repr(u32)]
pub enum ContractError {
    NotAdmin = 1,
    InsufficientQuorum = 2,
    InsufficientBalance = 3,
    /// `initialize` was called after the contract was already set up.
    AlreadyInitialized = 4,
    /// The admins vector passed to `initialize` was empty.
    EmptyAdmins = 5,
    /// `quorum` was zero or exceeded the number of admins.
    InvalidQuorum = 6,
    /// The admins vector contained duplicate addresses.
    DuplicateAdmin = 7,
    /// Reentrancy guard triggered - reentrant call forbidden.
    ReentrancyGuardLocked = 8,
    /// Withdrawal would exceed the daily spending limit for the current period.
    SpendingLimitExceeded = 9,
}

/// Default: transfers ≥ this amount must be queued (stroops / token base units).
const DEFAULT_TIMELOCK_THRESHOLD: i128 = 10_000;
/// Minimum delay for any queued withdrawal (ledgers).
const DEFAULT_BASE_DELAY: u64 = 10;
/// Extra ledgers of delay per value unit.
const DEFAULT_PER_UNIT_DELAY: u64 = 1;
/// Value unit for proportional delay scaling.
const DEFAULT_VALUE_UNIT: i128 = 1_000;

#[contract]
pub struct Treasury;

#[contractimpl]
impl Treasury {
    /// Initialize the treasury with a list of admin addresses, a quorum threshold,
    /// and a daily spending limit (enforced per ledger-sequence period).
    ///
    /// # Errors
    /// - [`ContractError::AlreadyInitialized`] — called more than once.
    /// - [`ContractError::EmptyAdmins`] — `admins` is empty.
    /// - [`ContractError::InvalidQuorum`] — `quorum` is 0 or greater than `admins.len()`.
    /// - [`ContractError::DuplicateAdmin`] — `admins` contains duplicate addresses.
    pub fn initialize(env: Env, admins: Vec<Address>, quorum: u32, daily_limit: i128) {
        if env.storage().instance().has(&DataKey::Admins) {
            panic_with_error!(&env, ContractError::AlreadyInitialized);
        }
        if admins.is_empty() {
            panic_with_error!(&env, ContractError::EmptyAdmins);
        }
        let n = admins.len();
        if quorum == 0 || quorum > n {
            panic_with_error!(&env, ContractError::InvalidQuorum);
        }
        // O(n²) duplicate check — admin lists are expected to be small
        for i in 0..n {
            for j in (i + 1)..n {
                if admins.get(i).unwrap() == admins.get(j).unwrap() {
                    panic_with_error!(&env, ContractError::DuplicateAdmin);
                }
            }
        }
        env.storage().instance().set(&DataKey::Admins, &admins);
        env.storage().instance().set(&DataKey::Quorum, &quorum);
        let balances: Map<(Address, Address), i128> = Map::new(&env);
        env.storage().instance().set(&DataKey::Balances, &balances);
        // Spending period keyed to ledger sequence (not wall-clock timestamp).
        env.storage().instance().set(&DataKey::DailyLimit, &daily_limit);
        env.storage().instance().set(&DataKey::SpentInPeriod, &0i128);
        env.storage()
            .instance()
            .set(&DataKey::LastResetSequence, &env.ledger().sequence());
        env.events()
            .publish((symbol_short!("init"),), (admins, quorum));
    }

    fn is_admin(env: &Env, caller: &Address) -> bool {
        let admins: Vec<Address> = env.storage().instance().get(&DataKey::Admins).unwrap();
        admins.iter().any(|a| a == *caller)
    }

    fn is_guardian(env: &Env, caller: &Address) -> bool {
        let guardians: Vec<Address> = env
            .storage()
            .instance()
            .get(&DataKey::Guardians)
            .unwrap_or(Vec::new(env));
        guardians.iter().any(|g| g == *caller)
    }

    /// Delay in ledgers proportional to `amount` (base + amount/unit * per_unit).
    fn timelock_delay(env: &Env, amount: i128) -> u64 {
        let base: u64 = env
            .storage()
            .instance()
            .get(&DataKey::TimelockBaseDelay)
            .unwrap_or(DEFAULT_BASE_DELAY);
        let per_unit: u64 = env
            .storage()
            .instance()
            .get(&DataKey::TimelockPerUnit)
            .unwrap_or(DEFAULT_PER_UNIT_DELAY);
        let unit: i128 = env
            .storage()
            .instance()
            .get(&DataKey::TimelockUnit)
            .unwrap_or(DEFAULT_VALUE_UNIT)
            .max(1);
        let units = (amount / unit) as u64;
        base.saturating_add(units.saturating_mul(per_unit))
    }

    /// Deposit an amount of a given token (use Address::from([0;32]) for native XLM).
    pub fn deposit(env: Env, depositor: Address, token: Address, amount: i128) {
        // Ensure the depositor authorized this operation
        depositor.require_auth();

        // Perform actual token transfer from depositor to this contract (treasury)
        token::Client::new(&env, &token).transfer(
            &depositor,
            &env.current_contract_address(),
            &amount,
        );

        // Update internal accounting only after successful transfer
        let mut balances: Map<(Address, Address), i128> =
            env.storage().instance().get(&DataKey::Balances).unwrap();
        let treasury_addr = env.current_contract_address();
        let key = (treasury_addr.clone(), token.clone());
        let current = balances.get(key.clone()).unwrap_or(0);
        let new_balance = current + amount;
        if new_balance < 0 {
            panic_with_error!(&env, ContractError::InsufficientBalance);
        }
        balances.set(key.clone(), new_balance);
        env.storage().instance().set(&DataKey::Balances, &balances);
        env.events()
            .publish((symbol_short!("deposit"),), (depositor, token, amount));
    }

    fn lock_guard(env: &Env) {
        let is_locked: bool = env
            .storage()
            .instance()
            .get(&DataKey::ReentrancyGuard)
            .unwrap_or(false);
        if is_locked {
            panic_with_error!(env, ContractError::ReentrancyGuardLocked);
        }
        env.storage()
            .instance()
            .set(&DataKey::ReentrancyGuard, &true);
    }

    fn unlock_guard(env: &Env) {
        env.storage()
            .instance()
            .set(&DataKey::ReentrancyGuard, &false);
    }

    /// Enforce the daily spending limit using ledger sequence periods.
    /// Resets `SpentInPeriod` once `LEDGERS_PER_DAY` ledgers have elapsed.
    fn check_and_update_spending_limit(env: &Env, amount: i128) {
        let daily_limit: i128 = env
            .storage()
            .instance()
            .get(&DataKey::DailyLimit)
            .unwrap_or(i128::MAX);
        // No-op when limit is unset / unlimited.
        if daily_limit == i128::MAX || daily_limit <= 0 {
            return;
        }

        let mut spent: i128 = env
            .storage()
            .instance()
            .get(&DataKey::SpentInPeriod)
            .unwrap_or(0);
        let last_reset: u32 = env
            .storage()
            .instance()
            .get(&DataKey::LastResetSequence)
            .unwrap_or(0);
        let current_sequence = env.ledger().sequence();

        if current_sequence >= last_reset.saturating_add(LEDGERS_PER_DAY) {
            spent = 0;
            env.storage()
                .instance()
                .set(&DataKey::LastResetSequence, &current_sequence);
        }

        let new_spent = match spent.checked_add(amount) {
            Some(v) => v,
            None => {
                panic_with_error!(env, ContractError::SpendingLimitExceeded);
            }
        };
        if new_spent > daily_limit {
            panic_with_error!(env, ContractError::SpendingLimitExceeded);
        }
        env.storage()
            .instance()
            .set(&DataKey::SpentInPeriod, &new_spent);
    }

    /// Withdraw tokens from the treasury to a destination address.
    ///
    /// `signers` must include >= quorum admin addresses, each of which must authorize.
    ///
    /// Transfers below the timelock threshold execute immediately. Larger
    /// transfers are queued with a delay proportional to their value; guardians
    /// (or admins) may cancel during that window, and anyone may call
    /// [`execute_withdrawal`](Self::execute_withdrawal) once the delay elapses.
    ///
    /// Returns `0` for immediate withdrawals, or the pending operation id for queued ones.
    pub fn withdraw(
        env: Env,
        to: Address,
        token: Address,
        amount: i128,
        signers: Vec<Address>,
    ) -> u64 {
        Self::lock_guard(&env);

        // Require authorization from every signer before checking quorum.
        for s in signers.iter() {
            s.require_auth();
        }

        let quorum: u32 = env.storage().instance().get(&DataKey::Quorum).unwrap();
        let admins: Vec<Address> = env.storage().instance().get(&DataKey::Admins).unwrap();
        let mut valid = 0u32;
        for s in signers.iter() {
            if admins.iter().any(|a| a == s) {
                valid += 1;
            }
        }
        if valid < quorum {
            Self::unlock_guard(&env);
            panic_with_error!(&env, ContractError::InsufficientQuorum);
        }

        // Sequence-based spending limit (not wall-clock timestamp).
        Self::check_and_update_spending_limit(&env, amount);

        // Treasury address is the contract's own address
        let treasury_addr = env.current_contract_address();
        let mut balances: Map<(Address, Address), i128> =
            env.storage().instance().get(&DataKey::Balances).unwrap();
        let key = (treasury_addr.clone(), token.clone());
        let current = balances.get(key.clone()).unwrap_or(0);
        if current < amount {
            Self::unlock_guard(&env);
            panic_with_error!(&env, ContractError::InsufficientBalance);
        }

        let threshold: i128 = env
            .storage()
            .instance()
            .get(&DataKey::TimelockThreshold)
            .unwrap_or(DEFAULT_TIMELOCK_THRESHOLD);

        // Reserve / debit accounting up front for both paths.
        let new_balance = current - amount;
        balances.set(key, new_balance);
        env.storage().instance().set(&DataKey::Balances, &balances);

        if amount < threshold {
            env.events()
                .publish((symbol_short!("withdraw"),), (to, token, amount));
            Self::unlock_guard(&env);
            return 0;
        }

        // Large transfer: queue with proportional timelock.
        let delay = Self::timelock_delay(&env, amount);
        let execute_after = (env.ledger().sequence() as u64).saturating_add(delay);
        let mut next_id: u64 = env
            .storage()
            .instance()
            .get(&DataKey::NextOpId)
            .unwrap_or(0);
        let op_id = next_id;
        next_id = next_id.saturating_add(1);
        env.storage().instance().set(&DataKey::NextOpId, &next_id);

        let pending_op = PendingWithdrawal {
            to: to.clone(),
            token: token.clone(),
            amount,
            execute_after,
            cancelled: false,
        };
        let mut pending: Map<u64, PendingWithdrawal> = env
            .storage()
            .instance()
            .get(&DataKey::PendingOps)
            .unwrap_or(Map::new(&env));
        pending.set(op_id, pending_op);
        env.storage().instance().set(&DataKey::PendingOps, &pending);

        env.events().publish(
            (symbol_short!("queued"), op_id),
            (to, token, amount, execute_after),
        );

        Self::unlock_guard(&env);
        op_id
    }

    /// Execute a queued withdrawal after its timelock has elapsed.
    pub fn execute_withdrawal(env: Env, op_id: u64) {
        Self::lock_guard(&env);
        let mut pending: Map<u64, PendingWithdrawal> = env
            .storage()
            .instance()
            .get(&DataKey::PendingOps)
            .unwrap_or(Map::new(&env));
        let Some(op) = pending.get(op_id) else {
            Self::unlock_guard(&env);
            panic_with_error!(&env, ContractError::UnknownPendingOp);
        };
        if op.cancelled {
            Self::unlock_guard(&env);
            panic_with_error!(&env, ContractError::WithdrawalCancelled);
        }
        if (env.ledger().sequence() as u64) < op.execute_after {
            Self::unlock_guard(&env);
            panic_with_error!(&env, ContractError::TimelockNotElapsed);
        }

        pending.remove(op_id);
        env.storage().instance().set(&DataKey::PendingOps, &pending);

        // Funds were reserved at queue time; emit the settle event.
        env.events().publish(
            (symbol_short!("withdraw"),),
            (op.to.clone(), op.token.clone(), op.amount),
        );
        env.events()
            .publish((symbol_short!("exec_wd"), op_id), op.amount);

        Self::unlock_guard(&env);
    }

    /// Cancel a queued withdrawal during the timelock window.
    /// Callable by any guardian or admin; restores reserved balance.
    pub fn cancel_withdrawal(env: Env, op_id: u64, caller: Address) {
        caller.require_auth();
        if !Self::is_admin(&env, &caller) && !Self::is_guardian(&env, &caller) {
            panic_with_error!(&env, ContractError::NotGuardian);
        }

        Self::lock_guard(&env);
        let mut pending: Map<u64, PendingWithdrawal> = env
            .storage()
            .instance()
            .get(&DataKey::PendingOps)
            .unwrap_or(Map::new(&env));
        let Some(mut op) = pending.get(op_id) else {
            Self::unlock_guard(&env);
            panic_with_error!(&env, ContractError::UnknownPendingOp);
        };
        if op.cancelled {
            Self::unlock_guard(&env);
            panic_with_error!(&env, ContractError::WithdrawalCancelled);
        }

        op.cancelled = true;
        pending.set(op_id, op.clone());
        env.storage().instance().set(&DataKey::PendingOps, &pending);

        // Restore reserved funds.
        let treasury_addr = env.current_contract_address();
        let mut balances: Map<(Address, Address), i128> =
            env.storage().instance().get(&DataKey::Balances).unwrap();
        let key = (treasury_addr, op.token.clone());
        let current = balances.get(key.clone()).unwrap_or(0);
        balances.set(key, current + op.amount);
        env.storage().instance().set(&DataKey::Balances, &balances);

        env.events()
            .publish((symbol_short!("cancel"), op_id), (caller, op.amount));

        Self::unlock_guard(&env);
    }

    /// Inspect a pending withdrawal (if any).
    pub fn get_pending(env: Env, op_id: u64) -> Option<PendingWithdrawal> {
        let pending: Map<u64, PendingWithdrawal> = env
            .storage()
            .instance()
            .get(&DataKey::PendingOps)
            .unwrap_or(Map::new(&env));
        pending.get(op_id)
    }

    /// Query the balance of an account for a given token.
    pub fn balance_of(env: Env, account: Address, token: Address) -> i128 {
        let balances: Map<(Address, Address), i128> =
            env.storage().instance().get(&DataKey::Balances).unwrap();
        balances.get((account, token)).unwrap_or(0)
    }

    /// Execute a flash loan. Borrows `amount` of `token` to `borrower`.
    /// The borrower must repay `amount + fee` within the same transaction scope.
    pub fn flash_loan(env: Env, borrower: Address, token: Address, amount: i128) {
        Self::lock_guard(&env);
        borrower.require_auth();

        if amount <= 0 {
            Self::unlock_guard(&env);
            panic_with_error!(&env, ContractError::InsufficientBalance);
        }

        let treasury_addr = env.current_contract_address();
        let mut balances: Map<(Address, Address), i128> =
            env.storage().instance().get(&DataKey::Balances).unwrap();
        let key = (treasury_addr.clone(), token.clone());
        let current = balances.get(key.clone()).unwrap_or(0);
        if current < amount {
            Self::unlock_guard(&env);
            panic_with_error!(&env, ContractError::InsufficientBalance);
        }

        // Calculate 0.1% fee (minimum 1 unit)
        let mut fee = amount / 1000;
        if fee == 0 {
            fee = 1;
        }

        // Transfer funds from treasury to borrower
        token::Client::new(&env, &token).transfer(&treasury_addr, &borrower, &amount);

        // Repay funds + fee from borrower back to treasury
        token::Client::new(&env, &token).transfer(&borrower, &treasury_addr, &(amount + fee));

        // Update internal accounting with collected fee
        let new_balance = current + fee;
        balances.set(key.clone(), new_balance);
        env.storage().instance().set(&DataKey::Balances, &balances);

        env.events().publish(
            (symbol_short!("flashloan"),),
            (borrower, token, amount, fee),
        );

        Self::unlock_guard(&env);
    }
}
