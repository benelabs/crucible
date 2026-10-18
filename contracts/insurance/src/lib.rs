#![no_std]
use soroban_sdk::{contract, contractimpl, contracttype, symbol_short, token, Address, Env};

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
    fn lock_guard(env: &Env) -> Result<(), &'static str> {
        let is_locked: bool = env
            .storage()
            .instance()
            .get(&DataKey::ReentrancyGuard)
            .unwrap_or(false);
        if is_locked {
            return Err("Reentrancy guard locked");
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
    ) -> Result<u64, &'static str> {
        holder.require_auth();

        if coverage_amount <= 0 || premium <= 0 {
            return Err("Amounts must be positive");
        }
        if duration_days == 0 {
            return Err("Duration must be positive");
        }

        let duration_secs = duration_days.checked_mul(86400).ok_or("Duration overflow")?;
        let now = env.ledger().timestamp();
        let expires_at = now.checked_add(duration_secs).ok_or("Duration overflow")?;

        let storage = env.storage().instance();

        // Collect premium from holder
        let token_addr: Address = storage.get(&DataKey::Token).ok_or("Token not set")?;
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

        Ok(counter)
    }

    /// File a claim for contract failure
    pub fn file_claim(
        env: Env,
        policy_id: u64,
        amount: i128,
        failure_reason: String,
    ) -> Result<u64, &'static str> {
        let storage = env.storage().instance();

        // Verify policy exists and is active
        let mut policy: InsurancePolicy = storage
            .get(&DataKey::Policy(policy_id))
            .ok_or("Policy not found")?;

        if !policy.active {
            return Err("Policy is not active");
        }

        if env.ledger().timestamp() > policy.expires_at {
            return Err("Policy expired");
        }

        if amount <= 0 {
            return Err("Invalid claim amount");
        }

        let new_claimed_total = policy
            .claimed_total
            .checked_add(amount)
            .ok_or("Claim overflow")?;
        if new_claimed_total > policy.coverage_amount {
            return Err("Claim exceeds remaining coverage");
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

        Ok(counter)
    }

    /// Approve a claim (admin only)
    pub fn approve_claim(env: Env, claim_id: u64) -> Result<(), &'static str> {
        let admin: Address = env.storage().instance().get(&DataKey::Admin).unwrap();
        admin.require_auth();

        let storage = env.storage().instance();

        let mut claim: Claim = storage
            .get(&DataKey::Claim(claim_id))
            .ok_or("Claim not found")?;

        if claim.status != 0 {
            return Err("Claim already processed");
        }

        let reserves: i128 = storage.get(&DataKey::TotalReserves).unwrap_or(0);
        if reserves < claim.amount {
            return Err("Insufficient reserves");
        }

        let policy: InsurancePolicy = storage
            .get(&DataKey::Policy(claim.policy_id))
            .ok_or("Policy not found")?;

        // Effects before interaction
        claim.status = 1; // approved
        storage.set(&DataKey::Claim(claim_id), &claim);
        storage.set(&DataKey::TotalReserves, &(reserves - claim.amount));

        // Pay out to policy holder
        let token_addr: Address = storage.get(&DataKey::Token).ok_or("Token not set")?;
        token::Client::new(&env, &token_addr).transfer(
            &env.current_contract_address(),
            &policy.holder,
            &claim.amount,
        );

        env.events()
            .publish((symbol_short!("apprv"), claim_id), claim.amount);

        Ok(())
    }

    /// Reject a claim (admin only)
    pub fn reject_claim(env: Env, claim_id: u64) -> Result<(), &'static str> {
        let admin: Address = env.storage().instance().get(&DataKey::Admin).unwrap();
        admin.require_auth();

        let storage = env.storage().instance();

        let mut claim: Claim = storage
            .get(&DataKey::Claim(claim_id))
            .ok_or("Claim not found")?;

        if claim.status != 0 {
            return Err("Claim already processed");
        }

        claim.status = 2; // rejected
        storage.set(&DataKey::Claim(claim_id), &claim);

        // Release the rejected amount back to the policy's remaining coverage
        let mut policy: InsurancePolicy = storage
            .get(&DataKey::Policy(claim.policy_id))
            .ok_or("Policy not found")?;
        policy.claimed_total -= claim.amount;
        storage.set(&DataKey::Policy(claim.policy_id), &policy);

        env.events()
            .publish((symbol_short!("rejct"), claim_id), 0);

        Ok(())
    }

    /// Get policy details
    pub fn get_policy(env: Env, policy_id: u64) -> Result<InsurancePolicy, &'static str> {
        env.storage()
            .instance()
            .get(&DataKey::Policy(policy_id))
            .ok_or("Policy not found")
    }

    /// Get claim details
    pub fn get_claim(env: Env, claim_id: u64) -> Result<Claim, &'static str> {
        env.storage()
            .instance()
            .get(&DataKey::Claim(claim_id))
            .ok_or("Claim not found")
    }

    /// Get total reserves
    pub fn get_reserves(env: Env) -> i128 {
        env.storage()
            .instance()
            .get(&DataKey::TotalReserves)
            .unwrap_or(0)
    }

    /// Renew policy
    pub fn renew_policy(env: Env, policy_id: u64, extension_days: u64) -> Result<(), &'static str> {
        let storage = env.storage().instance();

        let mut policy: InsurancePolicy = storage
            .get(&DataKey::Policy(policy_id))
            .ok_or("Policy not found")?;

        policy.holder.require_auth();

        if !policy.active {
            return Err("Policy is not active");
        }

        if extension_days == 0 {
            return Err("Extension must be positive");
        }

        let extension_secs = extension_days
            .checked_mul(86400)
            .ok_or("Duration overflow")?;
        let new_expires_at = policy
            .expires_at
            .checked_add(extension_secs)
            .ok_or("Duration overflow")?;

        // Pro-rata renewal premium: premium * extension_days / duration_days (rounded up)
        let renewal_premium = policy
            .premium
            .checked_mul(extension_days as i128)
            .ok_or("Premium overflow")?;
        let duration = policy.duration_days as i128;
        let renewal_premium = (renewal_premium + duration - 1) / duration;

        let token_addr: Address = storage.get(&DataKey::Token).ok_or("Token not set")?;
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

        Ok(())
    }
}
