#![no_std]
use soroban_sdk::{contract, contracterror, contractimpl, contracttype, symbol_short, token, Address, Env, String};

#[contracterror]
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum InsuranceError {
    AmountsMustBePositive = 1,
    ClaimAlreadyProcessed = 2,
    ClaimExceedsRemainingCoverage = 3,
    ClaimNotFound = 4,
    ClaimOverflow = 5,
    DurationMustBePositive = 6,
    DurationOverflow = 7,
    ExtensionMustBePositive = 8,
    InsufficientReserves = 9,
    InvalidClaimAmount = 10,
    PolicyExpired = 11,
    PolicyIsNotActive = 12,
    PolicyNotFound = 13,
    PremiumOverflow = 14,
    ReentrancyGuardLocked = 15,
    TokenNotSet = 16,
}


#[contracttype]
#[derive(Clone)]
struct InsurancePolicy {
    policy_id: u64,
    holder: Address,
    contract_address: Address,
    coverage_amount: i128,
    premium: i128,
    duration_days: u64,
    claimed_total: i128, // sum of pending + approved claims
    active: bool,
    created_at: u64,
    expires_at: u64,
}

#[contracttype]
#[derive(Clone)]
struct Claim {
    claim_id: u64,
    policy_id: u64,
    amount: i128,
    status: u32, // 0 = pending, 1 = approved, 2 = rejected
    failure_reason: String,
    created_at: u64,
}

#[contracttype]
enum DataKey {
    Admin,
    Token,
    TotalReserves,
    PolicyCounter,
    ClaimCounter,
    Policy(u64),
    Claim(u64),
    PolicyBalance(Address),
    ReentrancyGuard,
}

/// Insurance Contract for smart contract failures
#[contract]
#[derive(Default)]
pub struct Insurance;

#[contractimpl]
impl Insurance {
    fn lock_guard(env: &Env) -> Result<(), InsuranceError> {
        let is_locked: bool = env
            .storage()
            .instance()
            .get(&DataKey::ReentrancyGuard)
            .unwrap_or(false);
        if is_locked {
            return Err(InsuranceError::ReentrancyGuardLocked);
        }
        env.storage().instance().set(&DataKey::ReentrancyGuard, &true);
        Ok(())
    }

    fn unlock_guard(env: &Env) {
        env.storage().instance().set(&DataKey::ReentrancyGuard, &false);
    }
    /// Initialize insurance contract
    pub fn initialize(env: Env, admin: Address, token: Address, initial_reserves: i128) {
        let storage = env.storage().instance();
        storage.set(&DataKey::Admin, &admin);
        storage.set(&DataKey::Token, &token);
        storage.set(&DataKey::TotalReserves, &initial_reserves);
        storage.set(&DataKey::PolicyCounter, &0u64);
        storage.set(&DataKey::ClaimCounter, &0u64);
    }

    /// Create an insurance policy for a contract
    pub fn create_policy(
        env: Env,
        holder: Address,
        contract_address: Address,
        coverage_amount: i128,
        premium: i128,
        duration_days: u64,
    ) -> Result<u64, InsuranceError> {
        Self::lock_guard(&env)?;
        holder.require_auth();

        if coverage_amount <= 0 || premium <= 0 {
            Self::unlock_guard(&env);
            return Err(InsuranceError::AmountsMustBePositive);
        }
        if duration_days == 0 {
            Self::unlock_guard(&env);
            return Err(InsuranceError::DurationMustBePositive);
        }

        let Some(duration_secs) = duration_days.checked_mul(86400) else {
            Self::unlock_guard(&env);
            return Err(InsuranceError::DurationOverflow);
        };
        let now = env.ledger().timestamp();
        let Some(expires_at) = now.checked_add(duration_secs) else {
            Self::unlock_guard(&env);
            return Err(InsuranceError::DurationOverflow);
        };

        let storage = env.storage().instance();

        // Collect premium from holder
        let Some(token_addr) = storage.get::<_, Address>(&DataKey::Token) else {
            Self::unlock_guard(&env);
            return Err(InsuranceError::TokenNotSet);
        };
        token::Client::new(&env, &token_addr).transfer(
            &holder,
            &env.current_contract_address(),
            &premium,
        );

        let mut counter: u64 = storage.get(&DataKey::PolicyCounter).unwrap_or(0);
        counter += 1;

        let policy = InsurancePolicy {
            policy_id: counter,
            holder: holder.clone(),
            contract_address,
            coverage_amount,
            premium,
            duration_days,
            claimed_total: 0,
            active: true,
            created_at: now,
            expires_at,
        };

        storage.set(&DataKey::Policy(counter), &policy);
        storage.set(&DataKey::PolicyCounter, &counter);

        // Update balance
        let balance: i128 = storage
            .get(&DataKey::PolicyBalance(holder.clone()))
            .unwrap_or(0);
        storage.set(&DataKey::PolicyBalance(holder), &(balance + premium));

        let reserves: i128 = storage.get(&DataKey::TotalReserves).unwrap_or(0);
        storage.set(&DataKey::TotalReserves, &(reserves + premium));

        env.events()
            .publish((symbol_short!("policy"), counter), coverage_amount);

        Self::unlock_guard(&env);
        Ok(counter)
    }

    /// File a claim for contract failure
    pub fn file_claim(
        env: Env,
        policy_id: u64,
        amount: i128,
        failure_reason: String,
    ) -> Result<u64, InsuranceError> {
        Self::lock_guard(&env)?;
        let storage = env.storage().instance();

        // Verify policy exists and is active
        let Some(mut policy) = storage.get::<_, InsurancePolicy>(&DataKey::Policy(policy_id)) else {
            Self::unlock_guard(&env);
            return Err(InsuranceError::PolicyNotFound);
        };

        if !policy.active {
            Self::unlock_guard(&env);
            return Err(InsuranceError::PolicyIsNotActive);
        }

        if env.ledger().timestamp() > policy.expires_at {
            Self::unlock_guard(&env);
            return Err(InsuranceError::PolicyExpired);
        }

        if amount <= 0 {
            Self::unlock_guard(&env);
            return Err(InsuranceError::InvalidClaimAmount);
        }

        let Some(new_claimed_total) = policy.claimed_total.checked_add(amount) else {
            Self::unlock_guard(&env);
            return Err(InsuranceError::ClaimOverflow);
        };
        if new_claimed_total > policy.coverage_amount {
            Self::unlock_guard(&env);
            return Err(InsuranceError::ClaimExceedsRemainingCoverage);
        }

        policy.holder.require_auth();

        policy.claimed_total = new_claimed_total;
        storage.set(&DataKey::Policy(policy_id), &policy);

        let mut counter: u64 = storage.get(&DataKey::ClaimCounter).unwrap_or(0);
        counter += 1;

        let claim = Claim {
            claim_id: counter,
            policy_id,
            amount,
            status: 0, // pending
            failure_reason,
            created_at: env.ledger().timestamp(),
        };

        storage.set(&DataKey::Claim(counter), &claim);
        storage.set(&DataKey::ClaimCounter, &counter);

        env.events()
            .publish((symbol_short!("claim"), counter), amount);

        Self::unlock_guard(&env);
        Ok(counter)
    }

    /// Approve a claim (admin only)
    pub fn approve_claim(env: Env, claim_id: u64) -> Result<(), InsuranceError> {
        Self::lock_guard(&env)?;
        let admin: Address = env.storage().instance().get(&DataKey::Admin).unwrap();
        admin.require_auth();

        let storage = env.storage().instance();

        let Some(mut claim) = storage.get::<_, Claim>(&DataKey::Claim(claim_id)) else {
            Self::unlock_guard(&env);
            return Err(InsuranceError::ClaimNotFound);
        };

        if claim.status != 0 {
            Self::unlock_guard(&env);
            return Err(InsuranceError::ClaimAlreadyProcessed);
        }

        let reserves: i128 = storage.get(&DataKey::TotalReserves).unwrap_or(0);
        if reserves < claim.amount {
            Self::unlock_guard(&env);
            return Err(InsuranceError::InsufficientReserves);
        }

        let Some(policy) = storage.get::<_, InsurancePolicy>(&DataKey::Policy(claim.policy_id)) else {
            Self::unlock_guard(&env);
            return Err(InsuranceError::PolicyNotFound);
        };

        // Effects before interaction
        claim.status = 1; // approved
        storage.set(&DataKey::Claim(claim_id), &claim);
        storage.set(&DataKey::TotalReserves, &(reserves - claim.amount));

        // Pay out to policy holder
        let Some(token_addr) = storage.get::<_, Address>(&DataKey::Token) else {
            Self::unlock_guard(&env);
            return Err(InsuranceError::TokenNotSet);
        };
        token::Client::new(&env, &token_addr).transfer(
            &env.current_contract_address(),
            &policy.holder,
            &claim.amount,
        );

        env.events()
            .publish((symbol_short!("apprv"), claim_id), claim.amount);

        Self::unlock_guard(&env);
        Ok(())
    }

    /// Reject a claim (admin only)
    pub fn reject_claim(env: Env, claim_id: u64) -> Result<(), InsuranceError> {
        Self::lock_guard(&env)?;
        let admin: Address = env.storage().instance().get(&DataKey::Admin).unwrap();
        admin.require_auth();

        let storage = env.storage().instance();

        let Some(mut claim) = storage.get::<_, Claim>(&DataKey::Claim(claim_id)) else {
            Self::unlock_guard(&env);
            return Err(InsuranceError::ClaimNotFound);
        };

        if claim.status != 0 {
            Self::unlock_guard(&env);
            return Err(InsuranceError::ClaimAlreadyProcessed);
        }

        claim.status = 2; // rejected
        storage.set(&DataKey::Claim(claim_id), &claim);

        // Release the rejected amount back to the policy's remaining coverage
        let Some(mut policy) = storage.get::<_, InsurancePolicy>(&DataKey::Policy(claim.policy_id)) else {
            Self::unlock_guard(&env);
            return Err(InsuranceError::PolicyNotFound);
        };
        policy.claimed_total -= claim.amount;
        storage.set(&DataKey::Policy(claim.policy_id), &policy);

        env.events()
            .publish((symbol_short!("rejct"), claim_id), 0);

        Self::unlock_guard(&env);
        Ok(())
    }

    /// Get policy details
    pub fn get_policy(env: Env, policy_id: u64) -> Result<InsurancePolicy, InsuranceError> {
        env.storage()
            .instance()
            .get(&DataKey::Policy(policy_id))
            .ok_or(InsuranceError::PolicyNotFound)
    }

    /// Get claim details
    pub fn get_claim(env: Env, claim_id: u64) -> Result<Claim, InsuranceError> {
        env.storage()
            .instance()
            .get(&DataKey::Claim(claim_id))
            .ok_or(InsuranceError::ClaimNotFound)
    }

    /// Get total reserves
    pub fn get_reserves(env: Env) -> i128 {
        env.storage()
            .instance()
            .get(&DataKey::TotalReserves)
            .unwrap_or(0)
    }

    /// Renew policy
    pub fn renew_policy(env: Env, policy_id: u64, extension_days: u64) -> Result<(), InsuranceError> {
        Self::lock_guard(&env)?;
        let storage = env.storage().instance();

        let Some(mut policy) = storage.get::<_, InsurancePolicy>(&DataKey::Policy(policy_id)) else {
            Self::unlock_guard(&env);
            return Err(InsuranceError::PolicyNotFound);
        };

        policy.holder.require_auth();

        if !policy.active {
            Self::unlock_guard(&env);
            return Err(InsuranceError::PolicyIsNotActive);
        }

        if extension_days == 0 {
            Self::unlock_guard(&env);
            return Err(InsuranceError::ExtensionMustBePositive);
        }

        let Some(extension_secs) = extension_days.checked_mul(86400) else {
            Self::unlock_guard(&env);
            return Err(InsuranceError::DurationOverflow);
        };
        let Some(new_expires_at) = policy.expires_at.checked_add(extension_secs) else {
            Self::unlock_guard(&env);
            return Err(InsuranceError::DurationOverflow);
        };

        // Pro-rata renewal premium: premium * extension_days / duration_days (rounded up)
        let Some(renewal_premium) = policy.premium.checked_mul(extension_days as i128) else {
            Self::unlock_guard(&env);
            return Err(InsuranceError::PremiumOverflow);
        };
        let duration = policy.duration_days as i128;
        let renewal_premium = (renewal_premium + duration - 1) / duration;

        let Some(token_addr) = storage.get::<_, Address>(&DataKey::Token) else {
            Self::unlock_guard(&env);
            return Err(InsuranceError::TokenNotSet);
        };
        token::Client::new(&env, &token_addr).transfer(
            &policy.holder,
            &env.current_contract_address(),
            &renewal_premium,
        );

        let reserves: i128 = storage.get(&DataKey::TotalReserves).unwrap_or(0);
        storage.set(&DataKey::TotalReserves, &(reserves + renewal_premium));

        policy.expires_at = new_expires_at;
        storage.set(&DataKey::Policy(policy_id), &policy);

        env.events()
            .publish((symbol_short!("renew"), policy_id), extension_days as i128);

        Self::unlock_guard(&env);
        Ok(())
    }
}
