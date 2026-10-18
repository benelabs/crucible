#![no_std]
use soroban_sdk::{
    contract, contracterror, contractimpl, contracttype, symbol_short, Address, Env, String,
    Symbol, Val, Vec,
};

#[contracterror]
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum GovernanceError {
    NotInitialized = 1,
    BasisPointCalculationOverflow = 2,
    InsufficientProposalPower = 3,
    ProposalCounterOverflow = 4,
    DeadlineTooSoon = 5,
    DeadlineTooLate = 6,
    ProposalNotFound = 7,
    VotingPeriodEnded = 8,
    InvalidVoteAmount = 9,
    InsufficientVotingPower = 10,
    AlreadyVoted = 11,
    VoteTotalOverflow = 12,
    VotingPeriodNotEnded = 13,
    ProposalAlreadyExecuted = 14,
    CannotDelegateDuringActiveVote = 15,
    InvalidDelegationAmount = 16,
    InsufficientPowerToDelegate = 17,
    VotingPowerUnderflow = 18,
    VotingPowerOverflow = 19,
}

#[contracttype]
#[derive(Clone)]
pub struct Proposal {
    id: u64,
    title: String,
    description: String,
    proposer: Address,
    target: Address,
    function: Symbol,
    args: Vec<Val>,
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
    LockedUntil(Address),
}

/// DAO Governance Contract with voting capabilities
#[contract]
#[derive(Default)]
pub struct Governance;

/// Minimum voting period: 3 days in seconds (3 * 24 * 60 * 60)
const MIN_VOTING_PERIOD: u64 = 259200;
/// Maximum voting period: 14 days in seconds (14 * 24 * 60 * 60)
const MAX_VOTING_PERIOD: u64 = 1209600;
/// Proposers must control at least 1% of the total voting supply.
const PROPOSAL_THRESHOLD_BPS: i128 = 100;
/// At least 20% of the total supply must participate for a proposal to pass.
const QUORUM_BPS: i128 = 2000;
const BPS_DENOMINATOR: i128 = 10_000;

fn ceil_bps(amount: i128, basis_points: i128) -> Result<i128, GovernanceError> {
    let whole = amount / BPS_DENOMINATOR;
    let remainder = amount % BPS_DENOMINATOR;
    let fractional = remainder
        .checked_mul(basis_points)
        .and_then(|value| value.checked_add(BPS_DENOMINATOR - 1))
        .ok_or(GovernanceError::BasisPointCalculationOverflow)?
        / BPS_DENOMINATOR;

    whole
        .checked_mul(basis_points)
        .and_then(|value| value.checked_add(fractional))
        .ok_or(GovernanceError::BasisPointCalculationOverflow)
}

#[contractimpl]
impl Governance {
    /// Initialize governance with admin and initial token supply
    pub fn initialize(env: Env, admin: Address, initial_supply: i128) {
        admin.require_auth();
        let storage = env.storage().instance();
        assert!(!storage.has(&DataKey::Admin), "Already initialized");
        assert!(initial_supply > 0, "Initial supply must be positive");
        storage.set(&DataKey::Admin, &admin);
        storage.set(&DataKey::TotalSupply, &initial_supply);
        storage.set(&DataKey::ProposalCounter, &0u64);
        storage.set(&DataKey::Balance(admin.clone()), &initial_supply);
        storage.set(&DataKey::VotingPower(admin), &initial_supply);
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
        target: Address,
        function: Symbol,
        args: Vec<Val>,
    ) -> Result<u64, GovernanceError> {
        proposer.require_auth();

        let storage = env.storage().instance();
        let total_supply: i128 = storage
            .get(&DataKey::TotalSupply)
            .ok_or(GovernanceError::NotInitialized)?;
        let proposal_threshold = ceil_bps(total_supply, PROPOSAL_THRESHOLD_BPS)?;
        let proposer_power: i128 = storage
            .get(&DataKey::VotingPower(proposer.clone()))
            .unwrap_or(0);
        if proposer_power < proposal_threshold {
            return Err(GovernanceError::InsufficientProposalPower);
        }

        let mut counter: u64 = storage.get(&DataKey::ProposalCounter).unwrap_or(0);
        counter = counter
            .checked_add(1)
            .ok_or(GovernanceError::ProposalCounterOverflow)?;

        let created_at = env.ledger().timestamp();

        // Validate deadline is within acceptable range
        let min_deadline = created_at + MIN_VOTING_PERIOD;
        let max_deadline = created_at + MAX_VOTING_PERIOD;

        if deadline < min_deadline {
            return Err(GovernanceError::DeadlineTooSoon);
        }

        if deadline > max_deadline {
            return Err(GovernanceError::DeadlineTooLate);
        }

        let proposal = Proposal {
            id: counter,
            title,
            description,
            proposer,
            target,
            function,
            args,
            votes_for: 0,
            votes_against: 0,
            created_at,
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
    ) -> Result<(), GovernanceError> {
        voter.require_auth();

        let storage = env.storage().instance();

        // Get proposal
        let mut proposal: Proposal = storage
            .get(&DataKey::Proposal(proposal_id))
            .ok_or(GovernanceError::ProposalNotFound)?;

        if env.ledger().timestamp() > proposal.deadline {
            return Err(GovernanceError::VotingPeriodEnded);
        }

        if amount <= 0 {
            return Err(GovernanceError::InvalidVoteAmount);
        }

        // Check voting power
        let voting_power: i128 = storage
            .get(&DataKey::VotingPower(voter.clone()))
            .unwrap_or(0);

        if voting_power < amount {
            return Err(GovernanceError::InsufficientVotingPower);
        }

        // Check if voter already cast a vote for this proposal (Issue #680)
        if storage.has(&DataKey::Vote(voter.clone(), proposal_id)) {
            return Err(GovernanceError::AlreadyVoted);
        }

        // Record vote
        let vote = Vote {
            voter: voter.clone(),
            proposal_id,
            amount,
            direction,
        };

        storage.set(&DataKey::Vote(voter.clone(), proposal_id), &vote);

        // Update proposal vote counts with checked arithmetic so large token
        // supplies / vote multipliers cannot overflow i128 (#1002).
        if direction {
            proposal.votes_for = proposal
                .votes_for
                .checked_add(amount)
                .ok_or(GovernanceError::VoteTotalOverflow)?;
        } else {
            proposal.votes_against = proposal
                .votes_against
                .checked_add(amount)
                .ok_or(GovernanceError::VoteTotalOverflow)?;
        }

        let locked_until: u64 = storage
            .get(&DataKey::LockedUntil(voter.clone()))
            .unwrap_or(0);
        if proposal.deadline > locked_until {
            storage.set(&DataKey::LockedUntil(voter.clone()), &proposal.deadline);
        }

        storage.set(&DataKey::Proposal(proposal_id), &proposal);

        env.events()
            .publish((symbol_short!("vote"), proposal_id), amount);

        Ok(())
    }

    /// Execute passed proposal
    pub fn execute_proposal(env: Env, proposal_id: u64) -> Result<bool, GovernanceError> {
        let storage = env.storage().instance();

        let mut proposal: Proposal = storage
            .get(&DataKey::Proposal(proposal_id))
            .ok_or(GovernanceError::ProposalNotFound)?;

        if env.ledger().timestamp() <= proposal.deadline {
            return Err(GovernanceError::VotingPeriodNotEnded);
        }

        if proposal.executed {
            return Err(GovernanceError::ProposalAlreadyExecuted);
        }

        let total_supply: i128 = storage
            .get(&DataKey::TotalSupply)
            .ok_or(GovernanceError::NotInitialized)?;
        let quorum = ceil_bps(total_supply, QUORUM_BPS)?;
        let votes_cast = proposal
            .votes_for
            .checked_add(proposal.votes_against)
            .ok_or(GovernanceError::VoteTotalOverflow)?;
        let passed = votes_cast >= quorum && proposal.votes_for > proposal.votes_against;

        if passed {
            let _: Val =
                env.invoke_contract(&proposal.target, &proposal.function, proposal.args.clone());
            proposal.executed = true;
            storage.set(&DataKey::Proposal(proposal_id), &proposal);
            env.events().publish((symbol_short!("exec"), proposal_id), true);
        }

        Ok(passed)
    }

    /// Get proposal details
    pub fn get_proposal(env: Env, proposal_id: u64) -> Result<Proposal, GovernanceError> {
        env.storage()
            .instance()
            .get(&DataKey::Proposal(proposal_id))
            .ok_or(GovernanceError::ProposalNotFound)
    }

    /// Delegate voting power
    pub fn delegate_voting_power(
        env: Env,
        from: Address,
        to: Address,
        amount: i128,
    ) -> Result<(), GovernanceError> {
        from.require_auth();

        if amount <= 0 {
            return Err(GovernanceError::InvalidDelegationAmount);
        }

        let storage = env.storage().instance();
        let locked_until: u64 = storage
            .get(&DataKey::LockedUntil(from.clone()))
            .unwrap_or(0);
        if locked_until >= env.ledger().timestamp() {
            return Err(GovernanceError::CannotDelegateDuringActiveVote);
        }

        let current_power: i128 = storage
            .get(&DataKey::VotingPower(from.clone()))
            .unwrap_or(0);

        if current_power < amount {
            return Err(GovernanceError::InsufficientPowerToDelegate);
        }

        // Update from power
        let from_power = current_power
            .checked_sub(amount)
            .ok_or(GovernanceError::VotingPowerUnderflow)?;
        storage.set(&DataKey::VotingPower(from.clone()), &from_power);

        // Update to power
        let to_power: i128 = storage.get(&DataKey::VotingPower(to.clone())).unwrap_or(0);
        let to_power = to_power
            .checked_add(amount)
            .ok_or(GovernanceError::VotingPowerOverflow)?;
        storage.set(&DataKey::VotingPower(to.clone()), &to_power);

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
    use soroban_sdk::{contract, contractimpl, IntoVal};

    #[contract]
    pub struct ActionTarget;

    #[contractimpl]
    impl ActionTarget {
        pub fn set_value(env: Env, value: i128) {
            env.storage().instance().set(&symbol_short!("value"), &value);
        }

        pub fn value(env: Env) -> i128 {
            env.storage().instance().get(&symbol_short!("value")).unwrap_or(0)
        }
    }

    fn proposal_args(env: &Env) -> (Symbol, Vec<Val>) {
        (
            Symbol::new(env, "set_value"),
            soroban_sdk::vec![env, 42i128.into_val(env)],
        )
    }

    #[test]
    fn active_vote_prevents_delegation_and_proposals_require_threshold() {
        let env = Env::default();
        let contract_id = env.register_contract(None, Governance);
        let client = GovernanceClient::new(&env, &contract_id);
        let target_id = env.register_contract(None, ActionTarget);

        let admin = Address::generate(&env);
        let voter = Address::generate(&env);

        env.mock_all_auths();
        client.initialize(&admin, &1_000_000);

        let deadline = env.ledger().timestamp() + MIN_VOTING_PERIOD;
        let (function, args) = proposal_args(&env);
        let title = soroban_sdk::String::from_str(&env, "Test Proposal");
        let description = soroban_sdk::String::from_str(&env, "Desc");

        assert!(matches!(
            client.try_create_proposal(
                &voter,
                &title,
                &description,
                &deadline,
                &target_id,
                &function,
                &args,
            ),
            Ok(Err(GovernanceError::InsufficientProposalPower))
        ));

        let proposal_id = client.create_proposal(
            &admin,
            &title,
            &description,
            &deadline,
            &target_id,
            &function,
            &args,
        );
        client.vote(&admin, &proposal_id, &200_000i128, &true);

        assert!(matches!(
            client.try_delegate_voting_power(&admin, &voter, &100_000i128),
            Ok(Err(GovernanceError::CannotDelegateDuringActiveVote))
        ));

        env.ledger().set_timestamp(deadline + 1);
        client.delegate_voting_power(&admin, &voter, &100_000i128);
    }

    #[test]
    fn quorum_is_required_and_passed_proposal_dispatches_action() {
        let env = Env::default();
        let contract_id = env.register_contract(None, Governance);
        let client = GovernanceClient::new(&env, &contract_id);
        let target_id = env.register_contract(None, ActionTarget);
        let target = ActionTargetClient::new(&env, &target_id);
        let admin = Address::generate(&env);

        env.mock_all_auths();
        client.initialize(&admin, &1_000i128);

        let deadline = env.ledger().timestamp() + MIN_VOTING_PERIOD;
        let (function, args) = proposal_args(&env);
        let title = soroban_sdk::String::from_str(&env, "Test Proposal");
        let description = soroban_sdk::String::from_str(&env, "Desc");
        let below_quorum = client.create_proposal(
            &admin,
            &title,
            &description,
            &deadline,
            &target_id,
            &function,
            &args,
        );
        let meets_quorum = client.create_proposal(
            &admin,
            &title,
            &description,
            &deadline,
            &target_id,
            &function,
            &args,
        );

        client.vote(&admin, &below_quorum, &100i128, &true);
        client.vote(&admin, &meets_quorum, &200i128, &true);
        env.ledger().set_timestamp(deadline + 1);

        assert!(!client.execute_proposal(&below_quorum));
        assert_eq!(target.value(), 0);
        assert!(client.execute_proposal(&meets_quorum));
        assert_eq!(target.value(), 42);
    }
}
