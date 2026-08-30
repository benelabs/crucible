//! Fixture dependency graph resolution tests.
//!
//! Compile-time rejection of circular graphs is covered by the trybuild UI
//! tests in `crucible-macros/tests/ui`. These tests cover the runtime half:
//! that a declared graph is resolved, in order, exactly once per fixture, and
//! that the shape of the graph is visible through the generated constants.

use crucible::prelude::*;
use std::cell::RefCell;

thread_local! {
    /// Records the order in which fixtures were constructed.
    static SETUP_LOG: RefCell<Vec<&'static str>> = const { RefCell::new(Vec::new()) };
}

fn record(name: &'static str) {
    SETUP_LOG.with(|log| log.borrow_mut().push(name));
}

fn take_log() -> Vec<&'static str> {
    SETUP_LOG.with(|log| std::mem::take(&mut *log.borrow_mut()))
}

// ── A diamond: Dex and Staking both build on Token; Protocol builds on both ──

#[fixture]
pub struct TokenFixture {
    pub env: MockEnv,
}

impl TokenFixture {
    pub fn setup() -> Self {
        record("token");
        Self {
            env: MockEnv::default(),
        }
    }
}

#[fixture]
pub struct OracleFixture {
    pub env: MockEnv,
    pub price: i128,
}

impl OracleFixture {
    pub fn setup() -> Self {
        record("oracle");
        Self {
            env: MockEnv::default(),
            price: 42,
        }
    }
}

#[fixture(requires = [TokenFixture, OracleFixture])]
pub struct DexFixture {
    pub token: TokenFixture,
    pub oracle: OracleFixture,
}

impl DexFixture {
    pub fn setup() -> Self {
        let (token, oracle) = Self::setup_deps();
        record("dex");
        Self { token, oracle }
    }
}

#[fixture(requires = [TokenFixture])]
pub struct StakingFixture {
    pub token: TokenFixture,
}

impl StakingFixture {
    pub fn setup() -> Self {
        let (token,) = Self::setup_deps();
        record("staking");
        Self { token }
    }
}

#[fixture(requires = [DexFixture, StakingFixture])]
pub struct ProtocolFixture {
    pub dex: DexFixture,
    pub staking: StakingFixture,
}

impl ProtocolFixture {
    pub fn setup() -> Self {
        let (dex, staking) = Self::setup_deps();
        record("protocol");
        Self { dex, staking }
    }
}

// ── Tests ───────────────────────────────────────────────────────────────────

#[test]
fn a_fixture_without_dependencies_reports_an_empty_graph() {
    let _ = take_log();

    assert_eq!(TokenFixture::DEPENDENCY_COUNT, 0);
    assert!(<TokenFixture as FixtureDeps>::DEPENDENCY_NAMES.is_empty());

    let fixture = TokenFixture::setup();
    let _ = &fixture.env;
    assert_eq!(take_log(), ["token"]);
}

#[test]
fn dependencies_are_reported_in_declaration_order() {
    assert_eq!(DexFixture::DEPENDENCY_COUNT, 2);
    assert_eq!(
        <DexFixture as FixtureDeps>::DEPENDENCY_NAMES,
        ["TokenFixture", "OracleFixture"]
    );

    assert_eq!(StakingFixture::DEPENDENCY_COUNT, 1);
    assert_eq!(
        <StakingFixture as FixtureDeps>::DEPENDENCY_NAMES,
        ["TokenFixture"]
    );
}

#[test]
fn setup_deps_builds_dependencies_before_the_dependent() {
    let _ = take_log();

    let dex = DexFixture::setup();

    assert_eq!(
        take_log(),
        ["token", "oracle", "dex"],
        "dependencies must be constructed before the fixture that requires them"
    );
    assert_eq!(dex.oracle.price, 42);
}

#[test]
fn a_deeper_graph_resolves_bottom_up() {
    let _ = take_log();

    let protocol = ProtocolFixture::setup();

    // Each branch of the diamond is resolved fully before the next begins, and
    // the root is constructed last.
    assert_eq!(
        take_log(),
        ["token", "oracle", "dex", "token", "staking", "protocol"],
        "a deeper graph must resolve depth-first, in declaration order"
    );

    assert_eq!(protocol.dex.oracle.price, 42);
    assert_eq!(ProtocolFixture::DEPENDENCY_COUNT, 2);
    assert_eq!(
        <ProtocolFixture as FixtureDeps>::DEPENDENCY_NAMES,
        ["DexFixture", "StakingFixture"]
    );
}

#[test]
fn a_shared_dependency_is_built_once_per_dependent() {
    let _ = take_log();

    let _ = ProtocolFixture::setup();
    let log = take_log();

    // `TokenFixture` sits under both branches of the diamond, so each branch
    // gets its own instance. Fixtures are values, not a shared registry, and
    // isolating them is what keeps one test from observing another's state.
    assert_eq!(
        log.iter().filter(|name| **name == "token").count(),
        2,
        "each dependent receives its own instance of a shared dependency"
    );
}

#[test]
fn reset_rebuilds_the_whole_graph() {
    let _ = take_log();

    let mut dex = DexFixture::setup();
    let _ = take_log();

    dex.reset();

    assert_eq!(
        take_log(),
        ["token", "oracle", "dex"],
        "reset must rebuild the dependency graph, not just the fixture"
    );
}

#[test]
fn setup_deps_returns_usable_fixtures() {
    let _ = take_log();

    let (token, oracle) = DexFixture::setup_deps();

    // The tuple is ordered as declared, and each element is a fully built
    // fixture rather than a placeholder.
    let _ = &token.env;
    assert_eq!(oracle.price, 42);
    assert_eq!(take_log(), ["token", "oracle"]);
}

#[test]
fn fixtures_in_a_graph_remain_independently_constructible() {
    let _ = take_log();

    // A fixture that others depend on is still usable on its own.
    let oracle = OracleFixture::setup();
    assert_eq!(oracle.price, 42);
    assert_eq!(take_log(), ["oracle"]);
}

#[test]
fn generated_fixtures_still_derive_debug() {
    let token = TokenFixture::setup();
    let rendered = format!("{token:?}");
    assert!(
        rendered.contains("TokenFixture"),
        "the macro must still derive Debug, was: {rendered}"
    );
}
