#![no_std]
use soroban_sdk::{contract, contractimpl, contracttype, symbol_short, Address, Env, Map};

#[contracttype]
#[derive(Clone)]
struct Proposal {
    id: u64,
    title: String,
    description: String,
    proposer: Address,
    votes_for: i128,
    votes_against: i128,
    created_at: u64,
    deadline: u64,
    executed: bool,
}

#[contracttype]
#[derive(Clone)]
struct Vote {
    voter: Address,
    proposal_id: u64,
    amount: i128,
    direction: bool, // true = for, false = against
}

#[contracttype]
enum DataKey {
    Admin,
    TotalSupply,
    Balance(Address),
    ProposalCounter,
    Proposal(u64),
    Vote(Address, u64),
    VotingPower(Address),
}

/// DAO Governance Contract with voting capabilities
#[contract]
#[derive(Default)]
pub struct Governance;

#[contractimpl]
impl Governance {
    /// Initialize governance with admin and initial token supply
    pub fn initialize(env: Env, admin: Address, initial_supply: i128) {
        let storage = env.storage().instance();
        storage.set(&DataKey::Admin, &admin);
        storage.set(&DataKey::TotalSupply, &initial_supply);
        storage.set(&DataKey::ProposalCounter, &0u64);
        storage.set(&DataKey::Balance(admin.clone()), &initial_supply);
    }

    /// Get voting power of an address
    pub fn voting_power(env: Env, account: Address) -> i128 {
        let storage = env.storage().instance();
        storage
            .get(&DataKey::VotingPower(account.clone()))
            .unwrap_or(0)
    }

    /// Create a new proposal
    pub fn create_proposal(
        env: Env,
        proposer: Address,
        title: String,
        description: String,
        deadline: u64,
    ) -> Result<u64, &'static str> {
        let admin: Address = env.storage().instance().get(&DataKey::Admin).unwrap();
        proposer.require_auth();

        let storage = env.storage().instance();
        let mut counter: u64 = storage.get(&DataKey::ProposalCounter).unwrap_or(0);
        counter += 1;

        let proposal = Proposal {
            id: counter,
            title,
            description,
            proposer,
            votes_for: 0,
            votes_against: 0,
            created_at: env.ledger().timestamp(),
            deadline,
            executed: false,
        };

        storage.set(&DataKey::Proposal(counter), &proposal);
        storage.set(&DataKey::ProposalCounter, &counter);

        env.events()
            .publish((symbol_short!("prop"), counter), proposal.title);

        Ok(counter)
    }

    /// Cast vote on a proposal
    pub fn vote(
        env: Env,
        voter: Address,
        proposal_id: u64,
        amount: i128,
        direction: bool,
    ) -> Result<(), &'static str> {
        voter.require_auth();

        let storage = env.storage().instance();

        // Get proposal
        let mut proposal: Proposal = storage
            .get(&DataKey::Proposal(proposal_id))
            .ok_or("Proposal not found")?;

        if env.ledger().timestamp() > proposal.deadline {
            return Err("Voting period ended");
        }

        if amount <= 0 {
            return Err("Vote amount must be positive");
        }

        // Check voting power
        let voting_power: i128 = storage
            .get(&DataKey::VotingPower(voter.clone()))
            .unwrap_or(0);

        if voting_power < amount {
            return Err("Insufficient voting power");
        }

        // Record vote
        let vote = Vote {
            voter: voter.clone(),
            proposal_id,
            amount,
            direction,
        };

        storage.set(&DataKey::Vote(voter.clone(), proposal_id), &vote);

        // Update proposal vote counts
        if direction {
            proposal.votes_for += amount;
        } else {
            proposal.votes_against += amount;
        }

        storage.set(&DataKey::Proposal(proposal_id), &proposal);

        env.events()
            .publish((symbol_short!("vote"), proposal_id), amount);

        Ok(())
    }

    /// Execute passed proposal
    pub fn execute_proposal(env: Env, proposal_id: u64) -> Result<bool, &'static str> {
        let storage = env.storage().instance();

        let mut proposal: Proposal = storage
            .get(&DataKey::Proposal(proposal_id))
            .ok_or("Proposal not found")?;

        if env.ledger().timestamp() <= proposal.deadline {
            return Err("Voting period not ended");
        }

        if proposal.executed {
            return Err("Proposal already executed");
        }

        let passed = proposal.votes_for > proposal.votes_against;

        if passed {
            proposal.executed = true;
            storage.set(&DataKey::Proposal(proposal_id), &proposal);
            env.events().publish((symbol_short!("exec"), proposal_id), true);
        }

        Ok(passed)
    }

    /// Get proposal details
    pub fn get_proposal(env: Env, proposal_id: u64) -> Result<Proposal, &'static str> {
        env.storage()
            .instance()
            .get(&DataKey::Proposal(proposal_id))
            .ok_or("Proposal not found")
    }

    /// Delegate voting power
    pub fn delegate_voting_power(
        env: Env,
        from: Address,
        to: Address,
        amount: i128,
    ) -> Result<(), &'static str> {
        from.require_auth();

        if amount <= 0 {
            return Err("Amount must be positive");
        }

        let storage = env.storage().instance();
        let current_power: i128 = storage
            .get(&DataKey::VotingPower(from.clone()))
            .unwrap_or(0);

        if current_power < amount {
            return Err("Insufficient power to delegate");
        }

        // Update from power
        storage.set(&DataKey::VotingPower(from.clone()), &(current_power - amount));

        // Update to power
        let to_power: i128 = storage.get(&DataKey::VotingPower(to.clone())).unwrap_or(0);
        storage.set(&DataKey::VotingPower(to.clone()), &(to_power + amount));

        env.events()
            .publish((symbol_short!("deleg"), from), amount);

        Ok(())
    }
}
