#![cfg(test)]
extern crate std;

use crucible::prelude::*;
use crucible::{assert_emitted, assert_reverts};
use soroban_sdk::{symbol_short, vec, Address, String, Vec};

use crate::{
    BountyEscrow, BountyEscrowClient, BountyState, Milestone, MilestoneStatus,
};

const PAYOUT_1: i128 = 300_000;
const PAYOUT_2: i128 = 700_000;
const TOTAL_BOUNTY: i128 = PAYOUT_1 + PAYOUT_2;

struct Ctx {
    pub env: MockEnv,
    pub id: Address,
    pub creator: AccountHandle,
    pub contributor: AccountHandle,
    pub arbiter: AccountHandle,
    pub stranger: AccountHandle,
    pub token: MockToken,
}

impl Ctx {
    fn setup() -> Self {
        let env = MockEnv::builder()
            .at_timestamp(1_000_000)
            .with_contract::<BountyEscrow>()
            .with_account("creator", Stroops::xlm(100))
            .with_account("contributor", Stroops::xlm(10))
            .with_account("arbiter", Stroops::xlm(10))
            .with_account("stranger", Stroops::xlm(10))
            .build();

        let id = env.contract_id::<BountyEscrow>();
        let creator = env.account("creator");
        let contributor = env.account("contributor");
        let arbiter = env.account("arbiter");
        let stranger = env.account("stranger");
        let token = MockToken::new(&env, "USDC", 6);
        token.mint(&creator, TOTAL_BOUNTY * 2);

        Ctx {
            env,
            id,
            creator,
            contributor,
            arbiter,
            stranger,
            token,
        }
    }

    fn client(&self) -> BountyEscrowClient<'_> {
        BountyEscrowClient::new(self.env.inner(), &self.id)
    }

    fn init_bounty(&self) {
        let payouts = vec![self.env.inner(), PAYOUT_1, PAYOUT_2];
        self.env.with_mock_all_auths(|| {
            self.client().create_bounty(
                &self.creator,
                &self.contributor,
                &self.arbiter,
                &self.token.address(),
                &payouts,
            );
        });
    }
}

// ---------------------------------------------------------------------------
// Bounty creation & initialization
// ---------------------------------------------------------------------------

#[test]
fn test_create_bounty_transfers_total_and_inits_milestones() {
    let ctx = Ctx::setup();
    ctx.init_bounty();

    // Verify token balances
    assert_eq!(ctx.token.balance(&ctx.id), TOTAL_BOUNTY);
    assert_eq!(ctx.token.balance(&ctx.creator), TOTAL_BOUNTY);

    // Verify state
    let bounty: BountyState = ctx.client().get_bounty();
    assert_eq!(bounty.total_bounty, TOTAL_BOUNTY);
    assert_eq!(bounty.released_total, 0);
    assert_eq!(bounty.milestones_count, 2);
    assert_eq!(bounty.is_cancelled, false);

    // Verify milestones
    let m0 = ctx.client().get_milestone(&0);
    assert_eq!(m0.status, MilestoneStatus::Pending);
    assert_eq!(m0.payout_amount, PAYOUT_1);

    let m1 = ctx.client().get_milestone(&1);
    assert_eq!(m1.status, MilestoneStatus::Pending);
    assert_eq!(m1.payout_amount, PAYOUT_2);

    let list = ctx.client().get_milestones();
    assert_eq!(list.len(), 2);
}

#[test]
fn test_create_bounty_emits_event() {
    let ctx = Ctx::setup();
    let payouts = vec![ctx.env.inner(), PAYOUT_1, PAYOUT_2];
    ctx.env.with_mock_all_auths(|| {
        ctx.client().create_bounty(
            &ctx.creator,
            &ctx.contributor,
            &ctx.arbiter,
            &ctx.token.address(),
            &payouts,
        );
    });

    assert_emitted!(
        ctx.env,
        ctx.id,
        (symbol_short!("created"), ctx.creator.address()),
        TOTAL_BOUNTY
    );
}

#[test]
fn test_create_with_empty_milestones_reverts() {
    let ctx = Ctx::setup();
    let empty: Vec<i128> = Vec::new(ctx.env.inner());
    ctx.env.mock_all_auths();
    assert_reverts!(
        ctx.client().create_bounty(
            &ctx.creator,
            &ctx.contributor,
            &ctx.arbiter,
            &ctx.token.address(),
            &empty,
        ),
        "at least one milestone required"
    );
}

// ---------------------------------------------------------------------------
// Milestone state transitions & proportional payouts
// ---------------------------------------------------------------------------

#[test]
fn test_milestone_progression_and_proportional_payout() {
    let ctx = Ctx::setup();
    ctx.init_bounty();

    // 1. Contributor submits milestone 0
    let submission_1 = String::from_str(ctx.env.inner(), "ipfs://QmDeliverable1");
    ctx.env.with_mock_all_auths(|| {
        ctx.client().submit_milestone(&0, &submission_1);
    });

    let m0_after_sub = ctx.client().get_milestone(&0);
    assert_eq!(m0_after_sub.status, MilestoneStatus::Submitted);
    assert_eq!(m0_after_sub.submission_ref, submission_1);

    // 2. Creator approves milestone 0 -> Proportional payout released
    ctx.env.with_mock_all_auths(|| {
        ctx.client().approve_milestone(&0, &ctx.creator);
    });

    let m0_after_app = ctx.client().get_milestone(&0);
    assert_eq!(m0_after_app.status, MilestoneStatus::Approved);
    assert_eq!(ctx.token.balance(&ctx.contributor), PAYOUT_1);
    assert_eq!(ctx.token.balance(&ctx.id), TOTAL_BOUNTY - PAYOUT_1);

    let bounty_after_0 = ctx.client().get_bounty();
    assert_eq!(bounty_after_0.released_total, PAYOUT_1);

    // 3. Contributor submits and gets approval for milestone 1
    let submission_2 = String::from_str(ctx.env.inner(), "ipfs://QmDeliverable2");
    ctx.env.with_mock_all_auths(|| {
        ctx.client().submit_milestone(&1, &submission_2);
    });
    ctx.env.with_mock_all_auths(|| {
        ctx.client().approve_milestone(&1, &ctx.creator);
    });

    let m1_after_app = ctx.client().get_milestone(&1);
    assert_eq!(m1_after_app.status, MilestoneStatus::Approved);
    assert_eq!(ctx.token.balance(&ctx.contributor), TOTAL_BOUNTY);
    assert_eq!(ctx.token.balance(&ctx.id), 0);

    let bounty_final = ctx.client().get_bounty();
    assert_eq!(bounty_final.released_total, TOTAL_BOUNTY);
}

#[test]
fn test_arbiter_can_approve_submitted_milestone() {
    let ctx = Ctx::setup();
    ctx.init_bounty();

    let submission = String::from_str(ctx.env.inner(), "ipfs://QmDeliverable1");
    ctx.env.with_mock_all_auths(|| {
        ctx.client().submit_milestone(&0, &submission);
    });

    // Arbiter approves instead of creator
    ctx.env.with_mock_all_auths(|| {
        ctx.client().approve_milestone(&0, &ctx.arbiter);
    });

    assert_eq!(
        ctx.client().get_milestone(&0).status,
        MilestoneStatus::Approved
    );
    assert_eq!(ctx.token.balance(&ctx.contributor), PAYOUT_1);
}

#[test]
fn test_approve_unsubmitted_milestone_reverts() {
    let ctx = Ctx::setup();
    ctx.init_bounty();

    ctx.env.mock_all_auths();
    assert_reverts!(
        ctx.client().approve_milestone(&0, &ctx.creator),
        "milestone must be submitted before approval"
    );
}

#[test]
fn test_stranger_cannot_approve_milestone() {
    let ctx = Ctx::setup();
    ctx.init_bounty();

    let submission = String::from_str(ctx.env.inner(), "ipfs://QmDeliverable1");
    ctx.env.with_mock_all_auths(|| {
        ctx.client().submit_milestone(&0, &submission);
    });

    ctx.env.mock_all_auths();
    assert_reverts!(
        ctx.client().approve_milestone(&0, &ctx.stranger),
        "only creator or arbiter can approve milestone"
    );
}

// ---------------------------------------------------------------------------
// Dispute freeze & Arbiter resolution
// ---------------------------------------------------------------------------

#[test]
fn test_dispute_freezes_milestone_actions() {
    let ctx = Ctx::setup();
    ctx.init_bounty();

    let submission = String::from_str(ctx.env.inner(), "ipfs://QmDeliverable1");
    ctx.env.with_mock_all_auths(|| {
        ctx.client().submit_milestone(&0, &submission);
    });

    // Creator raises dispute
    ctx.env.with_mock_all_auths(|| {
        ctx.client().raise_dispute(&0, &ctx.creator);
    });

    assert_eq!(
        ctx.client().get_milestone(&0).status,
        MilestoneStatus::Disputed
    );

    // Approval while disputed is frozen and reverts
    ctx.env.mock_all_auths();
    assert_reverts!(
        ctx.client().approve_milestone(&0, &ctx.creator),
        "milestone is disputed and frozen"
    );

    // Submission while disputed is frozen and reverts
    let new_submission = String::from_str(ctx.env.inner(), "ipfs://QmDeliverable1Revised");
    assert_reverts!(
        ctx.client().submit_milestone(&0, &new_submission),
        "milestone is disputed and frozen"
    );
}

#[test]
fn test_arbiter_resolves_dispute_in_favor_of_contributor() {
    let ctx = Ctx::setup();
    ctx.init_bounty();

    let submission = String::from_str(ctx.env.inner(), "ipfs://QmDeliverable1");
    ctx.env.with_mock_all_auths(|| {
        ctx.client().submit_milestone(&0, &submission);
        ctx.client().raise_dispute(&0, &ctx.creator);
    });

    // Arbiter resolves dispute with approve_payout = true
    ctx.env.with_mock_all_auths(|| {
        ctx.client().resolve_dispute(&0, &ctx.arbiter, &true);
    });

    assert_eq!(
        ctx.client().get_milestone(&0).status,
        MilestoneStatus::Approved
    );
    assert_eq!(ctx.token.balance(&ctx.contributor), PAYOUT_1);
    assert_eq!(ctx.client().get_bounty().released_total, PAYOUT_1);
}

#[test]
fn test_arbiter_resolves_dispute_resetting_to_pending() {
    let ctx = Ctx::setup();
    ctx.init_bounty();

    let submission = String::from_str(ctx.env.inner(), "ipfs://QmDeliverable1");
    ctx.env.with_mock_all_auths(|| {
        ctx.client().submit_milestone(&0, &submission);
        ctx.client().raise_dispute(&0, &ctx.creator);
    });

    // Arbiter resolves dispute with approve_payout = false -> resets to Pending
    ctx.env.with_mock_all_auths(|| {
        ctx.client().resolve_dispute(&0, &ctx.arbiter, &false);
    });

    let m0 = ctx.client().get_milestone(&0);
    assert_eq!(m0.status, MilestoneStatus::Pending);
    assert_eq!(ctx.token.balance(&ctx.contributor), 0);

    // Contributor can now re-submit with revised deliverables
    let revised_sub = String::from_str(ctx.env.inner(), "ipfs://QmDeliverable1Fixed");
    ctx.env.with_mock_all_auths(|| {
        ctx.client().submit_milestone(&0, &revised_sub);
        ctx.client().approve_milestone(&0, &ctx.creator);
    });

    assert_eq!(
        ctx.client().get_milestone(&0).status,
        MilestoneStatus::Approved
    );
    assert_eq!(ctx.token.balance(&ctx.contributor), PAYOUT_1);
}

#[test]
fn test_cannot_dispute_already_approved_milestone() {
    let ctx = Ctx::setup();
    ctx.init_bounty();

    let submission = String::from_str(ctx.env.inner(), "ipfs://QmDeliverable1");
    ctx.env.with_mock_all_auths(|| {
        ctx.client().submit_milestone(&0, &submission);
        ctx.client().approve_milestone(&0, &ctx.creator);
    });

    ctx.env.mock_all_auths();
    assert_reverts!(
        ctx.client().raise_dispute(&0, &ctx.creator),
        "cannot dispute already approved milestone"
    );
}

// ---------------------------------------------------------------------------
// Cancellation and Refund of Unreleased Funds
// ---------------------------------------------------------------------------

#[test]
fn test_cancel_and_refund_unreleased_funds() {
    let ctx = Ctx::setup();
    ctx.init_bounty();

    // Approve milestone 0 (300k paid out)
    let submission = String::from_str(ctx.env.inner(), "ipfs://QmDeliverable1");
    ctx.env.with_mock_all_auths(|| {
        ctx.client().submit_milestone(&0, &submission);
        ctx.client().approve_milestone(&0, &ctx.creator);
    });

    assert_eq!(ctx.token.balance(&ctx.creator), TOTAL_BOUNTY);

    // Cancel bounty and refund unreleased 700k back to creator
    ctx.env.with_mock_all_auths(|| {
        ctx.client().cancel_and_refund(&ctx.creator);
    });

    let bounty = ctx.client().get_bounty();
    assert_eq!(bounty.is_cancelled, true);
    assert_eq!(ctx.token.balance(&ctx.creator), TOTAL_BOUNTY + PAYOUT_2);
    assert_eq!(ctx.token.balance(&ctx.id), 0);
}

#[test]
fn test_cannot_cancel_while_dispute_active() {
    let ctx = Ctx::setup();
    ctx.init_bounty();

    ctx.env.with_mock_all_auths(|| {
        ctx.client().raise_dispute(&0, &ctx.creator);
    });

    ctx.env.mock_all_auths();
    assert_reverts!(
        ctx.client().cancel_and_refund(&ctx.creator),
        "cannot cancel while milestone is disputed"
    );
}
