#![no_std]
use soroban_sdk::{contract, contractimpl, contracttype, symbol_short, Address, Env};

#[contracttype]
#[derive(Clone)]
struct InsurancePolicy {
    policy_id: u64,
    holder: Address,
    contract_address: Address,
    coverage_amount: i128,
    premium: i128,
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
    TotalReserves,
    PolicyCounter,
    ClaimCounter,
    Policy(u64),
    Claim(u64),
    PolicyBalance(Address),
}

/// Insurance Contract for smart contract failures
#[contract]
#[derive(Default)]
pub struct Insurance;

#[contractimpl]
impl Insurance {
    /// Initialize insurance contract
    pub fn initialize(env: Env, admin: Address, initial_reserves: i128) {
        let storage = env.storage().instance();
        storage.set(&DataKey::Admin, &admin);
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

        let storage = env.storage().instance();
        let mut counter: u64 = storage.get(&DataKey::PolicyCounter).unwrap_or(0);
        counter += 1;

        let policy = InsurancePolicy {
            policy_id: counter,
            holder: holder.clone(),
            contract_address,
            coverage_amount,
            premium,
            active: true,
            created_at: env.ledger().timestamp(),
            expires_at: env.ledger().timestamp() + (duration_days * 86400),
        };

        storage.set(&DataKey::Policy(counter), &policy);
        storage.set(&DataKey::PolicyCounter, &counter);

        // Update balance
        let balance: i128 = storage
            .get(&DataKey::PolicyBalance(holder.clone()))
            .unwrap_or(0);
        storage.set(&DataKey::PolicyBalance(holder), &(balance + premium));

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
        let policy: InsurancePolicy = storage
            .get(&DataKey::Policy(policy_id))
            .ok_or("Policy not found")?;

        if !policy.active {
            return Err("Policy is not active");
        }

        if env.ledger().timestamp() > policy.expires_at {
            return Err("Policy expired");
        }

        if amount <= 0 || amount > policy.coverage_amount {
            return Err("Invalid claim amount");
        }

        policy.holder.require_auth();

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

        claim.status = 1; // approved
        storage.set(&DataKey::Claim(claim_id), &claim);

        // Deduct from reserves
        let reserves: i128 = storage.get(&DataKey::TotalReserves).unwrap_or(0);
        storage.set(&DataKey::TotalReserves, &(reserves - claim.amount));

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

        policy.expires_at += extension_days * 86400;
        storage.set(&DataKey::Policy(policy_id), &policy);

        env.events()
            .publish((symbol_short!("renew"), policy_id), extension_days as i128);

        Ok(())
    }
}
