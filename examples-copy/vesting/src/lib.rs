#![no_std]
use soroban_sdk::{contract, contractimpl, contracttype, token, Address, Env};

/// Persistent state for the vesting schedule.
#[contracttype]
#[derive(Clone)]
pub struct VestingSchedule {
    pub beneficiary: Address,
    pub token: Address,
    /// Total tokens to be vested.
    pub total: i128,
    /// Tokens already claimed by the beneficiary.
    pub claimed: i128,
    /// Unix timestamp when the vesting period starts.
    pub start: u64,
    /// Duration (in seconds) of the cliff — no tokens vest during this period.
    pub cliff: u64,
    /// Duration (in seconds) of the linear vesting after the cliff.
    pub duration: u64,
    /// Set to true when the admin revokes the schedule.
    pub revoked: bool,
}

#[contracttype]
enum DataKey {
    Admin,
    Schedule,
}

/// A cliff + linear vesting contract.
///
/// Timeline:
///   `[start, start+cliff)` — cliff period, nothing vests.
///   `[start+cliff, start+cliff+duration]` — linear vesting from 0 → total.
///   After `start+cliff+duration` — fully vested.
#[contract]
#[derive(Default)]
pub struct Vesting;

#[contractimpl]
impl Vesting {
    /// Initialise the vesting schedule.
    ///
    /// Transfers `total` tokens from `admin` into this contract.
    #[allow(clippy::too_many_arguments)]
    pub fn initialize(
        env: Env,
        admin: Address,
        beneficiary: Address,
        token: Address,
        total: i128,
        start: u64,
        cliff: u64,
        duration: u64,
    ) {
        admin.require_auth();
        token::TokenClient::new(&env, &token).transfer(&admin, &env.current_contract_address(), &total);
        env.storage().instance().set(&DataKey::Admin, &admin);
        env.storage().instance().set(
            &DataKey::Schedule,
            &VestingSchedule {
                beneficiary,
                token,
                total,
                claimed: 0,
                start,
                cliff,
                duration,
                revoked: false,
            },
        );
    }

    /// Return how many tokens are currently available for the beneficiary to claim.
    pub fn claimable(env: Env) -> i128 {
        let s: VestingSchedule = env.storage().instance().get(&DataKey::Schedule).unwrap();
        if s.revoked {
            return 0;
        }
        let vested = Self::vested_at(&env, &s);
        (vested - s.claimed).max(0)
    }

    /// Return the total amount vested so far (includes already-claimed tokens).
    pub fn vested(env: Env) -> i128 {
        let s: VestingSchedule = env.storage().instance().get(&DataKey::Schedule).unwrap();
        Self::vested_at(&env, &s)
    }

    /// Claim all currently available tokens.
    ///
    /// Only the beneficiary may call this.
    pub fn claim(env: Env) {
        let mut s: VestingSchedule = env.storage().instance().get(&DataKey::Schedule).unwrap();
        if s.revoked {
            panic!("vesting has been revoked");
        }
        s.beneficiary.require_auth();
        let vested = Self::vested_at(&env, &s);
        let claimable = (vested - s.claimed).max(0);
        if claimable == 0 {
            panic!("nothing to claim");
        }
        s.claimed += claimable;
        env.storage().instance().set(&DataKey::Schedule, &s);
        token::TokenClient::new(&env, &s.token).transfer(
            &env.current_contract_address(),
            &s.beneficiary,
            &claimable,
        );
    }

    /// Revoke the vesting schedule.
    ///
    /// Unvested tokens are returned to the admin. Admin only.
    pub fn revoke(env: Env) {
        let mut s: VestingSchedule = env.storage().instance().get(&DataKey::Schedule).unwrap();
        let admin: Address = env.storage().instance().get(&DataKey::Admin).unwrap();
        admin.require_auth();
        if s.revoked {
            panic!("already revoked");
        }
        let vested = Self::vested_at(&env, &s);
        let unvested = s.total - vested;
        s.revoked = true;
        env.storage().instance().set(&DataKey::Schedule, &s);
        if unvested > 0 {
            token::TokenClient::new(&env, &s.token).transfer(
                &env.current_contract_address(),
                &admin,
                &unvested,
            );
        }
    }

    // -----------------------------------------------------------------------
    // Internal helpers
    // -----------------------------------------------------------------------

    fn vested_at(env: &Env, s: &VestingSchedule) -> i128 {
        let now = env.ledger().timestamp();
        let cliff_end = s.start + s.cliff;
        if now < cliff_end {
            return 0;
        }
        let vesting_end = cliff_end + s.duration;
        if now >= vesting_end {
            s.total
        } else {
            let elapsed = (now - cliff_end) as i128;
            s.total * elapsed / s.duration as i128
        }
    }
}

#[cfg(test)]
mod test;
