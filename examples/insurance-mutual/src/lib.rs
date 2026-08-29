#![no_std]
use soroban_sdk::{contract, contractimpl, contracttype, symbol_short, token, Address, Env};

#[contracttype]
#[derive(Clone, Debug, PartialEq)]
pub enum ClaimStatus {
    Pending,
    Approved,
    Rejected,
    Paid,
}

#[contracttype]
#[derive(Clone, Debug)]
pub struct Policy {
    pub holder: Address,
    pub coverage_amount: i128,
    pub premium_paid: i128,
    pub expiry: u64,
}

#[contracttype]
#[derive(Clone, Debug)]
pub struct Claim {
    pub policy_id: u64,
    pub claimant: Address,
    pub amount: i128,
    pub yes_votes: i128,
    pub no_votes: i128,
    pub status: ClaimStatus,
}

#[contracttype]
pub enum DataKey {
    CapitalPool,
    TotalUnderwritten,
    MinReserveRatioBps, // e.g. 2000 = 20%
    Policy(u64),
    Claim(u64),
    NextPolicyId,
    NextClaimId,
    Token,
}

#[contract]
#[derive(Default)]
pub struct InsuranceMutual;

#[contractimpl]
impl InsuranceMutual {
    /// Initialize the mutual pool with capital token and minimum reserve ratio (e.g. 2000 for 20%).
    pub fn initialize(env: Env, token: Address, min_reserve_ratio_bps: u32) {
        assert!(!env.storage().instance().has(&DataKey::Token), "Already initialized");
        env.storage().instance().set(&DataKey::Token, &token);
        env.storage().instance().set(&DataKey::MinReserveRatioBps, &min_reserve_ratio_bps);
        env.storage().instance().set(&DataKey::CapitalPool, &0i128);
        env.storage().instance().set(&DataKey::TotalUnderwritten, &0i128);
        env.storage().instance().set(&DataKey::NextPolicyId, &1u64);
        env.storage().instance().set(&DataKey::NextClaimId, &1u64);
    }

    /// Underwriters deposit capital into the risk pool.
    pub fn deposit_capital(env: Env, underwriter: Address, amount: i128) {
        underwriter.require_auth();
        assert!(amount > 0, "Amount must be positive");

        let token: Address = env.storage().instance().get(&DataKey::Token).expect("Uninitialized");
        let client = token::Client::new(&env, &token);
        client.transfer(&underwriter, &env.current_contract_address(), &amount);

        let mut pool: i128 = env.storage().instance().get(&DataKey::CapitalPool).unwrap_or(0);
        pool += amount;
        env.storage().instance().set(&DataKey::CapitalPool, &pool);

        env.events().publish((symbol_short!("cap_dep"), underwriter), amount);
    }

    /// Buy insurance policy against risk, enforcing capital reserve ratio.
    pub fn buy_policy(
        env: Env,
        holder: Address,
        coverage_amount: i128,
        premium: i128,
        duration_seconds: u64,
    ) -> u64 {
        holder.require_auth();
        assert!(coverage_amount > 0 && premium > 0, "Amounts must be positive");

        let token: Address = env.storage().instance().get(&DataKey::Token).expect("Uninitialized");
        let pool: i128 = env.storage().instance().get(&DataKey::CapitalPool).unwrap_or(0);
        let mut total_underwritten: i128 = env.storage().instance().get(&DataKey::TotalUnderwritten).unwrap_or(0);

        // Check solvency: (pool + premium) / (total_underwritten + coverage) >= reserve_ratio
        let ratio_bps: u32 = env.storage().instance().get(&DataKey::MinReserveRatioBps).unwrap_or(2000);
        let new_underwritten = total_underwritten + coverage_amount;
        let min_required_pool = (new_underwritten * ratio_bps as i128) / 10000;

        assert!(pool + premium >= min_required_pool, "Insufficient capital pool reserve for policy");

        let client = token::Client::new(&env, &token);
        client.transfer(&holder, &env.current_contract_address(), &premium);

        let new_pool = pool + premium;
        env.storage().instance().set(&DataKey::CapitalPool, &new_pool);

        total_underwritten += coverage_amount;
        env.storage().instance().set(&DataKey::TotalUnderwritten, &total_underwritten);

        let policy_id: u64 = env.storage().instance().get(&DataKey::NextPolicyId).unwrap_or(1);
        env.storage().instance().set(&DataKey::NextPolicyId, &(policy_id + 1));

        let policy = Policy {
            holder: holder.clone(),
            coverage_amount,
            premium_paid: premium,
            expiry: env.ledger().timestamp() + duration_seconds,
        };
        env.storage().persistent().set(&DataKey::Policy(policy_id), &policy);

        policy_id
    }

    /// Submit a claim on an active policy.
    pub fn submit_claim(env: Env, claimant: Address, policy_id: u64, amount: i128) -> u64 {
        claimant.require_auth();
        let policy: Policy = env.storage().persistent().get(&DataKey::Policy(policy_id)).expect("Policy not found");

        assert_eq!(policy.holder, claimant, "Caller is not policy holder");
        assert!(env.ledger().timestamp() <= policy.expiry, "Policy expired");
        assert!(amount > 0 && amount <= policy.coverage_amount, "Claim amount exceeds coverage");

        let claim_id: u64 = env.storage().instance().get(&DataKey::NextClaimId).unwrap_or(1);
        env.storage().instance().set(&DataKey::NextClaimId, &(claim_id + 1));

        let claim = Claim {
            policy_id,
            claimant: claimant.clone(),
            amount,
            yes_votes: 0,
            no_votes: 0,
            status: ClaimStatus::Pending,
        };
        env.storage().persistent().set(&DataKey::Claim(claim_id), &claim);

        claim_id
    }

    /// Assessors vote and execute claim payout when approved.
    pub fn vote_and_process_claim(env: Env, assessor: Address, claim_id: u64, approve: bool, vote_weight: i128) {
        assessor.require_auth();
        let mut claim: Claim = env.storage().persistent().get(&DataKey::Claim(claim_id)).expect("Claim not found");
        assert_eq!(claim.status, ClaimStatus::Pending, "Claim not pending");

        if approve {
            claim.yes_votes += vote_weight;
        } else {
            claim.no_votes += vote_weight;
        }

        // If approval quorum met (e.g. yes > 100), process payout
        if claim.yes_votes >= 100 {
            claim.status = ClaimStatus::Paid;
            let mut pool: i128 = env.storage().instance().get(&DataKey::CapitalPool).unwrap_or(0);
            assert!(pool >= claim.amount, "Pool insolvent");

            pool -= claim.amount;
            env.storage().instance().set(&DataKey::CapitalPool, &pool);

            let token: Address = env.storage().instance().get(&DataKey::Token).expect("Uninitialized");
            let client = token::Client::new(&env, &token);
            client.transfer(&env.current_contract_address(), &claim.claimant, &claim.amount);

            env.events().publish((symbol_short!("paid"), claim.claimant.clone()), claim.amount);
        }

        env.storage().persistent().set(&DataKey::Claim(claim_id), &claim);
    }
}

#[cfg(test)]
mod test;
