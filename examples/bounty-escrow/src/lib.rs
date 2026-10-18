// Location: examples/bounty-escrow/src/lib.rs // Production requirement: Decentralized Bounty & Task Escrow with Milestone Payouts
#![no_std]
use soroban_sdk::{
    contract, contractimpl, contracttype, symbol_short, token, Address, Env, String, Vec,
};

/// Status lifecycle for individual milestones.
#[contracttype]
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum MilestoneStatus {
    Pending,
    Submitted,
    Approved,
    Disputed,
}

/// Metadata and state for an individual milestone.
#[contracttype]
#[derive(Clone, Debug, PartialEq)]
pub struct Milestone {
    pub id: u32,
    pub payout_amount: i128,
    pub status: MilestoneStatus,
    pub submission_ref: String,
}

/// Global state for the bounty escrow.
#[contracttype]
#[derive(Clone, Debug, PartialEq)]
pub struct BountyState {
    pub creator: Address,
    pub contributor: Address,
    pub arbiter: Address,
    pub token: Address,
    pub total_bounty: i128,
    pub released_total: i128,
    pub milestones_count: u32,
    pub is_cancelled: bool,
}

#[contracttype]
enum DataKey {
    State,
    Milestone(u32),
}

/// Decentralized Bounty & Task Escrow with Milestone Payouts.
///
/// Features:
/// - Milestone state tracking: `Pending` -> `Submitted` -> `Approved` / `Disputed`.
/// - Proportional fund release upon milestone approval.
/// - Dispute freezing preventing unauthorized submissions and releases during disputes.
/// - Arbiter resolution for disputes (either approving payout or returning milestone to Pending).
/// - Unreleased fund reclamation if bounty cancelled before completion.
#[contract]
#[derive(Default)]
pub struct BountyEscrow;

#[contractimpl]
impl BountyEscrow {
    /// Initialize the bounty escrow contract with milestones and transfer total funds from creator.
    pub fn create_bounty(
        env: Env,
        creator: Address,
        contributor: Address,
        arbiter: Address,
        token: Address,
        milestone_payouts: Vec<i128>,
    ) {
        if env.storage().instance().has(&DataKey::State) {
            panic!("bounty already exists");
        }
        if milestone_payouts.is_empty() {
            panic!("at least one milestone required");
        }
        creator.require_auth();

        let mut total_bounty: i128 = 0;
        let count = milestone_payouts.len();

        for i in 0..count {
            let payout = milestone_payouts.get(i).unwrap();
            if payout <= 0 {
                panic!("milestone payout must be positive");
            }
            total_bounty = total_bounty
                .checked_add(payout)
                .unwrap_or_else(|| panic!("bounty amount overflow"));

            let milestone = Milestone {
                id: i,
                payout_amount: payout,
                status: MilestoneStatus::Pending,
                submission_ref: String::from_str(&env, ""),
            };
            env.storage().instance().set(&DataKey::Milestone(i), &milestone);
        }

        token::TokenClient::new(&env, &token).transfer(
            &creator,
            &env.current_contract_address(),
            &total_bounty,
        );

        let state = BountyState {
            creator: creator.clone(),
            contributor,
            arbiter,
            token,
            total_bounty,
            released_total: 0,
            milestones_count: count,
            is_cancelled: false,
        };

        env.storage().instance().set(&DataKey::State, &state);
        env.events().publish((symbol_short!("created"), creator), total_bounty);
    }

    /// Contributor submits deliverable proof/reference for a pending milestone.
    pub fn submit_milestone(env: Env, milestone_id: u32, submission_ref: String) {
        let state = Self::get_bounty(env.clone());
        if state.is_cancelled {
            panic!("bounty is cancelled");
        }
        state.contributor.require_auth();

        let mut milestone = Self::get_milestone(env.clone(), milestone_id);
        if milestone.status == MilestoneStatus::Disputed {
            panic!("milestone is disputed and frozen");
        }
        if milestone.status == MilestoneStatus::Approved {
            panic!("milestone already approved");
        }
        if milestone.status != MilestoneStatus::Pending && milestone.status != MilestoneStatus::Submitted {
            panic!("milestone is not pending");
        }

        milestone.status = MilestoneStatus::Submitted;
        milestone.submission_ref = submission_ref;
        env.storage().instance().set(&DataKey::Milestone(milestone_id), &milestone);

        env.events().publish(
            (symbol_short!("submit"), milestone_id),
            milestone.payout_amount,
        );
    }

    /// Creator or Arbiter approves deliverable and releases proportional milestone funds.
    pub fn approve_milestone(env: Env, milestone_id: u32, caller: Address) {
        let mut state = Self::get_bounty(env.clone());
        if state.is_cancelled {
            panic!("bounty is cancelled");
        }
        if caller != state.creator && caller != state.arbiter {
            panic!("only creator or arbiter can approve milestone");
        }
        caller.require_auth();

        let mut milestone = Self::get_milestone(env.clone(), milestone_id);
        if milestone.status == MilestoneStatus::Disputed {
            panic!("milestone is disputed and frozen");
        }
        if milestone.status == MilestoneStatus::Approved {
            panic!("milestone already approved");
        }
        if milestone.status != MilestoneStatus::Submitted {
            panic!("milestone must be submitted before approval");
        }

        milestone.status = MilestoneStatus::Approved;
        env.storage().instance().set(&DataKey::Milestone(milestone_id), &milestone);

        state.released_total = state
            .released_total
            .checked_add(milestone.payout_amount)
            .unwrap_or_else(|| panic!("released total overflow"));
        env.storage().instance().set(&DataKey::State, &state);

        token::TokenClient::new(&env, &state.token).transfer(
            &env.current_contract_address(),
            &state.contributor,
            &milestone.payout_amount,
        );

        env.events().publish(
            (symbol_short!("approve"), milestone_id),
            (state.contributor.clone(), milestone.payout_amount),
        );
    }

    /// Raise a dispute on a milestone, freezing fund releases and submissions.
    pub fn raise_dispute(env: Env, milestone_id: u32, caller: Address) {
        let state = Self::get_bounty(env.clone());
        if state.is_cancelled {
            panic!("bounty is cancelled");
        }
        if caller != state.creator && caller != state.contributor && caller != state.arbiter {
            panic!("caller not authorized to dispute");
        }
        caller.require_auth();

        let mut milestone = Self::get_milestone(env.clone(), milestone_id);
        if milestone.status == MilestoneStatus::Approved {
            panic!("cannot dispute already approved milestone");
        }
        if milestone.status == MilestoneStatus::Disputed {
            panic!("milestone already disputed");
        }

        milestone.status = MilestoneStatus::Disputed;
        env.storage().instance().set(&DataKey::Milestone(milestone_id), &milestone);

        env.events().publish(
            (symbol_short!("dispute"), milestone_id),
            caller,
        );
    }

    /// Arbiter resolves dispute: if `approve_payout` is true, funds are released to contributor;
    /// if false, milestone is reset to `Pending` without releasing funds.
    pub fn resolve_dispute(env: Env, milestone_id: u32, arbiter: Address, approve_payout: bool) {
        let mut state = Self::get_bounty(env.clone());
        if state.is_cancelled {
            panic!("bounty is cancelled");
        }
        if arbiter != state.arbiter {
            panic!("only arbiter can resolve dispute");
        }
        arbiter.require_auth();

        let mut milestone = Self::get_milestone(env.clone(), milestone_id);
        if milestone.status != MilestoneStatus::Disputed {
            panic!("milestone is not disputed");
        }

        if approve_payout {
            milestone.status = MilestoneStatus::Approved;
            env.storage().instance().set(&DataKey::Milestone(milestone_id), &milestone);

            state.released_total = state
                .released_total
                .checked_add(milestone.payout_amount)
                .unwrap_or_else(|| panic!("released total overflow"));
            env.storage().instance().set(&DataKey::State, &state);

            token::TokenClient::new(&env, &state.token).transfer(
                &env.current_contract_address(),
                &state.contributor,
                &milestone.payout_amount,
            );
        } else {
            milestone.status = MilestoneStatus::Pending;
            milestone.submission_ref = String::from_str(&env, "");
            env.storage().instance().set(&DataKey::Milestone(milestone_id), &milestone);
        }

        env.events().publish(
            (symbol_short!("resolve"), milestone_id),
            (approve_payout, milestone.payout_amount),
        );
    }

    /// Cancel bounty and refund unreleased funds back to creator (creator auth or arbiter auth).
    pub fn cancel_and_refund(env: Env, caller: Address) {
        let mut state = Self::get_bounty(env.clone());
        if state.is_cancelled {
            panic!("bounty already cancelled");
        }
        if caller != state.creator && caller != state.arbiter {
            panic!("only creator or arbiter can cancel");
        }
        caller.require_auth();

        // Check if any milestone is currently disputed
        for i in 0..state.milestones_count {
            let m = Self::get_milestone(env.clone(), i);
            if m.status == MilestoneStatus::Disputed {
                panic!("cannot cancel while milestone is disputed");
            }
        }

        let unreleased = state.total_bounty - state.released_total;
        state.is_cancelled = true;
        env.storage().instance().set(&DataKey::State, &state);

        if unreleased > 0 {
            token::TokenClient::new(&env, &state.token).transfer(
                &env.current_contract_address(),
                &state.creator,
                &unreleased,
            );
        }

        env.events().publish(
            (symbol_short!("refund"), caller),
            unreleased,
        );
    }

    /// Return bounty state.
    pub fn get_bounty(env: Env) -> BountyState {
        env.storage()
            .instance()
            .get(&DataKey::State)
            .unwrap_or_else(|| panic!("bounty not found"))
    }

    /// Return details for a single milestone.
    pub fn get_milestone(env: Env, milestone_id: u32) -> Milestone {
        env.storage()
            .instance()
            .get(&DataKey::Milestone(milestone_id))
            .unwrap_or_else(|| panic!("milestone not found"))
    }

    /// Return all milestones.
    pub fn get_milestones(env: Env) -> Vec<Milestone> {
        let state = Self::get_bounty(env.clone());
        let mut list = Vec::new(&env);
        for i in 0..state.milestones_count {
            list.push_back(Self::get_milestone(env.clone(), i));
        }
        list
    }
}

#[cfg(test)]
mod test;
