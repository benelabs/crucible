#![cfg(test)]
extern crate std;

use crucible::prelude::*;
use crucible::{assert_emitted, assert_reverts};
use soroban_sdk::{symbol_short, Address};

use crate::{Escrow, EscrowClient, EscrowStatus};

const AMOUNT: i128 = 1_000_000; // 1 USDC (6 decimals)
const BASE_TIME: u64 = 1_000_000;
const LOCK_DURATION: u64 = 86_400; // 1 day in seconds

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

struct Ctx {
    pub env: MockEnv,
    pub id: Address,
    pub depositor: AccountHandle,
    pub recipient: AccountHandle,
    pub arbiter: AccountHandle,
    pub token: MockToken,
}

impl Ctx {
    fn setup() -> Self {
        let env = MockEnv::builder()
            .at_timestamp(BASE_TIME)
            .with_contract::<Escrow>()
            .with_account("depositor", Stroops::xlm(100))
            .with_account("recipient", Stroops::xlm(10))
            .with_account("arbiter", Stroops::xlm(10))
            .build();

        let id = env.contract_id::<Escrow>();
        let depositor = env.account("depositor");
        let recipient = env.account("recipient");
        let arbiter = env.account("arbiter");

        let token = MockToken::new(&env, "USDC", 6);
        token.mint(&depositor, AMOUNT * 2); // fund depositor generously

        Ctx {
            env,
            id,
            depositor,
            recipient,
            arbiter,
            token,
        }
    }

    fn client(&self) -> EscrowClient<'_> {
        EscrowClient::new(self.env.inner(), &self.id)
    }

    /// Convenience: create a live escrow with unlock_time = BASE_TIME + LOCK_DURATION.
    fn create_escrow(&self) {
        self.env.with_mock_all_auths(|| {
            self.client().create(
                &self.depositor,
                &self.recipient,
                &self.arbiter,
                &self.token.address(),
                &AMOUNT,
                &(BASE_TIME + LOCK_DURATION),
            );
        });
    }
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[test]
fn test_create_transfers_tokens_to_contract() {
    let ctx = Ctx::setup();
    ctx.create_escrow();

    // Contract should hold the escrowed amount.
    assert_eq!(ctx.token.balance(&ctx.id), AMOUNT);
    // Depositor balance reduced.
    assert_eq!(ctx.token.balance(&ctx.depositor), AMOUNT); // started with AMOUNT*2
}

#[test]
fn test_claim_after_timeout() {
    let ctx = Ctx::setup();
    ctx.create_escrow();

    // Advance past the time lock.
    ctx.env.advance_time(Duration::seconds(LOCK_DURATION + 1));
    ctx.env.with_mock_all_auths(|| ctx.client().claim());

    assert_eq!(ctx.token.balance(&ctx.recipient), AMOUNT);
    assert_eq!(ctx.token.balance(&ctx.id), 0);
    assert_eq!(ctx.client().get_state().status, EscrowStatus::Claimed);
}

#[test]
fn test_claim_before_timeout_reverts() {
    let ctx = Ctx::setup();
    ctx.create_escrow();

    // Do NOT advance time — time lock still active.
    ctx.env.mock_all_auths();
    assert_reverts!(ctx.client().claim(), "time lock");
}

#[test]
fn test_arbiter_approve_allows_early_claim() {
    let ctx = Ctx::setup();
    ctx.create_escrow();

    // Arbiter approves without waiting for the time lock.
    ctx.env.with_mock_all_auths(|| ctx.client().approve(&ctx.arbiter));

    // Recipient claims immediately.
    ctx.client().claim();

    assert_eq!(ctx.token.balance(&ctx.recipient), AMOUNT);
    assert_eq!(ctx.client().get_state().status, EscrowStatus::Claimed);
}

#[test]
fn test_only_arbiter_can_approve() {
    let ctx = Ctx::setup();
    ctx.create_escrow();

    ctx.env.mock_all_auths();
    // recipient tries to act as arbiter — logic check, not gated by auth
    assert_reverts!(
        ctx.client().approve(&ctx.recipient),
        "only the arbiter can approve"
    );
}

#[test]
fn test_refund_after_timeout() {
    let ctx = Ctx::setup();
    ctx.create_escrow();

    // Advance past the time lock.
    ctx.env.advance_time(Duration::seconds(LOCK_DURATION + 1));
    ctx.env.with_mock_all_auths(|| ctx.client().refund());

    // Depositor gets their tokens back.
    assert_eq!(ctx.token.balance(&ctx.depositor), AMOUNT * 2);
    assert_eq!(ctx.client().get_state().status, EscrowStatus::Refunded);
}

#[test]
fn test_refund_before_timeout_reverts() {
    let ctx = Ctx::setup();
    ctx.create_escrow();

    ctx.env.mock_all_auths();
    assert_reverts!(ctx.client().refund(), "time lock");
}

#[test]
fn test_double_claim_reverts() {
    let ctx = Ctx::setup();
    ctx.create_escrow();

    ctx.env.advance_time(Duration::seconds(LOCK_DURATION + 1));
    ctx.env.with_mock_all_auths(|| ctx.client().claim());

    // Second claim should revert.
    assert_reverts!(ctx.client().claim(), "already settled");
}

#[test]
fn test_create_emits_event() {
    let ctx = Ctx::setup();
    ctx.env.with_mock_all_auths(|| {
        ctx.client().create(
            &ctx.depositor,
            &ctx.recipient,
            &ctx.arbiter,
            &ctx.token.address(),
            &AMOUNT,
            &(BASE_TIME + LOCK_DURATION),
        );
    });
    assert_emitted!(ctx.env, ctx.id, (symbol_short!("created"),), AMOUNT);
}
