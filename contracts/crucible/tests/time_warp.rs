//! Time-warp and monotonic ledger clock tests.
//!
//! Covers the two shapes the issue calls out: an escrow whose claim window
//! expires, and a vesting schedule that releases over time. Both depend on the
//! ledger timestamp and sequence advancing together, and on storage entries
//! ageing the way they do on-chain.

use crucible::prelude::*;
use soroban_sdk::testutils::Address as _;
use soroban_sdk::{contract, contractimpl, contracttype, symbol_short, Address, Env};

#[contracttype]
#[derive(Clone)]
enum DataKey {
    Deadline,
    Claimed,
    Start,
    Cliff,
    Total,
    Duration,
}

/// An escrow that can only be claimed before its deadline.
#[contract]
struct ExpiringEscrow;

#[contractimpl]
impl ExpiringEscrow {
    pub fn init(env: Env, deadline: u64) {
        env.storage().instance().set(&DataKey::Deadline, &deadline);
        env.storage().instance().set(&DataKey::Claimed, &false);
    }

    pub fn claim(env: Env) {
        let deadline: u64 = env.storage().instance().get(&DataKey::Deadline).unwrap();
        assert!(
            env.ledger().timestamp() < deadline,
            "claim window has closed"
        );
        env.storage().instance().set(&DataKey::Claimed, &true);
    }

    pub fn claimed(env: Env) -> bool {
        env.storage()
            .instance()
            .get(&DataKey::Claimed)
            .unwrap_or(false)
    }

    /// A quote held in temporary storage, which lapses as ledgers pass.
    pub fn quote(env: Env, amount: i128) {
        env.storage().temporary().set(&symbol_short!("quote"), &amount);
    }

    pub fn read_quote(env: Env) -> Option<i128> {
        env.storage().temporary().get(&symbol_short!("quote"))
    }
}

/// A linear vesting schedule with a cliff.
#[contract]
struct Vesting;

#[contractimpl]
impl Vesting {
    pub fn init(env: Env, start: u64, cliff: u64, duration: u64, total: i128) {
        env.storage().instance().set(&DataKey::Start, &start);
        env.storage().instance().set(&DataKey::Cliff, &cliff);
        env.storage().instance().set(&DataKey::Duration, &duration);
        env.storage().instance().set(&DataKey::Total, &total);
    }

    pub fn vested(env: Env) -> i128 {
        let start: u64 = env.storage().instance().get(&DataKey::Start).unwrap();
        let cliff: u64 = env.storage().instance().get(&DataKey::Cliff).unwrap();
        let duration: u64 = env.storage().instance().get(&DataKey::Duration).unwrap();
        let total: i128 = env.storage().instance().get(&DataKey::Total).unwrap();

        let now = env.ledger().timestamp();
        if now < start + cliff {
            return 0;
        }
        let elapsed = now - start;
        if elapsed >= duration {
            return total;
        }
        total * (elapsed as i128) / (duration as i128)
    }
}

// ── advance_ledger moves time and sequence together ─────────────────────────

#[test]
fn advance_ledger_moves_timestamp_and_sequence_in_step() {
    let env = MockEnv::builder()
        .at_timestamp(1_000)
        .at_sequence(10)
        .build();

    env.advance_ledger(100, 5);

    assert_eq!(env.ledger_sequence(), 110, "sequence advances by the ledgers");
    assert_eq!(
        env.timestamp(),
        1_000 + 100 * 5,
        "timestamp advances by ledgers times the close time"
    );
}

#[test]
fn advance_ledger_is_a_noop_for_zero_ledgers() {
    let env = MockEnv::builder()
        .at_timestamp(1_000)
        .at_sequence(10)
        .build();

    env.advance_ledger(0, 5);

    assert_eq!(env.ledger_sequence(), 10);
    assert_eq!(env.timestamp(), 1_000);
}

#[test]
fn advance_ledgers_uses_the_nominal_close_time() {
    let env = MockEnv::builder().at_timestamp(0).at_sequence(0).build();

    env.advance_ledgers(12);

    assert_eq!(env.ledger_sequence(), 12);
    assert_eq!(env.timestamp(), 12 * DEFAULT_SECONDS_PER_LEDGER);
}

#[test]
fn successive_advances_accumulate_monotonically() {
    let env = MockEnv::builder().at_timestamp(0).at_sequence(0).build();

    let mut last_ts = env.timestamp();
    let mut last_seq = env.ledger_sequence();
    for _ in 0..5 {
        env.advance_ledger(7, 5);
        assert!(env.timestamp() > last_ts, "the clock must never go backwards");
        assert!(
            env.ledger_sequence() > last_seq,
            "the sequence must never go backwards"
        );
        last_ts = env.timestamp();
        last_seq = env.ledger_sequence();
    }

    assert_eq!(env.ledger_sequence(), 35);
    assert_eq!(env.timestamp(), 175);
}

#[test]
#[should_panic(expected = "timestamp overflow in advance_ledger")]
fn advance_ledger_panics_on_timestamp_overflow() {
    let env = MockEnv::builder().at_timestamp(u64::MAX - 1).build();
    env.advance_ledger(1, 10);
}

#[test]
#[should_panic(expected = "sequence number overflow in advance_ledger")]
fn advance_ledger_panics_on_sequence_overflow() {
    let env = MockEnv::builder().at_sequence(u32::MAX - 1).build();
    env.advance_ledger(2, 1);
}

#[test]
#[should_panic(expected = "elapsed seconds overflow in advance_ledger")]
fn advance_ledger_panics_when_elapsed_seconds_overflow() {
    let env = MockEnv::default();
    env.advance_ledger(u32::MAX, u64::MAX);
}

// ── advance_ledger_time ─────────────────────────────────────────────────────

#[test]
fn advance_ledger_time_derives_the_sequence_from_the_duration() {
    let env = MockEnv::builder().at_timestamp(0).at_sequence(0).build();

    env.advance_ledger_time(Duration::hours(1), 5);

    assert_eq!(env.ledger_sequence(), 720, "3600 seconds at 5s per ledger");
    assert_eq!(env.timestamp(), 3_600);
}

#[test]
fn advance_ledger_time_rounds_up_so_the_clock_never_falls_short() {
    let env = MockEnv::builder().at_timestamp(0).at_sequence(0).build();

    // 7 seconds does not divide evenly into 5-second ledgers.
    env.advance_ledger_time(Duration::seconds(7), 5);

    assert_eq!(env.ledger_sequence(), 2, "rounded up from 1.4 ledgers");
    assert!(
        env.timestamp() >= 7,
        "the clock must reach at least the requested duration"
    );
    assert_eq!(env.timestamp(), 10);
}

#[test]
fn advance_ledger_time_is_a_noop_for_zero_duration() {
    let env = MockEnv::builder().at_timestamp(500).at_sequence(3).build();

    env.advance_ledger_time(Duration::seconds(0), 5);

    assert_eq!(env.timestamp(), 500);
    assert_eq!(env.ledger_sequence(), 3);
}

#[test]
#[should_panic(expected = "seconds_per_ledger must be greater than zero")]
fn advance_ledger_time_rejects_a_zero_close_time() {
    let env = MockEnv::default();
    env.advance_ledger_time(Duration::seconds(10), 0);
}

// ── TTL settings ────────────────────────────────────────────────────────────

#[test]
fn entry_ttl_settings_round_trip() {
    let env = MockEnv::default();

    let defaults = env.entry_ttl_settings();
    assert!(defaults.min_temp > 0);
    assert!(defaults.min_persistent > 0);
    assert!(defaults.max >= defaults.min_persistent);

    let tightened = EntryTtlSettings {
        min_temp: 4,
        min_persistent: 8,
        max: 1_000,
    };
    env.set_entry_ttl_settings(tightened);

    assert_eq!(env.entry_ttl_settings(), tightened);
}

#[test]
fn setting_ttl_settings_leaves_the_clock_untouched() {
    let env = MockEnv::builder().at_timestamp(900).at_sequence(4).build();

    env.set_entry_ttl_settings(EntryTtlSettings {
        min_temp: 2,
        min_persistent: 3,
        max: 100,
    });

    assert_eq!(env.timestamp(), 900);
    assert_eq!(env.ledger_sequence(), 4);
}

// ── Storage ageing ──────────────────────────────────────────────────────────

#[test]
fn temporary_storage_lapses_as_ledgers_pass_while_persistent_survives() {
    let env = MockEnv::default();
    let inner = env.inner();
    let id = inner.register(ExpiringEscrow, ());
    let client = ExpiringEscrowClient::new(inner, &id);

    client.init(&u64::MAX);
    client.quote(&500);
    assert_eq!(
        client.read_quote(),
        Some(500),
        "a fresh temporary entry is readable"
    );

    // Past the default 16-ledger temporary lifetime.
    env.advance_ledger(32, 5);

    assert_eq!(
        client.read_quote(),
        None,
        "the temporary quote must lapse once its lifetime elapses"
    );
    assert!(
        !client.claimed(),
        "instance storage survives the same advance"
    );
}

#[test]
fn a_short_advance_leaves_temporary_storage_intact() {
    let env = MockEnv::default();
    let inner = env.inner();
    let id = inner.register(ExpiringEscrow, ());
    let client = ExpiringEscrowClient::new(inner, &id);

    client.init(&u64::MAX);
    client.quote(&500);

    // Well inside the default temporary lifetime.
    env.advance_ledger(4, 5);

    assert_eq!(
        client.read_quote(),
        Some(500),
        "an entry within its lifetime must not be collected"
    );
}

#[test]
fn advancing_only_time_does_not_age_storage() {
    let env = MockEnv::default();
    let inner = env.inner();
    let id = inner.register(ExpiringEscrow, ());
    let client = ExpiringEscrowClient::new(inner, &id);

    client.init(&u64::MAX);
    client.quote(&500);

    // `advance_time` leaves the sequence behind, so entries never age — the
    // inconsistency `advance_ledger` exists to avoid.
    env.advance_time(Duration::days(30));

    assert_eq!(env.ledger_sequence(), 0);
    assert_eq!(
        client.read_quote(),
        Some(500),
        "storage cannot age while the sequence stands still"
    );
}

// ── Expiring escrow ─────────────────────────────────────────────────────────

#[test]
fn escrow_can_be_claimed_before_its_deadline() {
    let env = MockEnv::builder().at_timestamp(1_000).build();
    let inner = env.inner();
    let id = inner.register(ExpiringEscrow, ());
    let client = ExpiringEscrowClient::new(inner, &id);

    client.init(&2_000);

    env.advance_ledger(100, 5); // to t = 1_500
    client.claim();

    assert!(client.claimed());
}

#[test]
fn escrow_claim_reverts_once_the_window_closes() {
    let env = MockEnv::builder().at_timestamp(1_000).build();
    let inner = env.inner();
    let id = inner.register(ExpiringEscrow, ());
    let client = ExpiringEscrowClient::new(inner, &id);

    client.init(&2_000);

    env.advance_ledger(400, 5); // to t = 3_000, past the deadline
    assert!(
        client.try_claim().is_err(),
        "claiming after the deadline must revert"
    );
    assert!(!client.claimed());
}

#[test]
fn escrow_deadline_is_reached_with_a_consistent_sequence() {
    let env = MockEnv::builder().at_timestamp(0).at_sequence(0).build();
    let inner = env.inner();
    let id = inner.register(ExpiringEscrow, ());
    let client = ExpiringEscrowClient::new(inner, &id);

    let deadline = Duration::days(7).as_seconds();
    client.init(&deadline);

    env.advance_ledger_time(Duration::days(7), 5);

    assert!(env.timestamp() >= deadline);
    assert_eq!(
        env.ledger_sequence(),
        (deadline / 5) as u32,
        "the sequence must reflect the ledgers that produced the elapsed time"
    );
    assert!(client.try_claim().is_err());
}

// ── Vesting schedule ────────────────────────────────────────────────────────

#[test]
fn vesting_releases_nothing_before_the_cliff() {
    let env = MockEnv::builder().at_timestamp(0).at_sequence(0).build();
    let inner = env.inner();
    let id = inner.register(Vesting, ());
    let client = VestingClient::new(inner, &id);

    let cliff = Duration::days(30).as_seconds();
    let duration = Duration::days(360).as_seconds();
    client.init(&0, &cliff, &duration, &360_000);

    env.advance_ledger_time(Duration::days(29), 5);
    assert_eq!(client.vested(), 0, "nothing vests before the cliff");
}

#[test]
fn vesting_accrues_linearly_after_the_cliff() {
    let env = MockEnv::builder().at_timestamp(0).at_sequence(0).build();
    let inner = env.inner();
    let id = inner.register(Vesting, ());
    let client = VestingClient::new(inner, &id);

    let cliff = Duration::days(30).as_seconds();
    let duration = Duration::days(360).as_seconds();
    client.init(&0, &cliff, &duration, &360_000);

    // Half the schedule: 180 of 360 days.
    env.advance_ledger_time(Duration::days(180), 5);
    let vested = client.vested();
    assert!(
        (179_000..=181_000).contains(&vested),
        "roughly half the grant must have vested, was {vested}"
    );

    // The whole schedule.
    env.advance_ledger_time(Duration::days(180), 5);
    assert_eq!(client.vested(), 360_000, "the full grant vests at the end");
}

#[test]
fn vesting_is_monotonic_across_repeated_advances() {
    let env = MockEnv::builder().at_timestamp(0).at_sequence(0).build();
    let inner = env.inner();
    let id = inner.register(Vesting, ());
    let client = VestingClient::new(inner, &id);

    let duration = Duration::days(100).as_seconds();
    client.init(&0, &0, &duration, &100_000);

    let mut previous = client.vested();
    for _ in 0..10 {
        env.advance_ledger_time(Duration::days(10), 5);
        let current = client.vested();
        assert!(
            current >= previous,
            "vested amount must never decrease: {previous} then {current}"
        );
        previous = current;
    }

    assert_eq!(previous, 100_000);
}

#[test]
fn vesting_caps_at_the_total_after_the_schedule_ends() {
    let env = MockEnv::builder().at_timestamp(0).at_sequence(0).build();
    let inner = env.inner();
    let id = inner.register(Vesting, ());
    let client = VestingClient::new(inner, &id);

    let duration = Duration::days(10).as_seconds();
    client.init(&0, &0, &duration, &50_000);

    env.advance_ledger_time(Duration::days(365), 5);

    assert_eq!(
        client.vested(),
        50_000,
        "vesting must not exceed the granted total"
    );
}

#[test]
fn a_generated_account_is_unaffected_by_the_clock() {
    // Advancing the ledger must not disturb unrelated environment state.
    let env = MockEnv::default();
    let alice = Address::generate(env.inner());

    env.advance_ledger(1_000, 5);

    assert_eq!(alice, alice.clone());
    assert_eq!(env.ledger_sequence(), 1_000);
}
