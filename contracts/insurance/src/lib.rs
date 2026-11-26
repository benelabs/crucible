#![no_std]
use soroban_sdk::{
    contract, contracterror, contractimpl, contracttype, symbol_short, token, Address, Env, String,
};

#[contracterror]
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum InsuranceError {
    AlreadyInitialized = 1,
    NotInitialized = 2,
    Unauthorized = 3,
    ReentrancyGuardLocked = 4,
    AmountsMustBePositive = 5,
    DurationMustBePositive = 6,
    DurationOverflow = 7,
    TokenNotSet = 8,
    PolicyNotFound = 9,
    PolicyNotActive = 10,
    PolicyExpired = 11,
    InvalidClaimAmount = 12,
    ClaimOverflow = 13,
    ClaimExceedsCoverage = 14,
    ClaimNotFound = 15,
    ClaimAlreadyProcessed = 16,
    InsufficientReserves = 17,
    ExtensionMustBePositive = 18,
    PremiumOverflow = 19,
}

#[contracttype]
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct InsurancePolicy {
    pub policy_id: u64,
    pub holder: Address,
    pub contract_address: Address,
    pub coverage_amount: i128,
    pub premium: i128,
    pub duration_days: u64,
    pub claimed_total: i128, // sum of pending + approved claims
    pub active: bool,
    pub created_at: u64,
    pub expires_at: u64,
}

#[contracttype]
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct Claim {
    pub claim_id: u64,
    pub policy_id: u64,
    pub amount: i128,
    pub status: u32, // 0 = pending, 1 = approved, 2 = rejected
    pub failure_reason: String,
    pub created_at: u64,
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

/// Drop-based reentrancy guard ensuring guards are safely released on all execution paths.
struct ReentrancyGuard<'a> {
    env: &'a Env,
}

impl<'a> ReentrancyGuard<'a> {
    fn acquire(env: &'a Env) -> Result<Self, InsuranceError> {
        let is_locked: bool = env
            .storage()
            .instance()
            .get(&DataKey::ReentrancyGuard)
            .unwrap_or(false);
        if is_locked {
            return Err(InsuranceError::ReentrancyGuardLocked);
        }
        env.storage()
            .instance()
            .set(&DataKey::ReentrancyGuard, &true);
        Ok(Self { env })
    }
}

impl<'a> Drop for ReentrancyGuard<'a> {
    fn drop(&mut self) {
        self.env
            .storage()
            .instance()
            .set(&DataKey::ReentrancyGuard, &false);
    }
}

/// Insurance Contract for smart contract failures
#[contract]
#[derive(Default)]
pub struct Insurance;

#[contractimpl]
impl Insurance {
    fn get_admin(env: &Env) -> Result<Address, InsuranceError> {
        env.storage()
            .instance()
            .get(&DataKey::Admin)
            .ok_or(InsuranceError::NotInitialized)
    }

    /// Initialize insurance contract.
    ///
    /// Requires admin authentication before any state is written, and enforces
    /// a one-time initialization guard to prevent overwriting admin control.
    pub fn initialize(
        env: Env,
        admin: Address,
        token: Address,
        initial_reserves: i128,
    ) -> Result<(), InsuranceError> {
        // Authenticate admin before storing any state
        admin.require_auth();

        let storage = env.storage().instance();
        // One-time initialization guard
        if storage.has(&DataKey::Admin) {
            return Err(InsuranceError::AlreadyInitialized);
        }
        if initial_reserves < 0 {
            return Err(InsuranceError::AmountsMustBePositive);
        }

        storage.set(&DataKey::Admin, &admin);
        storage.set(&DataKey::Token, &token);
        storage.set(&DataKey::TotalReserves, &initial_reserves);
        storage.set(&DataKey::PolicyCounter, &0u64);
        storage.set(&DataKey::ClaimCounter, &0u64);
        Ok(())
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
        // Validate inputs before locking
        if coverage_amount <= 0 || premium <= 0 {
            return Err(InsuranceError::AmountsMustBePositive);
        }
        if duration_days == 0 {
            return Err(InsuranceError::DurationMustBePositive);
        }

        let duration_secs = duration_days
            .checked_mul(86400)
            .ok_or(InsuranceError::DurationOverflow)?;
        let now = env.ledger().timestamp();
        let expires_at = now
            .checked_add(duration_secs)
            .ok_or(InsuranceError::DurationOverflow)?;

        holder.require_auth();

        let _guard = ReentrancyGuard::acquire(&env)?;
        let storage = env.storage().instance();

        // Collect premium from holder
        let token_addr = storage
            .get::<_, Address>(&DataKey::Token)
            .ok_or(InsuranceError::TokenNotSet)?;
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
    ) -> Result<u64, InsuranceError> {
        let storage = env.storage().instance();

        // Verify policy exists and is active before locking and before state mutation
        let mut policy = storage
            .get::<_, InsurancePolicy>(&DataKey::Policy(policy_id))
            .ok_or(InsuranceError::PolicyNotFound)?;

        policy.holder.require_auth();

        if !policy.active {
            return Err(InsuranceError::PolicyNotActive);
        }

        if env.ledger().timestamp() > policy.expires_at {
            return Err(InsuranceError::PolicyExpired);
        }

        if amount <= 0 {
            return Err(InsuranceError::InvalidClaimAmount);
        }

        let new_claimed_total = policy
            .claimed_total
            .checked_add(amount)
            .ok_or(InsuranceError::ClaimOverflow)?;
        if new_claimed_total > policy.coverage_amount {
            return Err(InsuranceError::ClaimExceedsCoverage);
        }

        let _guard = ReentrancyGuard::acquire(&env)?;

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
    pub fn approve_claim(env: Env, claim_id: u64) -> Result<(), InsuranceError> {
        let admin = Self::get_admin(&env)?;
        admin.require_auth();

        let storage = env.storage().instance();

        let mut claim = storage
            .get::<_, Claim>(&DataKey::Claim(claim_id))
            .ok_or(InsuranceError::ClaimNotFound)?;

        if claim.status != 0 {
            return Err(InsuranceError::ClaimAlreadyProcessed);
        }

        let reserves: i128 = storage.get(&DataKey::TotalReserves).unwrap_or(0);
        if reserves < claim.amount {
            return Err(InsuranceError::InsufficientReserves);
        }

        let policy = storage
            .get::<_, InsurancePolicy>(&DataKey::Policy(claim.policy_id))
            .ok_or(InsuranceError::PolicyNotFound)?;

        let token_addr = storage
            .get::<_, Address>(&DataKey::Token)
            .ok_or(InsuranceError::TokenNotSet)?;

        let _guard = ReentrancyGuard::acquire(&env)?;

        // Effects before interaction
        claim.status = 1; // approved
        storage.set(&DataKey::Claim(claim_id), &claim);
        storage.set(&DataKey::TotalReserves, &(reserves - claim.amount));

        // Pay out to policy holder
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
    pub fn reject_claim(env: Env, claim_id: u64) -> Result<(), InsuranceError> {
        let admin = Self::get_admin(&env)?;
        admin.require_auth();

        let storage = env.storage().instance();

        let mut claim = storage
            .get::<_, Claim>(&DataKey::Claim(claim_id))
            .ok_or(InsuranceError::ClaimNotFound)?;

        if claim.status != 0 {
            return Err(InsuranceError::ClaimAlreadyProcessed);
        }

        let mut policy = storage
            .get::<_, InsurancePolicy>(&DataKey::Policy(claim.policy_id))
            .ok_or(InsuranceError::PolicyNotFound)?;

        let _guard = ReentrancyGuard::acquire(&env)?;

        claim.status = 2; // rejected
        storage.set(&DataKey::Claim(claim_id), &claim);

        // Release the rejected amount back to the policy's remaining coverage
        policy.claimed_total -= claim.amount;
        storage.set(&DataKey::Policy(claim.policy_id), &policy);

        env.events().publish((symbol_short!("rejct"), claim_id), 0);

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
    pub fn renew_policy(
        env: Env,
        policy_id: u64,
        extension_days: u64,
    ) -> Result<(), InsuranceError> {
        let storage = env.storage().instance();

        let mut policy = storage
            .get::<_, InsurancePolicy>(&DataKey::Policy(policy_id))
            .ok_or(InsuranceError::PolicyNotFound)?;

        policy.holder.require_auth();

        if !policy.active {
            return Err(InsuranceError::PolicyNotActive);
        }

        if extension_days == 0 {
            return Err(InsuranceError::ExtensionMustBePositive);
        }

        let extension_secs = extension_days
            .checked_mul(86400)
            .ok_or(InsuranceError::DurationOverflow)?;
        let new_expires_at = policy
            .expires_at
            .checked_add(extension_secs)
            .ok_or(InsuranceError::DurationOverflow)?;

        // Pro-rata renewal premium: premium * extension_days / duration_days (rounded up)
        let renewal_premium = policy
            .premium
            .checked_mul(extension_days as i128)
            .ok_or(InsuranceError::PremiumOverflow)?;
        let duration = policy.duration_days as i128;
        let renewal_premium = (renewal_premium + duration - 1) / duration;

        let token_addr = storage
            .get::<_, Address>(&DataKey::Token)
            .ok_or(InsuranceError::TokenNotSet)?;

        let _guard = ReentrancyGuard::acquire(&env)?;

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

#[cfg(test)]
mod test {
    use super::*;
    use soroban_sdk::{
        testutils::{Address as _, Ledger as _},
        Env,
    };

    #[test]
    fn double_initialization_is_rejected() {
        let env = Env::default();
        let contract_id = env.register_contract(None, Insurance);
        let client = InsuranceClient::new(&env, &contract_id);

        let admin = Address::generate(&env);
        let token = Address::generate(&env);

        env.mock_all_auths();
        client.initialize(&admin, &token, &1_000_000);

        // Second call must fail with AlreadyInitialized.
        assert!(matches!(
            client.try_initialize(&admin, &token, &1_000_000),
            Err(Ok(InsuranceError::AlreadyInitialized))
        ));
    }

    #[test]
    fn approve_and_reject_claim_return_typed_errors_when_not_initialized() {
        let env = Env::default();
        let contract_id = env.register_contract(None, Insurance);
        let client = InsuranceClient::new(&env, &contract_id);

        env.mock_all_auths();

        // Before initialization, approve_claim must return NotInitialized.
        assert!(matches!(
            client.try_approve_claim(&1),
            Err(Ok(InsuranceError::NotInitialized))
        ));

        // Before initialization, reject_claim must return NotInitialized.
        assert!(matches!(
            client.try_reject_claim(&1),
            Err(Ok(InsuranceError::NotInitialized))
        ));
    }

    #[test]
    fn approve_nonexistent_claim_returns_typed_error() {
        let env = Env::default();
        let contract_id = env.register_contract(None, Insurance);
        let client = InsuranceClient::new(&env, &contract_id);
        let admin = Address::generate(&env);
        let token = Address::generate(&env);

        env.mock_all_auths();
        client.initialize(&admin, &token, &1_000_000);

        assert!(matches!(
            client.try_approve_claim(&999),
            Err(Ok(InsuranceError::ClaimNotFound))
        ));
    }

    #[test]
    fn reject_nonexistent_claim_returns_typed_error() {
        let env = Env::default();
        let contract_id = env.register_contract(None, Insurance);
        let client = InsuranceClient::new(&env, &contract_id);
        let admin = Address::generate(&env);
        let token = Address::generate(&env);

        env.mock_all_auths();
        client.initialize(&admin, &token, &1_000_000);

        assert!(matches!(
            client.try_reject_claim(&999),
            Err(Ok(InsuranceError::ClaimNotFound))
        ));
    }
}
