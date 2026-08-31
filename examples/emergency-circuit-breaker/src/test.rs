#![cfg(test)]
extern crate std;

use crucible::prelude::*;
use crucible::assert_emitted;
use soroban_sdk::{symbol_short, Vec};

use crate::{BreakerError, CircuitBreaker, CircuitBreakerClient};

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

/// TVL used throughout: 1_000_000, so the default 20% limit is 200_000.
const TVL: i128 = 1_000_000;
const LIMIT: i128 = 200_000;

struct Ctx {
    pub env: MockEnv,
    pub id: soroban_sdk::Address,
    pub admin: AccountHandle,
    pub g1: AccountHandle,
    pub g2: AccountHandle,
    pub g3: AccountHandle,
    pub user: AccountHandle,
}

impl Ctx {
    /// Three guardians, two approvals needed to recover, default 20% trip.
    fn setup() -> Self {
        Self::setup_with(2, 0)
    }

    fn setup_with(threshold: u32, threshold_bps: u32) -> Self {
        let env = MockEnv::builder()
            .with_contract::<CircuitBreaker>()
            .with_account("admin", Stroops::xlm(100))
            .with_account("g1", Stroops::xlm(100))
            .with_account("g2", Stroops::xlm(100))
            .with_account("g3", Stroops::xlm(100))
            .with_account("user", Stroops::xlm(100))
            .build();

        let id = env.contract_id::<CircuitBreaker>();
        let admin = env.account("admin");
        let g1 = env.account("g1");
        let g2 = env.account("g2");
        let g3 = env.account("g3");
        let user = env.account("user");

        let mut guardians: Vec<soroban_sdk::Address> = Vec::new(env.inner());
        guardians.push_back(g1.address());
        guardians.push_back(g2.address());
        guardians.push_back(g3.address());

        env.with_mock_all_auths(|| {
            let client = CircuitBreakerClient::new(env.inner(), &id);
            client.initialize(&admin, &guardians, &threshold, &threshold_bps);
            client.report_tvl(&TVL);
        });

        Ctx {
            env,
            id,
            admin,
            g1,
            g2,
            g3,
            user,
        }
    }

    fn client(&self) -> CircuitBreakerClient<'_> {
        CircuitBreakerClient::new(self.env.inner(), &self.id)
    }
}

// ---------------------------------------------------------------------------
// Configuration
// ---------------------------------------------------------------------------

#[test]
fn test_default_threshold_is_twenty_percent_of_tvl() {
    let ctx = Ctx::setup();
    assert_eq!(ctx.client().trip_threshold(), LIMIT);
}

#[test]
fn test_explicit_threshold_bps_is_honoured() {
    let ctx = Ctx::setup_with(2, 500); // 5%
    assert_eq!(ctx.client().trip_threshold(), TVL * 5 / 100);
}

#[test]
fn test_threshold_of_hundred_percent_is_rejected() {
    // A breaker that can never trip would leave the protocol believing it is
    // protected when it is not.
    let env = MockEnv::builder()
        .with_contract::<CircuitBreaker>()
        .with_account("admin", Stroops::xlm(100))
        .with_account("g1", Stroops::xlm(100))
        .build();
    let id = env.contract_id::<CircuitBreaker>();
    let admin = env.account("admin");
    let mut guardians: Vec<soroban_sdk::Address> = Vec::new(env.inner());
    guardians.push_back(env.account("g1").address());

    env.mock_all_auths();
    let result = CircuitBreakerClient::new(env.inner(), &id)
        .try_initialize(&admin, &guardians, &1, &10_000);
    assert_eq!(result, Err(Ok(BreakerError::InvalidConfig)));
}

#[test]
fn test_recovery_threshold_above_guardian_count_is_rejected() {
    let env = MockEnv::builder()
        .with_contract::<CircuitBreaker>()
        .with_account("admin", Stroops::xlm(100))
        .with_account("g1", Stroops::xlm(100))
        .build();
    let id = env.contract_id::<CircuitBreaker>();
    let admin = env.account("admin");
    let mut guardians: Vec<soroban_sdk::Address> = Vec::new(env.inner());
    guardians.push_back(env.account("g1").address());

    env.mock_all_auths();
    let result = CircuitBreakerClient::new(env.inner(), &id)
        .try_initialize(&admin, &guardians, &2, &0);
    assert_eq!(result, Err(Ok(BreakerError::InvalidConfig)));
}

// ---------------------------------------------------------------------------
// Rolling window
// ---------------------------------------------------------------------------

#[test]
fn test_outflow_below_threshold_does_not_trip() {
    let ctx = Ctx::setup();
    ctx.env.mock_all_auths();

    assert!(!ctx.client().record_outflow(&ctx.admin, &(LIMIT - 1)));
    assert!(!ctx.client().is_tripped());
    assert_eq!(ctx.client().window_outflow(), LIMIT - 1);
}

#[test]
fn test_outflow_exactly_at_threshold_does_not_trip() {
    // The breaker trips on *exceeding* the limit, so a protocol configured to
    // pay out exactly 20% can still do so.
    let ctx = Ctx::setup();
    ctx.env.mock_all_auths();

    assert!(!ctx.client().record_outflow(&ctx.admin, &LIMIT));
    assert!(!ctx.client().is_tripped());
}

#[test]
fn test_outflow_above_threshold_trips() {
    let ctx = Ctx::setup();
    ctx.env.mock_all_auths();

    assert!(ctx.client().record_outflow(&ctx.admin, &(LIMIT + 1)));
    // Asserted before any further call: the event buffer reflects the most
    // recent invocation.
    assert_emitted!(
        ctx.env,
        ctx.id,
        (symbol_short!("tripped"),),
        (LIMIT + 1, LIMIT, ctx.env.timestamp())
    );
    assert!(ctx.client().is_tripped());
}

#[test]
fn test_drain_split_across_transfers_still_trips() {
    // The point of a rolling window: an attacker cannot stay under the limit
    // by splitting the drain into pieces.
    let ctx = Ctx::setup();
    ctx.env.mock_all_auths();

    for _ in 0..4 {
        assert!(!ctx.client().record_outflow(&ctx.admin, &50_000));
    }
    assert!(!ctx.client().is_tripped());

    assert!(ctx.client().record_outflow(&ctx.admin, &1));
    assert!(ctx.client().is_tripped());
}

#[test]
fn test_outflow_spread_beyond_the_window_does_not_trip() {
    // A protocol legitimately paying out the same total across a day must not
    // be penalised — only the last hour counts.
    let ctx = Ctx::setup();
    ctx.env.mock_all_auths();

    for _ in 0..6 {
        ctx.client().record_outflow(&ctx.admin, &100_000);
        ctx.env.advance_time(Duration::hours(2));
    }

    assert!(!ctx.client().is_tripped());
    assert_eq!(ctx.client().window_outflow(), 0);
}

#[test]
fn test_window_drops_outflow_older_than_one_hour() {
    let ctx = Ctx::setup();
    ctx.env.mock_all_auths();

    ctx.client().record_outflow(&ctx.admin, &150_000);
    assert_eq!(ctx.client().window_outflow(), 150_000);

    ctx.env.advance_time(Duration::hours(1));
    ctx.env.advance_time(Duration::minutes(5));

    assert_eq!(ctx.client().window_outflow(), 0);
    assert!(!ctx.client().record_outflow(&ctx.admin, &150_000));
}

#[test]
fn test_outflow_is_recorded_even_when_it_trips() {
    // The amount that crossed the line is still money that left; dropping it
    // would let the next call start from a clean slate.
    let ctx = Ctx::setup();
    ctx.env.mock_all_auths();

    ctx.client().record_outflow(&ctx.admin, &(LIMIT + 5_000));
    assert_eq!(ctx.client().window_outflow(), LIMIT + 5_000);
}

#[test]
fn test_no_reported_tvl_means_no_trip() {
    // With no TVL the ratio is meaningless; the breaker stays out of the way
    // rather than tripping on the first withdrawal.
    let env = MockEnv::builder()
        .with_contract::<CircuitBreaker>()
        .with_account("admin", Stroops::xlm(100))
        .with_account("g1", Stroops::xlm(100))
        .build();
    let id = env.contract_id::<CircuitBreaker>();
    let admin = env.account("admin");
    let mut guardians: Vec<soroban_sdk::Address> = Vec::new(env.inner());
    guardians.push_back(env.account("g1").address());

    env.mock_all_auths();
    let client = CircuitBreakerClient::new(env.inner(), &id);
    client.initialize(&admin, &guardians, &1, &0);

    assert!(!client.record_outflow(&admin, &1_000_000_000));
    assert!(!client.is_tripped());
}

#[test]
fn test_zero_and_negative_outflow_rejected() {
    let ctx = Ctx::setup();
    ctx.env.mock_all_auths();

    assert_eq!(
        ctx.client().try_record_outflow(&ctx.admin, &0),
        Err(Ok(BreakerError::InvalidAmount))
    );
    assert_eq!(
        ctx.client().try_record_outflow(&ctx.admin, &-1),
        Err(Ok(BreakerError::InvalidAmount))
    );
}

// ---------------------------------------------------------------------------
// Pause behaviour
// ---------------------------------------------------------------------------

#[test]
fn test_protected_action_reverts_while_tripped() {
    let ctx = Ctx::setup();
    ctx.env.mock_all_auths();

    ctx.client().protected_action(&ctx.user);
    ctx.client().record_outflow(&ctx.admin, &(LIMIT + 1));

    assert_eq!(
        ctx.client().try_protected_action(&ctx.user),
        Err(Ok(BreakerError::Tripped))
    );
}

#[test]
fn test_further_outflow_reverts_while_tripped() {
    let ctx = Ctx::setup();
    ctx.env.mock_all_auths();

    ctx.client().record_outflow(&ctx.admin, &(LIMIT + 1));
    assert_eq!(
        ctx.client().try_record_outflow(&ctx.admin, &1),
        Err(Ok(BreakerError::Tripped))
    );
}

#[test]
fn test_guardian_can_trip_manually() {
    let ctx = Ctx::setup();
    ctx.env.mock_all_auths();

    ctx.client().trip(&ctx.g1);
    assert!(ctx.client().is_tripped());
}

#[test]
fn test_stranger_cannot_trip() {
    let ctx = Ctx::setup();
    ctx.env.mock_all_auths();

    assert_eq!(
        ctx.client().try_trip(&ctx.user),
        Err(Ok(BreakerError::Unauthorized))
    );
}

// ---------------------------------------------------------------------------
// Multi-sig recovery
// ---------------------------------------------------------------------------

#[test]
fn test_single_approval_does_not_reopen() {
    let ctx = Ctx::setup();
    ctx.env.mock_all_auths();
    ctx.client().trip(&ctx.g1);

    assert!(!ctx.client().approve_recovery(&ctx.g1));
    assert!(ctx.client().is_tripped());
}

#[test]
fn test_threshold_approvals_reopen_the_breaker() {
    let ctx = Ctx::setup();
    ctx.env.mock_all_auths();
    ctx.client().trip(&ctx.g1);

    ctx.client().approve_recovery(&ctx.g1);
    assert!(ctx.client().approve_recovery(&ctx.g2));
    assert_emitted!(ctx.env, ctx.id, (symbol_short!("resumed"),), 2u32);

    assert!(!ctx.client().is_tripped());
    ctx.client().protected_action(&ctx.user);
}

#[test]
fn test_guardian_cannot_approve_twice() {
    let ctx = Ctx::setup();
    ctx.env.mock_all_auths();
    ctx.client().trip(&ctx.g1);

    ctx.client().approve_recovery(&ctx.g1);
    assert_eq!(
        ctx.client().try_approve_recovery(&ctx.g1),
        Err(Ok(BreakerError::AlreadyApproved))
    );
    assert!(ctx.client().is_tripped());
}

#[test]
fn test_non_guardian_cannot_approve() {
    let ctx = Ctx::setup();
    ctx.env.mock_all_auths();
    ctx.client().trip(&ctx.g1);

    assert_eq!(
        ctx.client().try_approve_recovery(&ctx.user),
        Err(Ok(BreakerError::Unauthorized))
    );
}

#[test]
fn test_admin_alone_cannot_reopen() {
    // Recovery is deliberately not an admin power: a compromised admin key is
    // one of the scenarios the breaker exists to contain.
    let ctx = Ctx::setup();
    ctx.env.mock_all_auths();
    ctx.client().trip(&ctx.g1);

    assert_eq!(
        ctx.client().try_approve_recovery(&ctx.admin),
        Err(Ok(BreakerError::Unauthorized))
    );
    assert!(ctx.client().is_tripped());
}

#[test]
fn test_approving_when_not_tripped_reverts() {
    let ctx = Ctx::setup();
    ctx.env.mock_all_auths();

    assert_eq!(
        ctx.client().try_approve_recovery(&ctx.g1),
        Err(Ok(BreakerError::NotTripped))
    );
}

#[test]
fn test_approvals_do_not_carry_into_a_later_incident() {
    // Stale approvals must never count towards a new incident: one guardian
    // approving twice, across two trips, would otherwise reopen a 2-of-3
    // breaker on its own.
    let ctx = Ctx::setup();
    ctx.env.mock_all_auths();

    ctx.client().trip(&ctx.g1);
    ctx.client().approve_recovery(&ctx.g1);
    ctx.client().approve_recovery(&ctx.g2);
    assert!(!ctx.client().is_tripped());

    ctx.client().trip(&ctx.g1);
    assert_eq!(ctx.client().recovery_approvals().len(), 0);

    assert!(!ctx.client().approve_recovery(&ctx.g1));
    assert!(ctx.client().is_tripped());
    assert!(ctx.client().approve_recovery(&ctx.g3));
    assert!(!ctx.client().is_tripped());
}

#[test]
fn test_recovery_clears_the_window() {
    // Otherwise the outflow that caused the trip would immediately re-trip the
    // breaker on the next withdrawal, and recovery could never complete.
    let ctx = Ctx::setup();
    ctx.env.mock_all_auths();

    ctx.client().record_outflow(&ctx.admin, &(LIMIT + 1));
    ctx.client().approve_recovery(&ctx.g1);
    ctx.client().approve_recovery(&ctx.g2);

    assert_eq!(ctx.client().window_outflow(), 0);
    assert!(!ctx.client().record_outflow(&ctx.admin, &1_000));
    assert!(!ctx.client().is_tripped());
}
