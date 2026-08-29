//! Tests for the multi-arbiter escrow contract.
//!
//! Exercises the M-of-1 approval model: any single arbiter from the registered
//! set may approve early release, while none can approve twice or override a
//! settled escrow.
#![cfg(test)]
extern crate std;

use crucible::prelude::*;
use crucible::{assert_emitted, assert_reverts};
use soroban_sdk::{symbol_short, vec, Address};

use crate::{EscrowStatus, MultiArbiterEscrow, MultiArbiterEscrowClient};

const AMOUNT: i128 = 1_000_000;
const BASE_TIME: u64 = 1_000_000;
const LOCK_DURATION: u64 = 86_400;

// ---------------------------------------------------------------------------
// Fixture
// ---------------------------------------------------------------------------

struct Ctx {
    pub env: MockEnv,
    pub id: Address,
    pub depositor: AccountHandle,
    pub recipient: AccountHandle,
    pub arbiter_a: AccountHandle,
    pub arbiter_b: AccountHandle,
    pub arbiter_c: AccountHandle,
    pub token: MockToken,
}

impl Ctx {
    fn setup() -> Self {
        let env = MockEnv::builder()
            .at_timestamp(BASE_TIME)
            .with_contract::<MultiArbiterEscrow>()
            .with_account("depositor", Stroops::xlm(100))
            .with_account("recipient", Stroops::xlm(10))
            .with_account("arbiter_a", Stroops::xlm(10))
            .with_account("arbiter_b", Stroops::xlm(10))
            .with_account("arbiter_c", Stroops::xlm(10))
            .build();

        let id = env.contract_id::<MultiArbiterEscrow>();
        let depositor = env.account("depositor");
        let recipient = env.account("recipient");
        let arbiter_a = env.account("arbiter_a");
        let arbiter_b = env.account("arbiter_b");
        let arbiter_c = env.account("arbiter_c");
        let token = MockToken::new(&env, "USDC", 6);
        token.mint(&depositor, AMOUNT * 3);

        Ctx {
            env,
            id,
            depositor,
            recipient,
            arbiter_a,
            arbiter_b,
            arbiter_c,
            token,
        }
    }

    fn client(&self) -> MultiArbiterEscrowClient<'_> {
        MultiArbiterEscrowClient::new(self.env.inner(), &self.id)
    }

    fn arbiters(&self) -> soroban_sdk::Vec<Address> {
        vec![
            self.env.inner(),
            self.arbiter_a.address(),
            self.arbiter_b.address(),
            self.arbiter_c.address(),
        ]
    }

    fn create_escrow(&self) {
        self.env.with_mock_all_auths(|| {
            self.client().create(
                &self.depositor,
                &self.recipient,
                &self.arbiters(),
                &self.token.address(),
                &AMOUNT,
                &(BASE_TIME + LOCK_DURATION),
            );
        });
    }
}

// ---------------------------------------------------------------------------
// Creation
// ---------------------------------------------------------------------------

#[test]
fn test_create_transfers_tokens_to_contract() {
    let ctx = Ctx::setup();
    ctx.create_escrow();

    assert_eq!(ctx.token.balance(&ctx.id), AMOUNT);
    assert_eq!(ctx.token.balance(&ctx.depositor), AMOUNT * 2);
}

#[test]
fn test_create_emits_event() {
    let ctx = Ctx::setup();
    ctx.env.with_mock_all_auths(|| {
        ctx.client().create(
            &ctx.depositor,
            &ctx.recipient,
            &ctx.arbiters(),
            &ctx.token.address(),
            &AMOUNT,
            &(BASE_TIME + LOCK_DURATION),
        );
    });
    assert_emitted!(ctx.env, ctx.id, (symbol_short!("created"),), AMOUNT);
}

#[test]
fn test_create_with_zero_arbiters_reverts() {
    let ctx = Ctx::setup();
    let empty: soroban_sdk::Vec<Address> = soroban_sdk::Vec::new(ctx.env.inner());
    ctx.env.mock_all_auths();
    assert_reverts!(
        ctx.client().create(
            &ctx.depositor,
            &ctx.recipient,
            &empty,
            &ctx.token.address(),
            &AMOUNT,
            &(BASE_TIME + LOCK_DURATION),
        ),
        "at least one arbiter required"
    );
}

// ---------------------------------------------------------------------------
// Approval by any arbiter
// ---------------------------------------------------------------------------

#[test]
fn test_arbiter_a_can_approve() {
    let ctx = Ctx::setup();
    ctx.create_escrow();

    ctx.env
        .with_mock_all_auths(|| ctx.client().approve(&ctx.arbiter_a));
    assert_eq!(ctx.client().get_state().status, EscrowStatus::Approved);
}

#[test]
fn test_arbiter_b_can_approve() {
    let ctx = Ctx::setup();
    ctx.create_escrow();

    ctx.env
        .with_mock_all_auths(|| ctx.client().approve(&ctx.arbiter_b));
    assert_eq!(ctx.client().get_state().status, EscrowStatus::Approved);
}

#[test]
fn test_arbiter_c_can_approve() {
    let ctx = Ctx::setup();
    ctx.create_escrow();

    ctx.env
        .with_mock_all_auths(|| ctx.client().approve(&ctx.arbiter_c));
    assert_eq!(ctx.client().get_state().status, EscrowStatus::Approved);
}

#[test]
fn test_non_arbiter_cannot_approve() {
    let ctx = Ctx::setup();
    ctx.create_escrow();

    ctx.env.mock_all_auths();
    // recipient is not in the arbiters list
    assert_reverts!(
        ctx.client().approve(&ctx.recipient),
        "caller is not a registered arbiter"
    );
}

#[test]
fn test_depositor_cannot_approve() {
    let ctx = Ctx::setup();
    ctx.create_escrow();

    ctx.env.mock_all_auths();
    assert_reverts!(
        ctx.client().approve(&ctx.depositor),
        "caller is not a registered arbiter"
    );
}

// ---------------------------------------------------------------------------
// Claim paths
// ---------------------------------------------------------------------------

#[test]
fn test_claim_after_arbiter_approval_succeeds_before_timeout() {
    let ctx = Ctx::setup();
    ctx.create_escrow();

    // Approve without advancing time.
    ctx.env
        .with_mock_all_auths(|| ctx.client().approve(&ctx.arbiter_b));
    ctx.env.with_mock_all_auths(|| ctx.client().claim());

    assert_eq!(ctx.token.balance(&ctx.recipient), AMOUNT);
    assert_eq!(ctx.token.balance(&ctx.id), 0);
    assert_eq!(ctx.client().get_state().status, EscrowStatus::Claimed);
}

#[test]
fn test_claim_after_timeout_without_approval() {
    let ctx = Ctx::setup();
    ctx.create_escrow();

    ctx.env.advance_time(Duration::seconds(LOCK_DURATION + 1));
    ctx.env.with_mock_all_auths(|| ctx.client().claim());

    assert_eq!(ctx.token.balance(&ctx.recipient), AMOUNT);
    assert_eq!(ctx.client().get_state().status, EscrowStatus::Claimed);
}

#[test]
fn test_claim_before_timeout_and_without_approval_reverts() {
    let ctx = Ctx::setup();
    ctx.create_escrow();

    ctx.env.mock_all_auths();
    assert_reverts!(ctx.client().claim(), "time lock has not expired");
}

#[test]
fn test_double_claim_reverts() {
    let ctx = Ctx::setup();
    ctx.create_escrow();

    ctx.env.advance_time(Duration::seconds(LOCK_DURATION + 1));
    ctx.env.with_mock_all_auths(|| ctx.client().claim());

    assert_reverts!(ctx.client().claim(), "already settled");
}

// ---------------------------------------------------------------------------
// Refund
// ---------------------------------------------------------------------------

#[test]
fn test_refund_after_timeout_without_claim() {
    let ctx = Ctx::setup();
    ctx.create_escrow();

    ctx.env.advance_time(Duration::seconds(LOCK_DURATION + 1));
    ctx.env.with_mock_all_auths(|| ctx.client().refund());

    assert_eq!(ctx.token.balance(&ctx.depositor), AMOUNT * 3);
    assert_eq!(ctx.client().get_state().status, EscrowStatus::Refunded);
}

#[test]
fn test_refund_before_timeout_reverts() {
    let ctx = Ctx::setup();
    ctx.create_escrow();

    ctx.env.mock_all_auths();
    assert_reverts!(ctx.client().refund(), "time lock has not expired");
}

// ---------------------------------------------------------------------------
// simulate_failing_call integration
// ---------------------------------------------------------------------------

#[test]
fn test_simulate_failing_call_captures_revert() {
    let ctx = Ctx::setup();
    ctx.create_escrow();

    // Claim before unlock with no approval — should fail.
    let result = ctx.env.simulate_failing_call(|| ctx.client().claim());

    assert!(result.did_fail(), "expected the call to revert");
    let msg = result.panic_message().unwrap_or("");
    assert!(
        msg.contains("time lock") || msg.is_empty(),
        "unexpected panic message: {msg}"
    );
}

#[test]
fn test_simulate_failing_call_succeeds_when_call_passes() {
    let ctx = Ctx::setup();
    ctx.create_escrow();

    // Advance past lock — claim should now succeed (did_fail == false).
    ctx.env.advance_time(Duration::seconds(LOCK_DURATION + 1));
    let result = ctx.env.simulate_failing_call(|| ctx.client().claim());
    assert!(!result.did_fail(), "expected the call to succeed");
}
