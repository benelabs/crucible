// Location: examples/quadratic-funding/src/lib.rs // Production requirement: Quadratic Funding Public Goods Grant Distributor
#![no_std]
use soroban_sdk::{
    contract, contractimpl, contracttype, symbol_short, token, Address, Env, Map, String, Vec,
};

/// Public goods project proposal receiving direct contributions and matching grants.
#[contracttype]
#[derive(Clone, Debug, PartialEq)]
pub struct ProjectProposal {
    pub project_id: u32,
    pub recipient: Address,
    pub name: String,
    pub total_contributions: i128,
    pub sum_sqrt_contributions: i128,
    pub contributors_count: u32,
    pub claimed: bool,
}

#[contracttype]
enum DataKey {
    Admin,
    MatchingToken,
    MatchingPool,
    RoundStart,
    RoundEnd,
    ClaimPeriodEnd,
    Project(u32),
    Contribution(u32, Address), // (project_id, contributor) -> amount
    TotalWeightedSumSqrts,
    ProjectsCount,
}

#[contract]
#[derive(Default)]
pub struct QuadraticFundingDistributor;

#[contractimpl]
impl QuadraticFundingDistributor {
    /// Initialize quadratic funding round timelines and matching pool.
    pub fn initialize(
        env: Env,
        admin: Address,
        matching_token: Address,
        matching_pool_amount: i128,
        round_start: u64,
        round_end: u64,
        claim_period_end: u64,
    ) {
        if env.storage().instance().has(&DataKey::Admin) {
            panic!("already initialized");
        }
        if round_start >= round_end || round_end >= claim_period_end {
            panic!("invalid timeline configuration");
        }
        if matching_pool_amount <= 0 {
            panic!("matching pool amount must be positive");
        }

        admin.require_auth();

        token::TokenClient::new(&env, &matching_token).transfer(
            &admin,
            &env.current_contract_address(),
            &matching_pool_amount,
        );

        env.storage().instance().set(&DataKey::Admin, &admin);
        env.storage().instance().set(&DataKey::MatchingToken, &matching_token);
        env.storage().instance().set(&DataKey::MatchingPool, &matching_pool_amount);
        env.storage().instance().set(&DataKey::RoundStart, &round_start);
        env.storage().instance().set(&DataKey::RoundEnd, &round_end);
        env.storage().instance().set(&DataKey::ClaimPeriodEnd, &claim_period_end);
        env.storage().instance().set(&DataKey::TotalWeightedSumSqrts, &0i128);
        env.storage().instance().set(&DataKey::ProjectsCount, &0u32);
    }

    /// Register a public goods project proposal for the funding round.
    pub fn register_project(
        env: Env,
        recipient: Address,
        name: String,
    ) -> u32 {
        recipient.require_auth();

        let count: u32 = env
            .storage()
            .instance()
            .get(&DataKey::ProjectsCount)
            .unwrap_or(0);

        let project_id = count + 1;

        let proposal = ProjectProposal {
            project_id,
            recipient: recipient.clone(),
            name,
            total_contributions: 0,
            sum_sqrt_contributions: 0,
            contributors_count: 0,
            claimed: false,
        };

        env.storage().instance().set(&DataKey::Project(project_id), &proposal);
        env.storage().instance().set(&DataKey::ProjectsCount, &project_id);

        env.events().publish((symbol_short!("reg_proj"), project_id), recipient);
        project_id
    }

    /// Make a direct quadratic contribution to a project.
    pub fn contribute(
        env: Env,
        contributor: Address,
        project_id: u32,
        amount: i128,
    ) {
        let now = env.ledger().timestamp();
        let start: u64 = env.storage().instance().get(&DataKey::RoundStart).expect("not initialized");
        let end: u64 = env.storage().instance().get(&DataKey::RoundEnd).expect("not initialized");

        if now < start || now > end {
            panic!("contribution outside round timeline");
        }

        if amount <= 0 {
            panic!("contribution amount must be positive");
        }
        contributor.require_auth();

        let mut project: ProjectProposal = env
            .storage()
            .instance()
            .get(&DataKey::Project(project_id))
            .expect("project not found");

        let matching_token: Address = env.storage().instance().get(&DataKey::MatchingToken).unwrap();
        token::TokenClient::new(&env, &matching_token).transfer(
            &contributor,
            &env.current_contract_address(),
            &amount,
        );

        // Previous contribution by this contributor
        let prev_contrib: i128 = env
            .storage()
            .instance()
            .get(&DataKey::Contribution(project_id, contributor.clone()))
            .unwrap_or(0);

        let new_contrib = prev_contrib + amount;
        env.storage().instance().set(
            &DataKey::Contribution(project_id, contributor.clone()),
            &new_contrib,
        );

        // QF Math: subtract sqrt(prev_contrib), add sqrt(new_contrib) to project.sum_sqrt_contributions
        let prev_sqrt = prev_contrib.isqrt();
        let new_sqrt = new_contrib.isqrt();
        let delta_sqrt = new_sqrt - prev_sqrt;

        project.sum_sqrt_contributions += delta_sqrt;
        project.total_contributions += amount;
        if prev_contrib == 0 {
            project.contributors_count += 1;
        }

        env.storage().instance().set(&DataKey::Project(project_id), &project);

        env.events().publish(
            (symbol_short!("contrib"), project_id),
            (contributor, amount),
        );
    }

    /// Claim direct contributions and quadratic matching subsidy for a project after round ends.
    pub fn claim_payout(env: Env, project_id: u32) -> (i128, i128) {
        let now = env.ledger().timestamp();
        let end: u64 = env.storage().instance().get(&DataKey::RoundEnd).expect("not initialized");
        let claim_end: u64 = env.storage().instance().get(&DataKey::ClaimPeriodEnd).expect("not initialized");

        if now <= end {
            panic!("round has not ended yet");
        }
        if now > claim_end {
            panic!("claim period has expired");
        }

        let mut project: ProjectProposal = env
            .storage()
            .instance()
            .get(&DataKey::Project(project_id))
            .expect("project not found");

        if project.claimed {
            panic!("funds already claimed");
        }

        project.recipient.require_auth();

        // Calculate total sum of (sum_sqrt)^2 across all projects to allocate matching pool proportionally
        let projects_count: u32 = env.storage().instance().get(&DataKey::ProjectsCount).unwrap_or(0);
        let mut total_qf_weight: i128 = 0;

        for i in 1..=projects_count {
            if let Some(p) = env.storage().instance().get::<_, ProjectProposal>(&DataKey::Project(i)) {
                let weight = p.sum_sqrt_contributions * p.sum_sqrt_contributions;
                total_qf_weight += weight;
            }
        }

        let matching_pool: i128 = env.storage().instance().get(&DataKey::MatchingPool).unwrap();
        let project_weight = project.sum_sqrt_contributions * project.sum_sqrt_contributions;

        let matching_subsidy = if total_qf_weight > 0 {
            (matching_pool * project_weight) / total_qf_weight
        } else {
            0
        };

        let total_payout = project.total_contributions + matching_subsidy;
        project.claimed = true;
        env.storage().instance().set(&DataKey::Project(project_id), &project);

        let matching_token: Address = env.storage().instance().get(&DataKey::MatchingToken).unwrap();
        token::TokenClient::new(&env, &matching_token).transfer(
            &env.current_contract_address(),
            &project.recipient,
            &total_payout,
        );

        env.events().publish(
            (symbol_short!("claimed"), project_id),
            (project.total_contributions, matching_subsidy),
        );

        (project.total_contributions, matching_subsidy)
    }

    /// Query project proposal details
    pub fn get_project(env: Env, project_id: u32) -> ProjectProposal {
        env.storage()
            .instance()
            .get(&DataKey::Project(project_id))
            .expect("project not found")
    }
}

#[cfg(test)]
mod test;
