#![no_std]
//! Cross-contract rate limiter and security circuit breaker.
//!
//! A protocol routes its outbound withdrawals through `record_outflow`. The
//! breaker keeps a rolling one-hour total and trips automatically once that
//! total crosses a configured fraction of TVL — 20% by default. While tripped,
//! every protected operation reverts, and only an M-of-N multi-sig of
//! guardians can reopen it.
//!
//! The design assumption is that a zero-day drain is fast and large: an
//! attacker moves a big fraction of the pool in minutes. A rolling window
//! catches that shape without penalising a protocol that legitimately pays out
//! the same amount spread across a day.

use soroban_sdk::{
    contract, contracterror, contractimpl, contracttype, symbol_short, Address, Env, Vec,
};

/// Rolling window over which outflow is summed.
const WINDOW_SECONDS: u64 = 3600;

/// Number of buckets the window is divided into.
///
/// Outflow is accumulated per bucket rather than per transaction, so the
/// window costs a fixed 12 entries regardless of how busy the protocol is. The
/// cost is granularity: the window is accurate to five minutes, which is far
/// finer than the timescale of the drain it exists to catch.
const BUCKET_COUNT: u64 = 12;
const BUCKET_SECONDS: u64 = WINDOW_SECONDS / BUCKET_COUNT;

/// Default trip threshold, in basis points of TVL. 2000 bps = 20%.
const DEFAULT_THRESHOLD_BPS: u32 = 2000;
const BPS_DENOMINATOR: i128 = 10_000;

#[contracttype]
#[derive(Clone)]
pub enum DataKey {
    /// Admin address.
    Admin,
    /// Guardians authorised to approve an unpause.
    Guardians,
    /// Approvals required to unpause.
    Threshold,
    /// Trip threshold in basis points of TVL.
    ThresholdBps,
    /// Last reported total value locked.
    Tvl,
    /// Whether the breaker is currently tripped.
    Tripped,
    /// Approvals collected for the current recovery attempt.
    RecoveryApprovals,
    /// Monotonic id of the current recovery round.
    RecoveryRound,
    /// Outflow accumulated in one bucket: (bucket index, amount).
    Bucket(u64),
}

#[contracterror]
#[derive(Copy, Clone, Debug, Eq, PartialEq, PartialOrd, Ord)]
#[repr(u32)]
pub enum BreakerError {
    AlreadyInitialized = 1,
    NotInitialized = 2,
    Unauthorized = 3,
    Tripped = 4,
    NotTripped = 5,
    InvalidConfig = 6,
    InvalidAmount = 7,
    AlreadyApproved = 8,
    InsufficientApprovals = 9,
}

#[contract]
#[derive(Default)]
pub struct CircuitBreaker;

#[contractimpl]
impl CircuitBreaker {
    /// Initialise the breaker.
    ///
    /// `guardians` are the addresses that can approve a recovery, and
    /// `threshold` is how many of them must approve. `threshold_bps` is the
    /// share of TVL that trips the breaker; passing 0 uses the 20% default.
    pub fn initialize(
        env: Env,
        admin: Address,
        guardians: Vec<Address>,
        threshold: u32,
        threshold_bps: u32,
    ) -> Result<(), BreakerError> {
        if env.storage().instance().has(&DataKey::Admin) {
            return Err(BreakerError::AlreadyInitialized);
        }
        admin.require_auth();

        if threshold == 0 || threshold > guardians.len() {
            return Err(BreakerError::InvalidConfig);
        }
        // A threshold of 100% or more can never trip, which would leave the
        // protocol believing it is protected when it is not.
        let bps = if threshold_bps == 0 {
            DEFAULT_THRESHOLD_BPS
        } else {
            threshold_bps
        };
        if bps as i128 >= BPS_DENOMINATOR {
            return Err(BreakerError::InvalidConfig);
        }

        let storage = env.storage().instance();
        storage.set(&DataKey::Admin, &admin);
        storage.set(&DataKey::Guardians, &guardians);
        storage.set(&DataKey::Threshold, &threshold);
        storage.set(&DataKey::ThresholdBps, &bps);
        storage.set(&DataKey::Tripped, &false);
        storage.set(&DataKey::Tvl, &0i128);
        storage.set(&DataKey::RecoveryRound, &0u64);

        Ok(())
    }

    /// Report the protocol's current total value locked. Admin only.
    ///
    /// TVL is reported rather than derived because the breaker is a
    /// cross-contract guard with no view into the protocol's own accounting.
    pub fn report_tvl(env: Env, tvl: i128) -> Result<(), BreakerError> {
        Self::require_admin(&env)?;
        if tvl < 0 {
            return Err(BreakerError::InvalidAmount);
        }
        env.storage().instance().set(&DataKey::Tvl, &tvl);
        Ok(())
    }

    /// Record an outbound transfer and trip the breaker if the rolling
    /// one-hour total now exceeds the configured share of TVL.
    ///
    /// Returns `true` when this call tripped the breaker. The outflow is
    /// recorded either way: an amount that crosses the line is still money
    /// that left, and dropping it would let the next call start from a clean
    /// slate.
    pub fn record_outflow(env: Env, caller: Address, amount: i128) -> Result<bool, BreakerError> {
        Self::require_admin(&env)?;
        caller.require_auth();

        if amount <= 0 {
            return Err(BreakerError::InvalidAmount);
        }
        if Self::is_tripped_internal(&env) {
            return Err(BreakerError::Tripped);
        }

        let now = env.ledger().timestamp();
        let bucket = now / BUCKET_SECONDS;
        let current: i128 = Self::bucket_amount(&env, bucket);

        // Buckets live in temporary storage with a TTL just past the window,
        // so expiry does the pruning and no sweep is needed.
        env.storage()
            .temporary()
            .set(&DataKey::Bucket(bucket), &(current + amount));
        env.storage().temporary().extend_ttl(
            &DataKey::Bucket(bucket),
            0,
            Self::window_ttl_ledgers(),
        );

        let total = Self::window_outflow_at(&env, now);
        let limit = Self::trip_limit(&env);

        // A protocol with no reported TVL has no meaningful ratio, so the
        // breaker stays out of the way rather than tripping on the first
        // withdrawal.
        if limit > 0 && total > limit {
            env.storage().instance().set(&DataKey::Tripped, &true);
            Self::begin_recovery_round(&env);
            env.events().publish(
                (symbol_short!("tripped"),),
                (total, limit, env.ledger().timestamp()),
            );
            return Ok(true);
        }

        Ok(false)
    }

    /// Trip the breaker by hand. Any guardian, or the admin, may do this.
    pub fn trip(env: Env, caller: Address) -> Result<(), BreakerError> {
        caller.require_auth();
        if !Self::is_admin(&env, &caller) && !Self::is_guardian(&env, &caller) {
            return Err(BreakerError::Unauthorized);
        }
        if Self::is_tripped_internal(&env) {
            return Err(BreakerError::Tripped);
        }

        env.storage().instance().set(&DataKey::Tripped, &true);
        Self::begin_recovery_round(&env);
        env.events()
            .publish((symbol_short!("tripped"),), (caller, symbol_short!("manual")));
        Ok(())
    }

    /// Approve reopening the breaker.
    ///
    /// The last approval needed also performs the unpause, so recovery cannot
    /// stall with a full set of approvals and nobody to execute them.
    /// Approvals belong to a recovery round and are discarded when the breaker
    /// trips again — stale approvals must never carry into a new incident.
    pub fn approve_recovery(env: Env, guardian: Address) -> Result<bool, BreakerError> {
        guardian.require_auth();
        if !Self::is_guardian(&env, &guardian) {
            return Err(BreakerError::Unauthorized);
        }
        if !Self::is_tripped_internal(&env) {
            return Err(BreakerError::NotTripped);
        }

        let mut approvals: Vec<Address> = env
            .storage()
            .instance()
            .get(&DataKey::RecoveryApprovals)
            .unwrap_or(Vec::new(&env));

        if approvals.contains(&guardian) {
            return Err(BreakerError::AlreadyApproved);
        }
        approvals.push_back(guardian.clone());

        let threshold: u32 = env
            .storage()
            .instance()
            .get(&DataKey::Threshold)
            .ok_or(BreakerError::NotInitialized)?;

        env.storage()
            .instance()
            .set(&DataKey::RecoveryApprovals, &approvals);
        env.events()
            .publish((symbol_short!("approved"),), (guardian, approvals.len()));

        if approvals.len() >= threshold {
            Self::reset(&env);
            env.events()
                .publish((symbol_short!("resumed"),), approvals.len());
            return Ok(true);
        }

        Ok(false)
    }

    /// Protected operation — reverts while the breaker is tripped.
    pub fn protected_action(env: Env, caller: Address) -> Result<(), BreakerError> {
        caller.require_auth();
        if Self::is_tripped_internal(&env) {
            return Err(BreakerError::Tripped);
        }
        env.events().publish((symbol_short!("action"),), caller);
        Ok(())
    }

    /// Outflow recorded in the rolling window ending now.
    pub fn window_outflow(env: Env) -> i128 {
        let now = env.ledger().timestamp();
        Self::window_outflow_at(&env, now)
    }

    /// Outflow above which the breaker trips. Zero when no TVL is reported.
    pub fn trip_threshold(env: Env) -> i128 {
        Self::trip_limit(&env)
    }

    pub fn is_tripped(env: Env) -> bool {
        Self::is_tripped_internal(&env)
    }

    /// Guardians who have approved the current recovery round.
    pub fn recovery_approvals(env: Env) -> Vec<Address> {
        env.storage()
            .instance()
            .get(&DataKey::RecoveryApprovals)
            .unwrap_or(Vec::new(&env))
    }

    // -----------------------------------------------------------------------
    // Internal helpers
    // -----------------------------------------------------------------------

    /// Sum every bucket that overlaps the window ending at `now`.
    fn window_outflow_at(env: &Env, now: u64) -> i128 {
        let newest = now / BUCKET_SECONDS;
        let oldest = newest.saturating_sub(BUCKET_COUNT - 1);

        let mut total: i128 = 0;
        let mut bucket = oldest;
        while bucket <= newest {
            total += Self::bucket_amount(env, bucket);
            bucket += 1;
        }
        total
    }

    fn bucket_amount(env: &Env, bucket: u64) -> i128 {
        env.storage()
            .temporary()
            .get(&DataKey::Bucket(bucket))
            .unwrap_or(0i128)
    }

    /// TTL long enough to outlive the window, in ledgers (~5s each).
    fn window_ttl_ledgers() -> u32 {
        ((WINDOW_SECONDS / 5) + 100) as u32
    }

    fn trip_limit(env: &Env) -> i128 {
        let tvl: i128 = env
            .storage()
            .instance()
            .get(&DataKey::Tvl)
            .unwrap_or(0i128);
        let bps: u32 = env
            .storage()
            .instance()
            .get(&DataKey::ThresholdBps)
            .unwrap_or(DEFAULT_THRESHOLD_BPS);
        tvl * bps as i128 / BPS_DENOMINATOR
    }

    /// Clear approvals and start a new round, so approvals from a previous
    /// incident cannot be counted towards this one.
    fn begin_recovery_round(env: &Env) {
        let round: u64 = env
            .storage()
            .instance()
            .get(&DataKey::RecoveryRound)
            .unwrap_or(0);
        env.storage()
            .instance()
            .set(&DataKey::RecoveryRound, &(round + 1));
        env.storage()
            .instance()
            .set(&DataKey::RecoveryApprovals, &Vec::<Address>::new(env));
    }

    /// Reopen the breaker and clear the window.
    ///
    /// The window is cleared on recovery so the outflow that caused the trip
    /// does not immediately re-trip the breaker on the next withdrawal.
    fn reset(env: &Env) {
        env.storage().instance().set(&DataKey::Tripped, &false);
        env.storage()
            .instance()
            .set(&DataKey::RecoveryApprovals, &Vec::<Address>::new(env));

        let now = env.ledger().timestamp();
        let newest = now / BUCKET_SECONDS;
        let oldest = newest.saturating_sub(BUCKET_COUNT - 1);
        let mut bucket = oldest;
        while bucket <= newest {
            env.storage().temporary().remove(&DataKey::Bucket(bucket));
            bucket += 1;
        }
    }

    fn is_tripped_internal(env: &Env) -> bool {
        env.storage()
            .instance()
            .get(&DataKey::Tripped)
            .unwrap_or(false)
    }

    fn is_admin(env: &Env, caller: &Address) -> bool {
        match env.storage().instance().get::<_, Address>(&DataKey::Admin) {
            Some(admin) => admin == *caller,
            None => false,
        }
    }

    fn is_guardian(env: &Env, caller: &Address) -> bool {
        let guardians: Vec<Address> = env
            .storage()
            .instance()
            .get(&DataKey::Guardians)
            .unwrap_or(Vec::new(env));
        guardians.contains(caller)
    }

    fn require_admin(env: &Env) -> Result<(), BreakerError> {
        let admin: Address = env
            .storage()
            .instance()
            .get(&DataKey::Admin)
            .ok_or(BreakerError::NotInitialized)?;
        admin.require_auth();
        Ok(())
    }
}

#[cfg(test)]
mod test;
