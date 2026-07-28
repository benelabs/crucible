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

        // Check if voter already cast a vote for this proposal (Issue #680)
        if storage.has(&DataKey::Vote(voter.clone(), proposal_id)) {
            return Err("Already voted");
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

// Gas Optimization via Bit-Packing (Issue #704)
// Packing multiple booleans and small integers into a single 64-bit storage key.
// [bool flag1] [bool flag2] [u8 small_int] [u32 large_int] [16 bits reserved]
// Total = 1 + 1 + 8 + 32 = 42 bits used, easily fits in u64.
#[contracttype]
#[derive(Clone, Copy)]
pub struct PackedState {
    pub packed: u64,
}

impl PackedState {
    pub fn new(flag1: bool, flag2: bool, small_int: u8, large_int: u32) -> Self {
        let mut packed: u64 = 0;
        if flag1 { packed |= 1 << 0; }
        if flag2 { packed |= 1 << 1; }
        packed |= (small_int as u64) << 2;
        packed |= (large_int as u64) << 10;
        Self { packed }
    }

    pub fn flag1(&self) -> bool {
        (self.packed & (1 << 0)) != 0
    }

    pub fn flag2(&self) -> bool {
        (self.packed & (1 << 1)) != 0
    }

    pub fn small_int(&self) -> u8 {
        ((self.packed >> 2) & 0xFF) as u8
    }

    pub fn large_int(&self) -> u32 {
        ((self.packed >> 10) & 0xFFFFFFFF) as u32
    }
}

#[cfg(test)]
mod test {
    use super::*;
    use soroban_sdk::Env;

    #[test]
    fn test_double_voting_prevention() {
        let env = Env::default();
        let contract_id = env.register_contract(None, Governance);
        let client = GovernanceClient::new(&env, &contract_id);

        let admin = Address::generate(&env);
        let voter = Address::generate(&env);

        env.mock_all_auths();

        client.initialize(&admin, &1_000_000);

        // Grant voting power
        let storage = env.storage().instance();
        // Register proposal
        let deadline = env.ledger().timestamp() + 1000;
        let prop_id = client.create_proposal(&admin, &soroban_sdk::String::from_str(&env, "Test Proposal"), &soroban_sdk::String::from_str(&env, "Desc"), &deadline);

        // Set voting power
        env.as_contract(&contract_id, || {
            env.storage().instance().set(&DataKey::VotingPower(voter.clone()), &500i128);
        });

        // First vote should succeed
        let res1 = client.try_vote(&voter, &prop_id, &100i128, &true);
        assert!(res1.is_ok());

        // Second vote should fail with Already voted
        let res2 = client.try_vote(&voter, &prop_id, &100i128, &true);
        assert!(res2.is_err());
    }
}
