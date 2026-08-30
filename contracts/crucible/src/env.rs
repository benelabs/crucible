//! Mock environment for Soroban contract testing.
//!
//! Provides `MockEnv` - a wrapper around `soroban_sdk::Env` with convenient
//! helpers for testing, and `MockEnvBuilder` for fluent environment construction.
//!
//! **Host-only:** All types in this module depend on `std` and the Soroban host
//! test utilities. They are intended exclusively for use in `#[cfg(test)]`
//! contexts on the host and are not available inside contract WASM builds.

use crate::account::AccountHandle;
use crate::assertions::RevertAssertion;
use crate::checkpoint::{CheckpointId, CheckpointStack, CheckpointStats};
use crate::cost::CostReport;
use crate::sim::{PreparedTx, SimulatedTx};
use crate::token::MockToken;
use ed25519_dalek::{Signer as _, SigningKey as Ed25519SigningKey};
use k256::ecdsa::{SigningKey as K256SigningKey, VerifyingKey as K256VerifyingKey};
use p256::ecdsa::{SigningKey as P256SigningKey, VerifyingKey as P256VerifyingKey};
use rand::SeedableRng as _;
use rand_chacha::ChaCha8Rng;
use crate::zk::{
    self, G1, G2, Groth16Proof, Groth16VerifyingKey, PairingCurve, PlonkProof,
};
use soroban_sdk::{
    testutils::{ContractEvents, Events, Ledger, Register},
    Address, Env, FromVal, IntoVal, Val, Vec as SorobanVec,
};
use std::cell::RefCell;
use std::collections::HashMap;
use std::rc::Rc;
use std::time::Duration as StdDuration;

/// The ledger's storage time-to-live settings.
///
/// Soroban ties a storage entry's lifetime to the ledger sequence: a temporary
/// entry lives at least `min_temp` ledgers, a persistent one at least
/// `min_persistent`, and no entry may be extended beyond `max`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct EntryTtlSettings {
    /// Minimum lifetime, in ledgers, of a temporary storage entry.
    pub min_temp: u32,
    /// Minimum lifetime, in ledgers, of a persistent storage entry.
    pub min_persistent: u32,
    /// Maximum lifetime, in ledgers, any entry may be extended to.
    pub max: u32,
}

/// Nominal seconds between Stellar ledger closes.
///
/// Used as the default cadence by [`MockEnv::advance_ledgers`].
pub const DEFAULT_SECONDS_PER_LEDGER: u64 = 5;

/// A duration helper type for time-based test operations.
#[derive(Debug, Clone, Copy)]
pub struct Duration {
    seconds: u64,
}

impl Duration {
    /// Creates a duration from seconds.
    pub fn seconds(seconds: u64) -> Self {
        Self { seconds }
    }

    /// Creates a duration from minutes.
    pub fn minutes(minutes: u64) -> Self {
        Self {
            seconds: minutes * 60,
        }
    }

    /// Creates a duration from hours.
    pub fn hours(hours: u64) -> Self {
        Self {
            seconds: hours * 60 * 60,
        }
    }

    /// Creates a duration from days.
    pub fn days(days: u64) -> Self {
        Self {
            seconds: days * 24 * 60 * 60,
        }
    }

    /// Creates a duration from weeks.
    pub fn weeks(weeks: u64) -> Self {
        Self {
            seconds: weeks * 7 * 24 * 60 * 60,
        }
    }

    /// Returns the duration in seconds.
    pub fn as_seconds(&self) -> u64 {
        self.seconds
    }
}

impl From<StdDuration> for Duration {
    fn from(duration: StdDuration) -> Self {
        Self {
            seconds: duration.as_secs(),
        }
    }
}

/// A stroops helper type for XLM balance operations.
///
/// 1 XLM = 10,000,000 stroops
#[derive(Debug, Clone, Copy)]
pub struct Stroops {
    amount: i128,
}

impl Stroops {
    /// Creates stroops from a raw amount.
    ///
    /// # Panics
    /// Panics if the amount is negative, as negative balances are not supported.
    pub fn from(amount: i128) -> Self {
        assert!(amount >= 0, "Stroops amount cannot be negative: {}", amount);
        Self { amount }
    }

    /// Creates stroops from XLM (1 XLM = 10,000,000 stroops).
    ///
    /// # Panics
    /// Panics if the result would overflow or be negative.
    pub fn xlm(xlm: i128) -> Self {
        assert!(xlm >= 0, "XLM amount cannot be negative: {}", xlm);
        let amount = xlm
            .checked_mul(10_000_000)
            .expect("XLM amount overflowed when converting to stroops");
        Self { amount }
    }

    /// Creates stroops with fractional XLM from integer parts.
    ///
    /// # Arguments
    /// * `xlm` - Whole XLM units
    /// * `frac` - Fractional part in stroops (0 to 9,999,999)
    ///
    /// # Panics
    /// Panics if `xlm` is negative, `frac` is out of range, or the result overflows.
    pub fn from_parts(xlm: i128, frac: i128) -> Self {
        assert!(xlm >= 0, "XLM amount cannot be negative: {}", xlm);
        assert!(
            (0..10_000_000).contains(&frac),
            "Fractional stroops must be in range 0..10,000,000, got: {}",
            frac
        );
        let xlm_stroops = xlm
            .checked_mul(10_000_000)
            .expect("XLM amount overflowed when converting to stroops");
        let amount = xlm_stroops
            .checked_add(frac)
            .expect("Total stroops amount overflowed");
        Self { amount }
    }

    /// Creates stroops from a decimal string (e.g., "1.5", "0.0000001").
    ///
    /// This is the recommended way to construct Stroops from fractional XLM,
    /// as it avoids the precision loss of f64 conversion.
    ///
    /// # Arguments
    /// * `s` - Decimal string representing XLM amount
    ///
    /// # Panics
    /// Panics if the string is not a valid decimal, is negative, or overflows.
    pub fn from_xlm_str(s: &str) -> Self {
        let s = s.trim();
        assert!(!s.is_empty(), "XLM amount string cannot be empty");

        let (whole, frac_str) = if let Some((w, f)) = s.split_once('.') {
            (w, f)
        } else {
            (s, "")
        };

        let xlm: i128 = whole
            .parse()
            .expect(&format!("Invalid XLM amount: '{}'", s));
        assert!(xlm >= 0, "XLM amount cannot be negative: {}", s);

        let mut frac: i128 = 0;
        let mut divisor: i128 = 1;
        for c in frac_str.chars().take(7) {
            assert!(
                c.is_ascii_digit(),
                "Invalid character in fractional part: '{}'",
                s
            );
            frac = frac * 10 + (c as i128 - '0' as i128);
            divisor *= 10;
        }
        // Pad with zeros if fewer than 7 digits
        for _ in frac_str.len()..7 {
            frac *= 10;
            divisor *= 10;
        }
        // Ensure we have exactly 7 digits of precision
        assert!(
            frac_str.len() <= 7,
            "XLM amount has too many decimal places (max 7): '{}'",
            s
        );

        let xlm_stroops = xlm
            .checked_mul(10_000_000)
            .expect("XLM amount overflowed when converting to stroops");
        let frac_stroops = frac * 10_000_000 / divisor;
        let amount = xlm_stroops
            .checked_add(frac_stroops)
            .expect("Total stroops amount overflowed");

        Self { amount }
    }

    /// Creates stroops with fractional XLM (e.g., 0.5 XLM).
    ///
    /// # Deprecated
    /// This method uses f64 which can cause precision loss and silent truncation.
    /// Use `from_parts` or `from_xlm_str` instead.
    ///
    /// # Panics
    /// Panics if the result is negative or overflows.
    #[deprecated(
        since = "0.2.0",
        note = "Use `from_parts` or `from_xlm_str` to avoid lossy f64 conversion"
    )]
    pub fn xlm_frac(xlm: f64) -> Self {
        assert!(xlm >= 0.0, "XLM amount cannot be negative: {}", xlm);
        let amount = (xlm * 10_000_000.0).round() as i128;
        assert!(
            amount >= 0,
            "Converted stroops amount is negative, input may have been too small: {}",
            xlm
        );
        Self { amount }
    }

    /// Returns the amount in stroops.
    pub fn as_stroops(&self) -> i128 {
        self.amount
    }

    /// Returns the amount in XLM (as a float).
    pub fn as_xlm(&self) -> f64 {
        self.amount as f64 / 10_000_000.0
    }
}

/// Supported Soroban protocol versions for compatibility testing.
///
/// Crucible environments can be configured to target a specific protocol version
/// so that contracts can be tested against multiple Soroban network upgrades
/// (Protocol 20, 21, 22, …).
///
/// # Example
///
/// ```ignore
/// use crucible::prelude::*;
///
/// let env = MockEnv::builder()
///     .with_protocol_version(ProtocolVersion::V21)
///     .build();
/// assert_eq!(env.protocol_version(), 21);
/// ```
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum ProtocolVersion {
    V20 = 20,
    V21 = 21,
    V22 = 22,
}

impl ProtocolVersion {
    /// Returns the numeric protocol version.
    pub fn value(&self) -> u32 {
        *self as u32
    }

    /// Returns `true` if this protocol version supports the given host function
    /// name.  The list is intentionally conservative: if a function is not
    /// listed here we assume it is available in all supported versions.
    pub fn supports_host_function(&self, _function: &str) -> bool {
        let version = self.value();
        match _function {
            "v1_low_level_operations" | "v1_wasm_host_function_with_abi" => version >= 20,
            "v2_low_level_operations" | "v2_wasm_host_function_with_abi" => version >= 21,
            "v3_low_level_operations" | "v3_wasm_host_function_with_abi" => version >= 22,
            _ => true,
        }
    }

    /// Returns the maximum supported protocol version known to Crucible.
    pub fn max_supported() -> Self {
        ProtocolVersion::V22
    }

    /// Returns an iterator over all supported protocol versions.
    pub fn all() -> impl Iterator<Item = ProtocolVersion> {
        [ProtocolVersion::V20, ProtocolVersion::V21, ProtocolVersion::V22]
            .into_iter()
    }
}

impl std::fmt::Display for ProtocolVersion {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "Protocol {}", self.value())
    }
}

impl From<u32> for ProtocolVersion {
    fn from(value: u32) -> Self {
        match value {
            20 => ProtocolVersion::V20,
            21 => ProtocolVersion::V21,
            22 => ProtocolVersion::V22,
            _ => panic!(
                "Unsupported protocol version: {}. Supported versions are 20, 21, 22.",
                value
            ),
        }
    }
}

impl From<ProtocolVersion> for u32 {
    fn from(version: ProtocolVersion) -> Self {
        version.value()
    }
}

/// **Thread‑safety:** `MockEnv` is deliberately single‑threaded; it uses `Rc`/`RefCell` and does **not** implement `Send` or `Sync`. This ensures deterministic behavior in tests but means fixtures cannot be moved across async tasks.
/// A wrapper around the Soroban test environment with additional helpers.
///
/// **Host-only:** This type uses `std` and Soroban host test utilities.
/// It must only be used inside `#[cfg(test)]` blocks on the host,
/// never in contract WASM builds.
#[derive(Clone)]
pub struct MockEnv {
    inner: Env,
    accounts: Rc<RefCell<HashMap<String, Address>>>,
    contract_ids: Rc<RefCell<HashMap<String, Address>>>,
    tokens: Rc<RefCell<HashMap<String, MockToken>>>,
    xlm_token_address: Rc<RefCell<Option<Address>>>,
    prng_state: Rc<RefCell<[u8; 32]>>,
    track_costs: bool,
    crypto_registry: Rc<RefCell<MockCryptoRegistry>>,
    checkpoints: Rc<RefCell<CheckpointStack>>,
}

// Typed event wrapper to provide ergonomic access to event fields and typed data conversion.
#[derive(Clone)]
pub struct CapturedEvent {
    env: Env,
    pub contract: Address,
    pub topics: SorobanVec<Val>,
    pub data: Val,
}

impl std::fmt::Debug for CapturedEvent {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("CapturedEvent")
            .field("contract", &self.contract)
            .field("topics", &self.topics)
            .field("data", &self.data)
            .finish()
    }
}

impl CapturedEvent {
    /// Returns the contract address that emitted the event.
    pub fn contract(&self) -> Address {
        self.contract.clone()
    }

    /// Returns the raw topics as a SorobanVec<Val>.
    pub fn topics(&self) -> SorobanVec<Val> {
        self.topics.clone()
    }

    /// Returns the raw data value.
    pub fn data_raw(&self) -> Val {
        self.data
    }

    /// Convert the event data into a typed Rust value using Soroban's `FromVal`.
    ///
    /// ```ignore
    /// use crucible::prelude::*;
    /// use soroban_sdk::symbol_short;
    ///
    /// let events = env.events_parsed((symbol_short!("minted"),));
    /// for ev in &events {
    ///     let amount: i128 = ev.data_as();
    ///     assert!(amount > 0);
    /// }
    /// ```
    pub fn data_as<T: FromVal<Env, Val>>(&self) -> T {
        T::from_val(&self.env, &self.data)
    }

    /// Checks whether the event's topics match the given topic pattern (with wildcard support via the `_` symbol).
    pub fn matches_topic_pattern(&self, pattern: &SorobanVec<Val>) -> bool {
        crate::event_topic_match::topics_match(&self.env, pattern, &self.topics)
    }

    /// Checks whether the event's topics match the given topic tuple or vector pattern.
    pub fn matches_topics<T: IntoVal<Env, SorobanVec<Val>>>(&self, pattern: T) -> bool {
        let pattern_vec = pattern.into_val(&self.env);
        self.matches_topic_pattern(&pattern_vec)
    }

    /// Safely decodes a topic segment at `index` into a typed Rust value if present.
    pub fn topic_as<T: FromVal<Env, Val>>(&self, index: usize) -> Option<T> {
        if (index as u32) < self.topics.len() {
            let val = self.topics.get(index as u32).unwrap();
            Some(T::from_val(&self.env, &val))
        } else {
            None
        }
    }

    /// Validates the event topic segment count and applies a schema validator closure to the data payload.
    pub fn assert_schema<F: FnOnce(&Val) -> bool>(
        &self,
        expected_topic_count: usize,
        data_validator: F,
    ) -> bool {
        (self.topics.len() as usize) == expected_topic_count && data_validator(&self.data)
    }
}

/// Wrapper around matching events returned by `env.events_matching()` to provide
/// ergonomic access, filtering, conversion, and testing assertions.
#[derive(Clone)]
pub struct EventMatches {
    pub(crate) env: Env,
    pub(crate) items: SorobanVec<(Address, SorobanVec<Val>, Val)>,
}

impl std::fmt::Debug for EventMatches {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("EventMatches")
            .field("count", &self.items.len())
            .field("events", &self.items)
            .finish()
    }
}

impl std::ops::Deref for EventMatches {
    type Target = SorobanVec<(Address, SorobanVec<Val>, Val)>;

    fn deref(&self) -> &Self::Target {
        &self.items
    }
}

impl PartialEq for EventMatches {
    fn eq(&self, other: &Self) -> bool {
        self.items == other.items
    }
}

impl Eq for EventMatches {}

impl PartialEq<SorobanVec<(Address, SorobanVec<Val>, Val)>> for EventMatches {
    fn eq(&self, other: &SorobanVec<(Address, SorobanVec<Val>, Val)>) -> bool {
        &self.items == other
    }
}

impl PartialEq<EventMatches> for SorobanVec<(Address, SorobanVec<Val>, Val)> {
    fn eq(&self, other: &EventMatches) -> bool {
        self == &other.items
    }
}

impl EventMatches {
    /// Creates a new `EventMatches` wrapper.
    pub fn new(env: Env, items: SorobanVec<(Address, SorobanVec<Val>, Val)>) -> Self {
        Self { env, items }
    }

    /// Returns the underlying Soroban `Env`.
    pub fn env(&self) -> &Env {
        &self.env
    }

    /// Filter matching events by topic pattern supporting wildcards (the `_` symbol).
    pub fn filter_by_topic_pattern<T: IntoVal<Env, SorobanVec<Val>>>(&self, pattern: T) -> EventMatches {
        let pattern_vec = pattern.into_val(&self.env);
        let mut filtered = SorobanVec::new(&self.env);
        for item in self.items.iter() {
            let topics = &item.1;
            if crate::event_topic_match::topics_match(&self.env, &pattern_vec, topics) {
                filtered.push_back(item);
            }
        }
        EventMatches {
            env: self.env.clone(),
            items: filtered,
        }
    }

    /// Returns a reference to the inner SorobanVec.
    pub fn inner(&self) -> &SorobanVec<(Address, SorobanVec<Val>, Val)> {
        &self.items
    }

    /// Consumes self and returns the inner SorobanVec.
    pub fn into_inner(self) -> SorobanVec<(Address, SorobanVec<Val>, Val)> {
        self.items
    }

    /// Returns the number of matched events.
    pub fn len(&self) -> usize {
        self.items.len() as usize
    }

    /// Returns true if no events matched.
    pub fn is_empty(&self) -> bool {
        self.items.is_empty()
    }

    /// Returns the event tuple at the specified index, if present.
    pub fn get_event(&self, index: u32) -> Option<(Address, SorobanVec<Val>, Val)> {
        if index < self.items.len() {
            Some(self.items.get(index).unwrap())
        } else {
            None
        }
    }

    /// Returns the first matched event tuple, if any.
    pub fn first_event(&self) -> Option<(Address, SorobanVec<Val>, Val)> {
        self.get_event(0)
    }

    /// Returns the last matched event tuple, if any.
    pub fn last_event(&self) -> Option<(Address, SorobanVec<Val>, Val)> {
        let len = self.items.len();
        if len > 0 {
            Some(self.items.get(len - 1).unwrap())
        } else {
            None
        }
    }

    /// Filter matched events to only those emitted by the specified contract address.
    pub fn by_contract(&self, contract: &Address) -> Self {
        let mut filtered = SorobanVec::new(&self.env);
        for item in self.items.iter() {
            if item.0 == *contract {
                filtered.push_back(item);
            }
        }
        Self::new(self.env.clone(), filtered)
    }

    /// Decodes the event data at `index` into typed Rust value using `FromVal`.
    pub fn data_as<D: FromVal<Env, Val>>(&self, index: u32) -> D {
        let event = self.items.get(index).unwrap_or_else(|| {
            panic!(
                "EventMatches::data_as index {} out of bounds (len {})",
                index,
                self.items.len()
            )
        });
        D::from_val(&self.env, &event.2)
    }

    /// Decodes the topic at `topic_idx` of event at `event_idx` into typed Rust value using `FromVal`.
    pub fn topic_as<T: FromVal<Env, Val>>(&self, event_idx: u32, topic_idx: u32) -> T {
        let event = self.items.get(event_idx).unwrap_or_else(|| {
            panic!(
                "EventMatches::topic_as event index {} out of bounds (len {})",
                event_idx,
                self.items.len()
            )
        });
        let topic_val = event.1.get(topic_idx).unwrap_or_else(|| {
            panic!(
                "EventMatches::topic_as topic index {} out of bounds (topics len {})",
                topic_idx,
                event.1.len()
            )
        });
        T::from_val(&self.env, &topic_val)
    }

    /// Convert matches into a `Vec<CapturedEvent>` for typed inspection.
    pub fn to_captured(&self) -> std::vec::Vec<CapturedEvent> {
        let mut result = std::vec::Vec::with_capacity(self.len());
        for item in self.items.iter() {
            result.push(CapturedEvent {
                env: self.env.clone(),
                contract: item.0,
                topics: item.1,
                data: item.2,
            });
        }
        result
    }

    /// Assert that at least one matching event was emitted.
    pub fn assert_emitted(&self) {
        assert!(
            !self.is_empty(),
            "Expected at least one matching event, but none were emitted"
        );
    }

    /// Assert that exactly `expected` matching events were emitted.
    pub fn assert_count(&self, expected: usize) {
        assert_eq!(
            self.len(),
            expected,
            "Expected {} matching event(s), but found {}",
            expected,
            self.len()
        );
    }
}

impl MockEnv {
    /// Returns the underlying `soroban_sdk::Env`.
    pub fn inner(&self) -> &Env {
        &self.inner
    }

    /// Returns the current Soroban protocol version configured on the ledger.
    pub fn protocol_version(&self) -> u32 {
        self.inner.ledger().get().protocol_version
    }

    /// Creates a new `MockEnvBuilder` for fluent environment construction.
    pub fn builder() -> MockEnvBuilder {
        MockEnvBuilder::new()
    }

    /// Set the deterministic pseudo-random seed used by test helpers.
    pub fn set_prng_seed(&self, seed: [u8; 32]) {
        *self.prng_state.borrow_mut() = seed;
    }

    /// Advance and return the deterministic pseudo-random stream.
    ///
    /// This is deliberately a test PRNG, not a source of cryptographic
    /// randomness. Equal seeds produce equal sequences across environments.
    pub fn next_prng_u64(&self) -> u64 {
        let mut state = self.prng_state.borrow_mut();
        let mut value = u64::from_le_bytes(state[..8].try_into().expect("fixed seed width"));
        value ^= value << 13;
        value ^= value >> 7;
        value ^= value << 17;
        state[..8].copy_from_slice(&value.to_le_bytes());
        value
    }

    /// Get an account handle by name.
    pub fn account(&self, name: &str) -> AccountHandle {
        let address = self.accounts
            .borrow()
            .get(name)
            .cloned()
            .unwrap_or_else(|| {
                let mut available: Vec<_> = self.accounts.borrow().keys().cloned().collect();
                available.sort();
                panic!(
                    "Account '{}' not found. Available accounts: [{}]. Ensure it was registered via MockEnvBuilder or AccountBuilder.",
                    name,
                    available.join(", ")
                )
            });

        AccountHandle::new(self.clone(), name.to_string(), address)
    }

    /// Get a registered mock token by symbol.
    ///
    /// # Panics
    /// Panics with a clear error message if the symbol was not registered via
    /// [`MockEnvBuilder::with_token`].
    pub fn token(&self, symbol: &str) -> MockToken {
        self.token_opt(symbol).unwrap_or_else(|| {
            let mut available: Vec<_> = self.tokens.borrow().keys().cloned().collect();
            available.sort();
            panic!(
                "Token '{}' not found in MockEnv. Available tokens: [{}]. Ensure it was registered via MockEnvBuilder.",
                symbol,
                available.join(", ")
            )
        })
    }

    /// Get a registered mock token by symbol, returning `None` if not registered.
    pub fn token_opt(&self, symbol: &str) -> Option<MockToken> {
        self.tokens.borrow().get(symbol).cloned()
    }

    /// Registers a [`MockToken`] by symbol into this environment's token registry.
    pub fn register_token(&self, symbol: &str, token: MockToken) {
        self.tokens
            .borrow_mut()
            .insert(symbol.to_string(), token);
    }

    /// Get a contract ID by type.
    pub fn contract_id<C>(&self) -> Address {
        let type_name = std::any::type_name::<C>();
        self.contract_ids
            .borrow()
            .get(type_name)
            .cloned()
            .unwrap_or_else(|| {
                let mut available: Vec<_> = self.contract_ids.borrow().keys().cloned().collect();
                available.sort();
                panic!(
                    "Contract '{}' not registered. Available contracts: [{}]",
                    type_name,
                    available.join(", ")
                )
            })
    }

    /// Enable mock authorization for all calls.
    ///
    /// The bypass persists until explicitly cleared. Prefer
    /// [`with_mock_all_auths`] or [`mock_all_auths_scoped`] to contain the
    /// bypass to a single block.
    pub fn mock_all_auths(&self) {
        self.inner.mock_all_auths();
    }

    /// Enable mock authorization for the duration of a closure, then clear it.
    ///
    /// After `f` returns, `mock_auths(&[])` is called so that subsequent
    /// `require_auth()` calls are enforced normally.
    ///
    /// # Example
    /// ```rust,ignore
    /// env.with_mock_all_auths(|| {
    ///     contract.initialize(&admin);
    /// });
    /// // Auth required again from here on.
    /// ```
    pub fn with_mock_all_auths<F, T>(&self, f: F) -> T
    where
        F: FnOnce() -> T,
    {
        self.inner.mock_all_auths();
        let result = f();
        self.inner.mock_auths(&[]);
        result
    }

    /// Returns an RAII guard that enables mock authorization until dropped.
    ///
    /// Useful when the scoped block spans multiple statements that cannot
    /// easily be wrapped in a single closure.
    ///
    /// # Example
    /// ```rust,ignore
    /// {
    ///     let _guard = env.mock_all_auths_scoped();
    ///     contract.step_one();
    ///     contract.step_two();
    /// } // guard dropped — auth restored
    /// ```
    pub fn mock_all_auths_scoped(&self) -> MockAuthGuard {
        self.inner.mock_all_auths();
        MockAuthGuard {
            env: self.inner.clone(),
        }
    }

    /// Set explicit mock authorizations for subsequent contract calls.
    ///
    /// Unlike [`mock_all_auths`](Self::mock_all_auths), this authorizes only the
    /// invocations described by the supplied entries. Passing an empty slice
    /// clears all mocked authorizations so that `require_auth()` calls fail —
    /// useful for negative authorization tests.
    pub fn mock_auths(&self, auths: &[soroban_sdk::testutils::MockAuth<'_>]) {
        self.inner.mock_auths(auths);
    }

    /// Returns the current ledger timestamp (UNIX seconds).
    pub fn timestamp(&self) -> u64 {
        self.inner.ledger().get().timestamp
    }

    /// Returns the current ledger sequence number.
    pub fn ledger_sequence(&self) -> u32 {
        self.inner.ledger().get().sequence_number
    }

    /// Advance the ledger timestamp by a duration.
    ///
    /// # Panics
    /// Panics if the timestamp overflows.
    pub fn advance_time(&self, duration: Duration) {
        // Guard: zero duration is a no-op
        if duration.as_seconds() == 0 {
            return;
        }

        let info = self.inner.ledger().get();
        let new_ts = info
            .timestamp
            .checked_add(duration.as_seconds())
            .expect("timestamp overflow in advance_time");
        self.inner.ledger().set(soroban_sdk::testutils::LedgerInfo {
            sequence_number: info.sequence_number,
            timestamp: new_ts,
            protocol_version: info.protocol_version,
            base_reserve: info.base_reserve,
            network_id: info.network_id,
            min_temp_entry_ttl: info.min_temp_entry_ttl,
            min_persistent_entry_ttl: info.min_persistent_entry_ttl,
            max_entry_ttl: info.max_entry_ttl,
        });
    }

    /// Advance the ledger timestamp by `months` using calendar month arithmetic.
    ///
    /// When the current day does not exist in the target month (e.g. Jan 31 → Feb),
    /// the result is clamped to the last valid day of that month.
    pub fn advance_time_by_months(&self, months: u32) {
        let info = self.inner.ledger().get();
        let new_timestamp = crate::time::add_months(info.timestamp, months);
        self.inner.ledger().set(soroban_sdk::testutils::LedgerInfo {
            sequence_number: info.sequence_number,
            timestamp: new_timestamp,
            protocol_version: info.protocol_version,
            base_reserve: info.base_reserve,
            network_id: info.network_id,
            min_temp_entry_ttl: info.min_temp_entry_ttl,
            min_persistent_entry_ttl: info.min_persistent_entry_ttl,
            max_entry_ttl: info.max_entry_ttl,
        });
    }

    /// Advance the ledger timestamp by `years` using calendar year arithmetic.
    ///
    /// When the current day does not exist in the target year (e.g. Feb 29 → non-leap year),
    /// the result is clamped to Feb 28.
    pub fn advance_time_by_years(&self, years: u32) {
        let info = self.inner.ledger().get();
        let new_timestamp = crate::time::add_years(info.timestamp, years);
        self.inner.ledger().set(soroban_sdk::testutils::LedgerInfo {
            sequence_number: info.sequence_number,
            timestamp: new_timestamp,
            protocol_version: info.protocol_version,
            base_reserve: info.base_reserve,
            network_id: info.network_id,
            min_temp_entry_ttl: info.min_temp_entry_ttl,
            min_persistent_entry_ttl: info.min_persistent_entry_ttl,
            max_entry_ttl: info.max_entry_ttl,
        });
    }

    /// Advances the ledger clock by `ledgers`, moving time and sequence together.
    ///
    /// Time-dependent contracts — vesting schedules, auctions, timelocks — read
    /// both the ledger timestamp and its sequence number, and Soroban ties
    /// storage entry lifetimes to the sequence. Advancing only the timestamp
    /// with [`advance_time`](Self::advance_time) leaves the sequence behind, so
    /// entries never age; advancing only the sequence with
    /// [`advance_sequence`](Self::advance_sequence) ages entries while the clock
    /// stands still. Either produces a ledger state that cannot occur on the
    /// network, and the resulting TTL expirations are artefacts of the test
    /// rather than behaviour of the contract.
    ///
    /// This method moves both in step: the sequence advances by `ledgers` and
    /// the timestamp by `ledgers * seconds_per_ledger`. Soroban's own close
    /// time is roughly 5 seconds, which [`advance_ledgers`](Self::advance_ledgers)
    /// uses as its default.
    ///
    /// Temporary storage whose time-to-live elapses over the advance is
    /// collected by the host exactly as it would be on-chain, so a test can
    /// assert that a temporary entry is gone and a persistent one survives.
    ///
    /// # Panics
    ///
    /// Panics if the timestamp or sequence number overflows.
    ///
    /// ```ignore
    /// // Roll a vesting schedule forward one full day of ledgers.
    /// env.advance_ledger(17_280, 5);
    /// assert!(vesting.claimable(&alice) > 0);
    /// ```
    pub fn advance_ledger(&self, ledgers: u32, seconds_per_ledger: u64) {
        if ledgers == 0 {
            return;
        }

        let info = self.inner.ledger().get();
        let elapsed = (ledgers as u64)
            .checked_mul(seconds_per_ledger)
            .expect("elapsed seconds overflow in advance_ledger");
        let timestamp = info
            .timestamp
            .checked_add(elapsed)
            .expect("timestamp overflow in advance_ledger");
        let sequence_number = info
            .sequence_number
            .checked_add(ledgers)
            .expect("sequence number overflow in advance_ledger");

        self.inner.ledger().set(soroban_sdk::testutils::LedgerInfo {
            sequence_number,
            timestamp,
            protocol_version: info.protocol_version,
            base_reserve: info.base_reserve,
            network_id: info.network_id,
            min_temp_entry_ttl: info.min_temp_entry_ttl,
            min_persistent_entry_ttl: info.min_persistent_entry_ttl,
            max_entry_ttl: info.max_entry_ttl,
        });
    }

    /// Advances the ledger clock by `ledgers` at Soroban's nominal close time.
    ///
    /// Shorthand for [`advance_ledger(ledgers, 5)`](Self::advance_ledger); the
    /// Stellar network closes a ledger roughly every 5 seconds.
    pub fn advance_ledgers(&self, ledgers: u32) {
        self.advance_ledger(ledgers, DEFAULT_SECONDS_PER_LEDGER);
    }

    /// Advances the ledger clock far enough to cover `duration`.
    ///
    /// The complement of [`advance_ledger`](Self::advance_ledger) for tests
    /// expressed in wall-clock terms: it derives the ledger count from the
    /// duration so the sequence still moves with the timestamp, rather than
    /// leaving it behind as [`advance_time`](Self::advance_time) does.
    ///
    /// The ledger count is rounded up, so the clock never stops short of
    /// `duration`.
    ///
    /// # Panics
    ///
    /// Panics if `seconds_per_ledger` is zero, or if the ledger count exceeds
    /// [`u32::MAX`].
    ///
    /// ```ignore
    /// // Cross a 30-day cliff with a consistent sequence number.
    /// env.advance_ledger_time(Duration::days(30), 5);
    /// ```
    pub fn advance_ledger_time(&self, duration: Duration, seconds_per_ledger: u64) {
        assert!(
            seconds_per_ledger > 0,
            "seconds_per_ledger must be greater than zero"
        );

        let seconds = duration.as_seconds();
        if seconds == 0 {
            return;
        }

        // Round up so the clock never stops short of the requested duration.
        let ledgers = seconds.div_ceil(seconds_per_ledger);
        let ledgers = u32::try_from(ledgers)
            .expect("advance_ledger_time requires at most u32::MAX ledgers");
        self.advance_ledger(ledgers, seconds_per_ledger);
    }

    /// Returns the ledger's time-to-live settings.
    ///
    /// These bound how long storage entries survive: a temporary entry starts
    /// with at least `min_temp`, a persistent entry with at least
    /// `min_persistent`, and neither may be extended past `max`.
    pub fn entry_ttl_settings(&self) -> EntryTtlSettings {
        let info = self.inner.ledger().get();
        EntryTtlSettings {
            min_temp: info.min_temp_entry_ttl,
            min_persistent: info.min_persistent_entry_ttl,
            max: info.max_entry_ttl,
        }
    }

    /// Overrides the ledger's time-to-live settings.
    ///
    /// Lower the minimums to reach an expiry in a handful of ledgers rather
    /// than the thousands the network defaults to, so a test for expiring
    /// escrow or vesting state stays fast and readable.
    ///
    /// ```ignore
    /// // A temporary entry now lapses after 4 ledgers instead of 16.
    /// env.set_entry_ttl_settings(EntryTtlSettings { min_temp: 4, ..env.entry_ttl_settings() });
    /// ```
    pub fn set_entry_ttl_settings(&self, settings: EntryTtlSettings) {
        let info = self.inner.ledger().get();
        self.inner.ledger().set(soroban_sdk::testutils::LedgerInfo {
            sequence_number: info.sequence_number,
            timestamp: info.timestamp,
            protocol_version: info.protocol_version,
            base_reserve: info.base_reserve,
            network_id: info.network_id,
            min_temp_entry_ttl: settings.min_temp,
            min_persistent_entry_ttl: settings.min_persistent,
            max_entry_ttl: settings.max,
        });
    }

    /// Advance the ledger sequence number by n.
    ///
    /// This moves the sequence without moving the clock, which ages storage
    /// entries while the timestamp stands still. Prefer
    /// [`advance_ledger`](Self::advance_ledger) unless a test specifically
    /// needs the two to diverge.
    pub fn advance_sequence(&self, n: u32) {
        // Guard: zero is a no-op
        if n == 0 {
            return;
        }

        let info = self.inner.ledger().get();
        self.inner.ledger().set(soroban_sdk::testutils::LedgerInfo {
            sequence_number: info.sequence_number + n,
            timestamp: info.timestamp,
            protocol_version: info.protocol_version,
            base_reserve: info.base_reserve,
            network_id: info.network_id,
            min_temp_entry_ttl: info.min_temp_entry_ttl,
            min_persistent_entry_ttl: info.min_persistent_entry_ttl,
            max_entry_ttl: info.max_entry_ttl,
        });
    }

    /// Set the ledger timestamp to an absolute value.
    pub fn set_timestamp(&self, unix_ts: u64) {
        let info = self.inner.ledger().get();
        self.inner.ledger().set(soroban_sdk::testutils::LedgerInfo {
            sequence_number: info.sequence_number,
            timestamp: unix_ts,
            protocol_version: info.protocol_version,
            base_reserve: info.base_reserve,
            network_id: info.network_id,
            min_temp_entry_ttl: info.min_temp_entry_ttl,
            min_persistent_entry_ttl: info.min_persistent_entry_ttl,
            max_entry_ttl: info.max_entry_ttl,
        });
    }

    /// Set the ledger sequence number to an absolute value.
    pub fn set_sequence(&self, n: u32) {
        let info = self.inner.ledger().get();
        self.inner.ledger().set(soroban_sdk::testutils::LedgerInfo {
            sequence_number: n,
            timestamp: info.timestamp,
            protocol_version: info.protocol_version,
            base_reserve: info.base_reserve,
            network_id: info.network_id,
            min_temp_entry_ttl: info.min_temp_entry_ttl,
            min_persistent_entry_ttl: info.min_persistent_entry_ttl,
            max_entry_ttl: info.max_entry_ttl,
        });
    }

    /// Register an account with a name.
    pub fn register_account(&self, name: &str, address: Address) {
        if self.accounts.borrow().contains_key(name) {
            panic!("Account '{}' already registered. Use a unique name.", name);
        }
        self.accounts.borrow_mut().insert(name.to_string(), address);
    }

    /// Register a contract with its type name.
    pub fn register_contract<C>(&self, address: Address) {
        let type_name = std::any::type_name::<C>();
        self.contract_ids
            .borrow_mut()
            .insert(type_name.to_string(), address);
    }

    /// Returns all events emitted during the test.
    ///
    /// In Soroban SDK v25.x, this returns the ContractEvents wrapper.
    pub fn events_all(&self) -> ContractEvents {
        self.inner.events().all()
    }

    /// Returns all events emitted by a specific contract address.
    ///
    /// Useful for asserting that a particular contract (in a multi-contract
    /// scenario) emitted (or did not emit) certain events.
    ///
    /// # Example
    ///
    /// ```ignore
    /// let pool_events = env.events_from_contract(&pool_address);
    /// assert!(!pool_events.is_empty());
    /// ```
    pub fn events_from_contract(
        &self,
        contract_id: &Address,
    ) -> SorobanVec<(Address, SorobanVec<Val>, Val)> {
        use soroban_sdk::xdr::{self, ScAddress};
        let all_events = self.inner.events().all();
        let mut result = SorobanVec::new(&self.inner);
        for event in all_events.events() {
            if let Some(ref id) = event.contract_id {
                let sc_addr = ScAddress::Contract(id.clone());
                let addr = Address::from_val(&self.inner, &sc_addr);
                if addr == *contract_id {
                    let xdr::ContractEventBody::V0(body) = &event.body;
                    let topics: SorobanVec<Val> = body.topics.clone().into_val(&self.inner);
                    let data: Val = body.data.clone().into_val(&self.inner);
                    result.push_back((addr, topics, data));
                }
            }
        }
        result
    }

    /// Returns all events emitted by any of the given contract addresses.
    ///
    /// Useful for tracking events across multiple contracts simultaneously,
    /// e.g. verifying that an aggregator routed a call through exactly one pool.
    ///
    /// # Example
    ///
    /// ```ignore
    /// let events = env.events_from_contracts(&[&pool_a, &pool_b]);
    /// assert_eq!(events.len(), 1); // only one pool was used
    /// ```
    pub fn events_from_contracts(
        &self,
        contract_ids: &[&Address],
    ) -> SorobanVec<(Address, SorobanVec<Val>, Val)> {
        use soroban_sdk::xdr::{self, ScAddress};
        let all_events = self.inner.events().all();
        let mut result = SorobanVec::new(&self.inner);
        for event in all_events.events() {
            if let Some(ref id) = event.contract_id {
                let sc_addr = ScAddress::Contract(id.clone());
                let addr = Address::from_val(&self.inner, &sc_addr);
                if contract_ids.iter().any(|c| **c == addr) {
                    let xdr::ContractEventBody::V0(body) = &event.body;
                    let topics: SorobanVec<Val> = body.topics.clone().into_val(&self.inner);
                    let data: Val = body.data.clone().into_val(&self.inner);
                    result.push_back((addr, topics, data));
                }
            }
        }
        result
    }

    /// Returns events matching the given topics wrapped in ergonomic [`EventMatches`].
    ///
    /// Updated for Soroban SDK v25.x ContractEvents compatibility and programmatic inspection ergonomics.
    pub fn events_matching<T>(&self, topics: T) -> EventMatches
    where
        T: IntoVal<Env, SorobanVec<Val>>,
    {
        let filter_topics: SorobanVec<Val> = topics.into_val(&self.inner);
        let all_events = self.inner.events().all();
        let mut matching = SorobanVec::new(&self.inner);

        // We use the internal representation for filtering in this helper
        use soroban_sdk::xdr::{self, ScAddress};
        for event in all_events.events() {
            // Skip diagnostic/system events that lack a contract ID.
            let hash = match event.contract_id.as_ref() {
                Some(id) => id,
                None => continue,
            };
            let xdr::ContractEventBody::V0(body) = &event.body;
            let event_topics: SorobanVec<Val> = body.topics.clone().into_val(&self.inner);
            if event_topics.len() < filter_topics.len() {
                continue;
            }
            let matches =
                crate::event_topic_match::topics_match(&self.inner, &filter_topics, &event_topics);
            if matches {
                let sc_addr = ScAddress::Contract(hash.clone());
                let contract_id = Address::from_val(&self.inner, &sc_addr);
                let data: Val = body.data.clone().into_val(&self.inner);
                matching.push_back((contract_id, event_topics, data));
            }
        }
        EventMatches::new(self.inner.clone(), matching)
    }

    /// Returns events matching the given topics as typed [`CapturedEvent`] wrappers.
    ///
    /// This keeps the low-level [`events_matching`](Self::events_matching) available
    /// for advanced users while providing an ergonomic path to decode event data into
    /// concrete Rust types via [`CapturedEvent::data_as`].
    ///
    /// ```ignore
    /// use crucible::prelude::*;
    /// use soroban_sdk::symbol_short;
    ///
    /// // After invoking a contract that emits `(symbol_short!("minted"),)` with i128 data:
    /// let events: Vec<CapturedEvent> = env.events_parsed((symbol_short!("minted"),));
    /// assert_eq!(events.len(), 1);
    /// let amount: i128 = events[0].data_as();
    /// assert_eq!(amount, 1_000);
    /// ```
    pub fn events_parsed<T>(&self, topics: T) -> std::vec::Vec<CapturedEvent>
    where
        T: IntoVal<Env, SorobanVec<Val>>,
    {
        let filter_topics: SorobanVec<Val> = topics.into_val(&self.inner);
        let all_events = self.inner.events().all();
        let mut parsed = Vec::new();

        // We use the internal representation for filtering in this helper
        use soroban_sdk::xdr::{self, ScAddress};
        for event in all_events.events() {
            // Skip diagnostic/system events that lack a contract ID.
            let hash = match event.contract_id.as_ref() {
                Some(id) => id,
                None => continue,
            };
            let xdr::ContractEventBody::V0(body) = &event.body;
            let event_topics: SorobanVec<Val> = body.topics.clone().into_val(&self.inner);
            if event_topics.len() < filter_topics.len() {
                continue;
            }
            let matches =
                crate::event_topic_match::topics_match(&self.inner, &filter_topics, &event_topics);
            if matches {
                let sc_addr = ScAddress::Contract(hash.clone());
                let contract_id = Address::from_val(&self.inner, &sc_addr);
                let data: Val = body.data.clone().into_val(&self.inner);
                parsed.push(CapturedEvent {
                    env: self.inner.clone(),
                    contract: contract_id,
                    topics: event_topics,
                    data,
                });
            }
        }
        parsed
    }

    /// Set the XLM token address for the environment.
    pub fn set_xlm_token_address(&self, address: Address) {
        *self.xlm_token_address.borrow_mut() = Some(address);
    }

    /// Get the XLM token address for the environment, if set.
    pub fn xlm_token_address(&self) -> Option<Address> {
        self.xlm_token_address.borrow().clone()
    }

    /// Check if cost tracking is enabled.
    pub fn track_costs(&self) -> bool {
        self.track_costs
    }

    /// Measure the execution cost of a contract call.
    pub fn measure<F, T>(&self, f: F) -> CostReport
    where
        F: FnOnce() -> T,
    {
        if !self.track_costs {
            panic!("MockEnv::measure() requires track_costs() to be enabled during environment construction");
        }

        let mut budget = self.inner.budget();
        budget.reset_default();
        let _result = f();
        let fee = self.inner.cost_estimate().fee();
        let fee_estimate = crate::cost::FeeEstimate {
            total: fee.total,
            instructions: fee.instructions,
            disk_read_entries: fee.disk_read_entries,
            write_entries: fee.write_entries,
            disk_read_bytes: fee.disk_read_bytes,
            write_bytes: fee.write_bytes,
            contract_events: fee.contract_events,
            persistent_entry_rent: fee.persistent_entry_rent,
            temporary_entry_rent: fee.temporary_entry_rent,
        };
        CostReport::new_with_fee_estimate(
            budget.cpu_instruction_cost(),
            budget.memory_bytes_cost(),
            fee_estimate.total as i128,
        )
    }

    /// Run a contract call once and capture its dry-run estimate, without
    /// retaining any way to commit it.
    ///
    /// This is the **inspect-only** API: the returned [`SimulatedTx`] holds no
    /// commit closure and imposes no `'static` bound, so the closure may
    /// borrow freely and `T` need not be `'static`. The closure runs exactly
    /// once and no state changes are committed.
    ///
    /// Auth is globally bypassed only for the duration of the dry-run call.
    /// After `simulate` returns the auth mock is cleared, so subsequent
    /// operations require explicit auth setup and will not silently pass.
    ///
    /// Use [`prepare`](Self::prepare) instead when you need to commit the call
    /// after inspecting the estimate.
    ///
    /// ```ignore
    /// // Look at the cost of a transfer without applying it.
    /// let sim = env.simulate(|| client.transfer(&from, &to, &100));
    /// assert!(sim.would_succeed());
    /// ```
    pub fn simulate<F, T>(&self, f: F) -> SimulatedTx<T>
    where
        F: FnOnce() -> T,
    {
        self.dry_run(f)
    }

    /// Run a contract call's dry-run and return a **commit-capable**
    /// [`PreparedTx`] that can later apply the call's state changes.
    ///
    /// The closure runs once here to produce the estimate (with auth mocked for
    /// that run only, then cleared) and is retained so it can run again when
    /// [`PreparedTx::commit`] is called. Because the closure is stored by
    /// generic type rather than boxed, there is no `'static` requirement.
    ///
    /// Use [`simulate`](Self::simulate) instead when you only need to inspect
    /// the call and will never commit it.
    ///
    /// ```ignore
    /// // Inspect, then commit only if the estimate is acceptable.
    /// let prepared = env.prepare(|| client.transfer(&from, &to, &100));
    /// if prepared.would_succeed() {
    ///     prepared.commit();
    /// }
    /// ```
    pub fn prepare<F, T>(&self, f: F) -> PreparedTx<F, T>
    where
        F: Fn() -> T,
    {
        let simulation = self.dry_run(|| f());
        PreparedTx::new(simulation, f)
    }

    /// Execute `f` once under mocked auth and capture the dry-run metrics.
    ///
    /// Shared by [`simulate`](Self::simulate) and [`prepare`](Self::prepare).
    /// The global auth bypass is cleared before returning so it does not leak
    /// into later operations.
    fn dry_run<F, T>(&self, f: F) -> SimulatedTx<T>
    where
        F: FnOnce() -> T,
    {
        let mut budget = self.inner.budget();
        budget.reset_default();

        self.inner.mock_all_auths();
        let result = f();
        let instructions = budget.cpu_instruction_cost();
        let fee = self.inner.cost_estimate().fee().total;
        let auths = self.inner.auths().iter().map(|(a, _)| a.clone()).collect();
        // Clear the global auth bypass so it does not leak into later operations.
        self.inner.mock_auths(&[]);

        SimulatedTx::new(fee, instructions, auths, true, Some(result))
    }

    /// Records the cross-contract invocation tree produced by `f`.
    ///
    /// The returned [`CallTrace`] mirrors the frames the host actually
    /// executed: callee address, function symbol, arguments, return value, and
    /// the gas apportioned to each nested frame. Use it to understand — or to
    /// assert on — the shape of a multi-contract invocation, and
    /// [`CallTrace::to_json_pretty`] to export the tree.
    ///
    /// State changes made by `f` are **kept**. Unlike
    /// [`simulate`](Self::simulate), this is an observation of a real call, not
    /// a dry run, so auth is left exactly as the caller configured it.
    ///
    /// A panic inside `f` propagates to the caller. To inspect the tree of a
    /// call that fails, use [`try_trace`](Self::try_trace), which captures the
    /// panic and still returns the frames recorded up to the failure.
    ///
    /// ```ignore
    /// let trace = env.trace(|| router.swap(&alice, &100));
    /// trace.assert_called("transfer");
    /// assert_eq!(trace.max_depth(), 2);
    /// ```
    pub fn trace<F, T>(&self, f: F) -> (T, crate::call_graph::CallTrace)
    where
        F: FnOnce() -> T,
    {
        let before = self.begin_trace();
        let result = f();
        let trace = self.end_trace(before);
        (result, trace)
    }

    /// Records the invocation tree of a call that may panic.
    ///
    /// Behaves like [`trace`](Self::trace) but captures a panic raised by `f`,
    /// returning `Err` with the frames recorded up to the failure. The frame
    /// that panicked has no return value, so
    /// [`CallTrace::panicked_frames`] identifies exactly where the invocation
    /// tree broke — which is the case an opaque host panic code leaves
    /// undiagnosable.
    ///
    /// ```ignore
    /// let (result, trace) = env.try_trace(|| router.swap(&alice, &too_much));
    /// assert!(result.is_err());
    /// let failed = trace.panicked_frames();
    /// assert_eq!(failed[0].function, "transfer");
    /// ```
    pub fn try_trace<F, T>(
        &self,
        f: F,
    ) -> (
        Result<T, Box<dyn std::any::Any + Send>>,
        crate::call_graph::CallTrace,
    )
    where
        F: FnOnce() -> T,
    {
        let before = self.begin_trace();
        // `MockEnv` holds `Rc<RefCell<..>>` and so is not `RefUnwindSafe`; the
        // environment is only read after the unwind, never left half-updated by
        // it, so the assertion is sound here.
        let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(f));
        let trace = self.end_trace(before);
        (result, trace)
    }

    /// Enables host diagnostics, resets the budget, and snapshots the buffer.
    ///
    /// The host clears its diagnostic event buffer at the start of every
    /// top-level invocation, so after a call the buffer holds exactly that
    /// call's frames. A closure that invokes no contract leaves the previous
    /// call's buffer in place, so the snapshot taken here lets `end_trace`
    /// recognise that nothing new ran and report an empty trace.
    ///
    /// The budget is reset so the instruction and fee figures attributed to the
    /// tree describe this call rather than everything the test has run so far.
    fn begin_trace(&self) -> std::vec::Vec<soroban_sdk::xdr::ContractEvent> {
        self.inner
            .host()
            .enable_debug()
            .expect("enabling host diagnostics must succeed");
        self.inner.cost_estimate().budget().reset_default();
        self.diagnostic_events()
    }

    /// Builds the call tree from the diagnostic events of the traced call.
    fn end_trace(
        &self,
        before: std::vec::Vec<soroban_sdk::xdr::ContractEvent>,
    ) -> crate::call_graph::CallTrace {
        let after = self.diagnostic_events();

        // An unchanged buffer means the closure invoked no contract, so the
        // events still present belong to an earlier call and are not this
        // trace's to report. The fee estimate is a running total the budget
        // reset does not clear, so it is likewise not attributable here.
        if after == before {
            return crate::call_graph::build_trace(&self.inner, &[], 0, 0);
        }

        let instructions = self.inner.cost_estimate().budget().cpu_instruction_cost();
        let fee = self.inner.cost_estimate().fee().total;
        crate::call_graph::build_trace(&self.inner, &after, instructions, fee)
    }

    /// Reads the host's current diagnostic event buffer.
    fn diagnostic_events(&self) -> std::vec::Vec<soroban_sdk::xdr::ContractEvent> {
        self.inner
            .host()
            .get_diagnostic_events()
            .map(|events| events.0)
            .unwrap_or_default()
            .into_iter()
            .map(|host_event| host_event.event)
            .collect()
    }

    /// Inspect a contract call without the ability to commit.
    ///
    /// Unlike `simulate`, this method does not require the closure to be `'static`,
    /// allowing it to borrow local clients, accounts, or fixture references.
    ///
    /// Auth is globally bypassed only for the duration of the dry-run call.
    /// After `simulate_inspect` returns the auth mock is cleared, so subsequent
    /// operations require explicit auth setup and will not silently pass.
    pub fn simulate_inspect<F, T>(&self, f: F) -> SimulatedTx<T>
    where
        F: FnOnce() -> T,
    {
        let mut budget = self.inner.budget();
        budget.reset_default();

        self.inner.mock_all_auths();
        #[allow(unused_variables)]
        let result = f();
        let instructions = budget.cpu_instruction_cost();
        let fee = self.inner.cost_estimate().fee().total;
        let auths = self.inner.auths().iter().map(|(a, _)| a.clone()).collect();
        // Clear the global auth bypass so it does not leak into later operations.
        self.inner.mock_auths(&[]);

        SimulatedTx::new(fee, instructions, auths, true, Some(result))
    }

    /// Creates a fully independent copy of this environment.
    ///
    /// Unlike [`Clone`], `fork` deep-copies the shared [`Rc`]`<`[`RefCell`]`<...>>`
    /// fields so that mutations in the fork are **not** visible in the original
    /// (and vice versa).
    ///
    /// The underlying [`Env`] is also cloned. In Soroban's test environment, this
    /// creates a new handle that shares state with the original — there is no
    /// built-in way to fully isolate ledger state in the Soroban SDK test utils.
    /// Use `fork` when you want independent account/contract registries while
    /// working within the same Soroban ledger.
    pub fn fork(&self) -> Self {
        Self {
            inner: self.inner.clone(),
            accounts: Rc::new(RefCell::new(self.accounts.borrow().clone())),
            contract_ids: Rc::new(RefCell::new(self.contract_ids.borrow().clone())),
            tokens: Rc::new(RefCell::new(self.tokens.borrow().clone())),
            xlm_token_address: Rc::new(RefCell::new(self.xlm_token_address.borrow().clone())),
            prng_state: Rc::new(RefCell::new(*self.prng_state.borrow())),
            track_costs: self.track_costs,
            crypto_registry: Rc::new(RefCell::new(self.crypto_registry.borrow().clone())),
            // A fork gets a fresh stack: checkpoints taken in one environment
            // must never be redeemable in the other.
            checkpoints: Rc::new(RefCell::new(self.checkpoints.borrow().forked())),
        }
    }

    // Location: contracts/crucible/src/env.rs // Production requirement: Zero-Knowledge Proof & BN254/BLS12-381 Verifier Mock Harness

    /// Mock BN254 G1 addition (`bn254_g1_add` host function stand-in).
    pub fn bn254_g1_add(&self, p: G1, q: G1) -> G1 {
        let _ = self;
        zk::g1_add(p, q)
    }

    /// Mock BN254 G1 scalar multiplication (`bn254_g1_mul` host function stand-in).
    pub fn bn254_g1_mul(&self, p: G1, scalar: u64) -> G1 {
        let _ = self;
        zk::g1_mul(p, scalar)
    }

    /// Mock BN254 G2 addition (`bn254_g2_add` host function stand-in).
    pub fn bn254_g2_add(&self, p: G2, q: G2) -> G2 {
        let _ = self;
        zk::g2_add(p, q)
    }

    /// Mock BN254 G2 scalar multiplication (`bn254_g2_mul` host function stand-in).
    pub fn bn254_g2_mul(&self, p: G2, scalar: u64) -> G2 {
        let _ = self;
        zk::g2_mul(p, scalar)
    }

    /// Mock BN254 multi-pairing check (`bn254_pairing_check` host function stand-in).
    pub fn bn254_pairing_check(&self, pairs: &[(G1, G2)]) -> bool {
        let _ = self;
        zk::pairing_check(pairs)
    }

    /// Mock BLS12-381 G1 addition (`bls12_381_g1_add` host function stand-in).
    pub fn bls12_381_g1_add(&self, p: G1, q: G1) -> G1 {
        let _ = self;
        zk::g1_add(p, q)
    }

    /// Mock BLS12-381 G1 scalar multiplication (`bls12_381_g1_mul` host function stand-in).
    pub fn bls12_381_g1_mul(&self, p: G1, scalar: u64) -> G1 {
        let _ = self;
        zk::g1_mul(p, scalar)
    }

    /// Mock BLS12-381 G2 addition (`bls12_381_g2_add` host function stand-in).
    pub fn bls12_381_g2_add(&self, p: G2, q: G2) -> G2 {
        let _ = self;
        zk::g2_add(p, q)
    }

    /// Mock BLS12-381 G2 scalar multiplication (`bls12_381_g2_mul` host function stand-in).
    pub fn bls12_381_g2_mul(&self, p: G2, scalar: u64) -> G2 {
        let _ = self;
        zk::g2_mul(p, scalar)
    }

    /// Mock BLS12-381 multi-pairing check (`bls12_381_pairing_check` host function stand-in).
    pub fn bls12_381_pairing_check(&self, pairs: &[(G1, G2)]) -> bool {
        let _ = self;
        zk::pairing_check(pairs)
    }

    /// Sample a Groth16 verifying key for `n_public` public inputs on `curve`.
    pub fn groth16_verifying_key(
        &self,
        curve: PairingCurve,
        n_public: usize,
    ) -> Groth16VerifyingKey {
        let _ = self;
        zk::sample_verifying_key(curve, n_public)
    }

    /// Generate a valid Groth16 proof for `vk` and `public_inputs`.
    pub fn generate_groth16_proof(
        &self,
        vk: &Groth16VerifyingKey,
        public_inputs: &[u64],
    ) -> Groth16Proof {
        let _ = self;
        zk::generate_groth16_proof(vk, public_inputs)
            .expect("public input count must match verifying key IC length")
    }

    /// Verify a Groth16 proof against `vk` using the mock pairing check.
    pub fn verify_groth16(
        &self,
        vk: &Groth16VerifyingKey,
        proof: &Groth16Proof,
        public_inputs: &[u64],
    ) -> bool {
        let _ = self;
        zk::verify_groth16(vk, proof, public_inputs)
    }

    /// Produce a structurally valid but algebraically invalid Groth16 proof.
    pub fn tamper_groth16_proof(&self, proof: Groth16Proof) -> Groth16Proof {
        let _ = self;
        zk::tamper_groth16_proof(proof)
    }

    /// Generate a valid Plonk proof for a single public input.
    pub fn generate_plonk_proof(&self, curve: PairingCurve, public_input: u64) -> PlonkProof {
        let _ = self;
        zk::generate_plonk_proof(curve, public_input)
    }

    /// Verify a Plonk proof using the mock pairing check.
    pub fn verify_plonk(&self, curve: PairingCurve, proof: &PlonkProof, public_input: u64) -> bool {
        let _ = self;
        zk::verify_plonk(curve, proof, public_input)
    }

    /// Simulate a contract call that is expected to fail (panic/revert).
    ///
    /// Runs `f` under [`std::panic::catch_unwind`] and returns a
    /// [`FailedCallResult`] indicating whether the call panicked and, if so,
    /// what message was captured.  Auth is mocked for the duration of the call
    /// and cleared before returning.
    ///
    /// This is useful for asserting that a cross-contract call chain rejects
    /// invalid inputs or unauthorized callers without letting the panic
    /// propagate out of the test.
    ///
    /// # Example
    /// ```ignore
    /// let result = env.simulate_failing_call(|| client.claim());
    /// assert!(result.did_fail());
    /// assert!(result.panic_message().unwrap_or_default().contains("time lock"));
    /// ```
    pub fn simulate_failing_call<F, T>(&self, f: F) -> FailedCallResult
    where
        F: FnOnce() -> T + std::panic::UnwindSafe,
    {
        self.inner.mock_all_auths();
        let outcome = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
            f();
        }));
        self.inner.mock_auths(&[]);

        match outcome {
            Err(payload) => {
                // Try to extract a string message from the panic payload.
                let message = if let Some(s) = payload.downcast_ref::<&str>() {
                    Some(s.to_string())
                } else if let Some(s) = payload.downcast_ref::<String>() {
                    Some(s.clone())
                } else {
                    None
                };
                FailedCallResult {
                    failed: true,
                    message,
                }
            }
            Ok(()) => FailedCallResult {
                failed: false,
                message: None,
            },
        }
    }

    /// Captures the current ledger state and returns a handle to it.
    ///
    /// Contract instance, persistent and temporary storage are all captured.
    /// Registered contracts, accounts and tokens are *not* part of the
    /// snapshot and are never disturbed by a rollback, so any client or handle
    /// obtained before the checkpoint stays valid afterwards.
    ///
    /// Snapshots share their entries structurally, so a checkpoint is cheap
    /// enough to take at every node of a speculative execution tree.
    ///
    /// # Example
    ///
    /// ```rust,ignore
    /// let before = env.checkpoint();
    /// vault.liquidate(&borrower.address());
    /// env.rollback_to(before);
    /// ```
    ///
    /// See also [`rollback_to`](Self::rollback_to) and
    /// [`release_checkpoint`](Self::release_checkpoint).
    pub fn checkpoint(&self) -> CheckpointId {
        self.checkpoints.borrow_mut().capture(self.inner.host())
    }

    /// Restores the ledger to the state captured by `id`.
    ///
    /// Entries written since the checkpoint are reverted, and entries created
    /// since the checkpoint are removed. `id` itself remains valid, so the same
    /// point can be rolled back to repeatedly while exploring alternative
    /// branches.
    ///
    /// Every checkpoint taken *after* `id` is discarded, since the state those
    /// checkpoints described no longer exists.
    ///
    /// # Panics
    ///
    /// Panics if `id` came from a different environment (including a
    /// [`fork`](Self::fork)), or if it was invalidated by an earlier rollback
    /// to an older checkpoint.
    ///
    /// # Example
    ///
    /// ```rust,ignore
    /// let before = env.checkpoint();
    /// counter.increment();
    /// assert_eq!(counter.value(), 1);
    ///
    /// env.rollback_to(before);
    /// assert_eq!(counter.value(), 0);
    /// ```
    pub fn rollback_to(&self, id: CheckpointId) {
        self.checkpoints
            .borrow_mut()
            .rollback(self.inner.host(), id);
    }

    /// Discards `id` and every checkpoint taken after it without restoring.
    ///
    /// Use this when a speculative branch is accepted: the work stays, and the
    /// snapshots backing it are released.
    ///
    /// # Panics
    ///
    /// Panics under the same conditions as [`rollback_to`](Self::rollback_to).
    pub fn release_checkpoint(&self, id: CheckpointId) {
        self.checkpoints.borrow_mut().release(id);
    }

    /// Returns how many checkpoints are currently live.
    pub fn checkpoint_depth(&self) -> usize {
        self.checkpoints.borrow().depth()
    }

    /// Returns the entry counts captured by `id`, broken down by durability.
    ///
    /// # Panics
    ///
    /// Panics under the same conditions as [`rollback_to`](Self::rollback_to).
    pub fn checkpoint_stats(&self, id: CheckpointId) -> CheckpointStats {
        self.checkpoints.borrow().stats(id)
    }

    /// Runs `f` inside a checkpoint and always rolls back afterwards.
    ///
    /// This is the speculative-execution shorthand: whatever `f` writes to the
    /// ledger is discarded, while its return value is handed back to the
    /// caller. The rollback also happens if `f` panics, so a reverted
    /// speculative branch cannot leak state into the rest of the test.
    ///
    /// Unlike [`simulate`](Self::simulate), which reports the cost and auth
    /// profile of a call, this is about ledger state: it answers "what would
    /// this do?" without leaving any trace.
    ///
    /// # Example
    ///
    /// ```rust,ignore
    /// // Find out what the liquidation would pay out, without performing it.
    /// let payout = env.speculate(|| vault.liquidate(&borrower.address()));
    /// assert_eq!(vault.collateral(&borrower.address()), 1_000); // untouched
    /// ```
    pub fn speculate<F, T>(&self, f: F) -> T
    where
        F: FnOnce() -> T,
    {
        let id = self.checkpoint();
        let outcome = std::panic::catch_unwind(std::panic::AssertUnwindSafe(f));
        self.rollback_to(id);
        self.release_checkpoint(id);
        match outcome {
            Ok(value) => value,
            Err(payload) => std::panic::resume_unwind(payload),
        }
    }

    /// Runs `f` and returns a chainable assertion that it reverted.
    ///
    /// This is the fluent counterpart to the [`assert_reverts!`](crate::assert_reverts)
    /// macro. Unlike the macro, the returned [`RevertAssertion`] is a value: it
    /// can be stored, passed around, and refined with
    /// [`with_error`](RevertAssertion::with_error),
    /// [`with_any_error`](RevertAssertion::with_any_error) or
    /// [`with_message_containing`](RevertAssertion::with_message_containing)
    /// before being checked with [`verify`](RevertAssertion::verify) or
    /// [`and_assert`](RevertAssertion::and_assert).
    ///
    /// `f` runs immediately, so any state the revert was supposed to leave
    /// untouched can be inspected inside `and_assert`.
    ///
    /// The assertion is `#[must_use]` and panics if dropped unchecked, so a
    /// forgotten `.verify()` fails the test rather than passing silently.
    ///
    /// # Example
    ///
    /// ```rust,ignore
    /// use crucible::prelude::*;
    ///
    /// env.expect_revert(|| client.transfer(&alice.address(), &bob.address(), &200_i128))
    ///     .with_error(ContractError::Unauthorized)
    ///     .verify();
    ///
    /// env.expect_revert(|| client.withdraw(&alice.address(), &999_i128))
    ///     .with_any_error()
    ///     .and_assert(|| {
    ///         // The failed withdrawal must not have moved any balance.
    ///         assert_eq!(token.balance(&alice.address()), 500_i128);
    ///     });
    /// ```
    pub fn expect_revert<F, T>(&self, f: F) -> RevertAssertion
    where
        F: FnOnce() -> T,
    {
        RevertAssertion::capture(f)
    }

    /// Returns a mutable reference guard to the mock crypto registry.
    pub fn crypto_registry(&self) -> std::cell::RefMut<'_, MockCryptoRegistry> {
        self.crypto_registry.borrow_mut()
    }

    /// Generates a deterministic mock keypair for the given curve and seed.
    pub fn generate_keypair(&self, curve: CryptoCurve, seed: u64) -> MockKeyPair {
        match curve {
            CryptoCurve::Ed25519 => MockKeyPair::generate_ed25519(seed),
            CryptoCurve::Secp256k1 => MockKeyPair::generate_secp256k1(seed),
            CryptoCurve::Secp256r1 => MockKeyPair::generate_secp256r1(seed),
        }
    }

    /// Generates a deterministic Ed25519 keypair from a numeric seed.
    pub fn generate_ed25519_keypair(&self, seed: u64) -> MockKeyPair {
        MockKeyPair::generate_ed25519(seed)
    }

    /// Generates a deterministic Secp256k1 keypair from a numeric seed.
    pub fn generate_secp256k1_keypair(&self, seed: u64) -> MockKeyPair {
        MockKeyPair::generate_secp256k1(seed)
    }

    /// Generates a deterministic Secp256r1 keypair from a numeric seed.
    pub fn generate_secp256r1_keypair(&self, seed: u64) -> MockKeyPair {
        MockKeyPair::generate_secp256r1(seed)
    }
}

/// The outcome of a [`MockEnv::simulate_failing_call`] invocation.
///
/// Holds whether the call panicked (reverted) and any captured panic message.
#[derive(Debug)]
pub struct FailedCallResult {
    failed: bool,
    message: Option<String>,
}

impl FailedCallResult {
    /// Returns `true` if the call panicked (i.e., reverted).
    pub fn did_fail(&self) -> bool {
        self.failed
    }

    /// Returns the panic message, if any was captured.
    ///
    /// Soroban host panics are typically `&str` or `String` payloads.  Returns
    /// `None` when the call succeeded or the payload type was not recognisable.
    pub fn panic_message(&self) -> Option<&str> {
        self.message.as_deref()
    }
}

/// RAII guard that clears mock auth when dropped.
///
/// Obtained via [`MockEnv::mock_all_auths_scoped`]. Auth bypass is active for
/// as long as this value is alive; dropping it calls `mock_auths(&[])` on the
/// underlying environment, restoring the requirement for real auth on all
/// subsequent calls.
///
/// # Example
/// ```rust,ignore
/// {
///     let _guard = env.mock_all_auths_scoped();
///     contract.step_one();
///     contract.step_two();
/// } // _guard dropped — auth required again
/// ```
pub struct MockAuthGuard {
    env: Env,
}

impl Drop for MockAuthGuard {
    fn drop(&mut self) {
        self.env.mock_auths(&[]);
    }
}

impl Default for MockEnv {
    fn default() -> Self {
        Self {
            inner: Env::default(),
            accounts: Rc::new(RefCell::new(HashMap::new())),
            contract_ids: Rc::new(RefCell::new(HashMap::new())),
            tokens: Rc::new(RefCell::new(HashMap::new())),
            xlm_token_address: Rc::new(RefCell::new(None)),
            prng_state: Rc::new(RefCell::new([0; 32])),
            track_costs: false,
            crypto_registry: Rc::new(RefCell::new(MockCryptoRegistry::new())),
            checkpoints: Rc::new(RefCell::new(CheckpointStack::new())),
        }
    }
}

impl std::fmt::Debug for MockEnv {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("MockEnv")
            .field(
                "accounts",
                &self
                    .accounts
                    .borrow()
                    .keys()
                    .cloned()
                    .collect::<std::vec::Vec<_>>(),
            )
            .field(
                "contract_ids",
                &self
                    .contract_ids
                    .borrow()
                    .keys()
                    .cloned()
                    .collect::<std::vec::Vec<_>>(),
            )
            .field("track_costs", &self.track_costs)
            .finish_non_exhaustive()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    // Ensure MockEnv does NOT implement Send or Sync.
    static_assertions::assert_not_impl_any!(MockEnv: Send, Sync);

    #[test]
    fn seeded_prng_is_repeatable_and_advances() {
        let first = MockEnv::default();
        let second = MockEnv::default();
        let seed = [7u8; 32];
        first.set_prng_seed(seed);
        second.set_prng_seed(seed);

        let first_values = [first.next_prng_u64(), first.next_prng_u64()];
        let second_values = [second.next_prng_u64(), second.next_prng_u64()];

        assert_eq!(first_values, second_values);
        assert_ne!(first_values[0], first_values[1]);
    }

    #[test]
    fn different_prng_seeds_produce_different_streams() {
        let first = MockEnv::default();
        let second = MockEnv::default();
        first.set_prng_seed([1u8; 32]);
        second.set_prng_seed([2u8; 32]);
        assert_ne!(first.next_prng_u64(), second.next_prng_u64());
    }
}

/// Builder for constructing a `MockEnv` with custom configuration.
///
/// **Host-only:** See [`MockEnv`] for runtime requirements.
pub struct MockEnvBuilder {
    env: MockEnv,
    account_configs: Vec<(String, Stroops)>,
    token_configs: Vec<(String, u32)>,
}

impl MockEnvBuilder {
    fn new() -> Self {
        Self {
            env: MockEnv::default(),
            account_configs: Vec::new(),
            token_configs: Vec::new(),
        }
    }

    /// Set the ledger sequence number.
    pub fn at_sequence(self, sequence: u32) -> Self {
        let info = self.env.inner.ledger().get();
        self.env
            .inner
            .ledger()
            .set(soroban_sdk::testutils::LedgerInfo {
                sequence_number: sequence,
                timestamp: info.timestamp,
                protocol_version: info.protocol_version,
                base_reserve: info.base_reserve,
                network_id: info.network_id,
                min_temp_entry_ttl: info.min_temp_entry_ttl,
                min_persistent_entry_ttl: info.min_persistent_entry_ttl,
                max_entry_ttl: info.max_entry_ttl,
            });
        self
    }

    /// Set the ledger timestamp.
    pub fn at_timestamp(self, timestamp: u64) -> Self {
        let info = self.env.inner.ledger().get();
        self.env
            .inner
            .ledger()
            .set(soroban_sdk::testutils::LedgerInfo {
                sequence_number: info.sequence_number,
                timestamp,
                protocol_version: info.protocol_version,
                base_reserve: info.base_reserve,
                network_id: info.network_id,
                min_temp_entry_ttl: info.min_temp_entry_ttl,
                min_persistent_entry_ttl: info.min_persistent_entry_ttl,
                max_entry_ttl: info.max_entry_ttl,
            });
        self
    }

    /// Set the protocol version.
    pub fn with_protocol_version(self, version: u32) -> Self {
        let info = self.env.inner.ledger().get();
        self.env
            .inner
            .ledger()
            .set(soroban_sdk::testutils::LedgerInfo {
                sequence_number: info.sequence_number,
                timestamp: info.timestamp,
                protocol_version: version,
                base_reserve: info.base_reserve,
                network_id: info.network_id,
                min_temp_entry_ttl: info.min_temp_entry_ttl,
                min_persistent_entry_ttl: info.min_persistent_entry_ttl,
                max_entry_ttl: info.max_entry_ttl,
            });
        self
    }

    /// Register a contract with the environment.
    pub fn with_contract<C>(self) -> Self
    where
        C: soroban_sdk::testutils::ContractFunctionSet + Default + 'static,
    {
        let contract_id = C::default().register(&self.env.inner, None, ());
        self.env.register_contract::<C>(contract_id);
        self
    }

    /// Register a contract at a deterministic address.
    ///
    /// This deploys the contract to the underlying `soroban_sdk::Env` at the
    /// specified address and registers the type-to-address mapping so that
    /// callers can look up the address deterministically via
    /// `env.contract_id::<C>()`.
    pub fn with_contract_at<C>(self, id: &Address) -> Self
    where
        C: soroban_sdk::testutils::ContractFunctionSet + Default + 'static,
    {
        let contract_id = C::default().register(&self.env.inner, Some(id), ());
        self.env.register_contract::<C>(contract_id);
        self
    }

    /// Add a named account with XLM balance.
    pub fn with_account(mut self, name: &str, balance: Stroops) -> Self {
        self.account_configs.push((name.to_string(), balance));
        self
    }

    /// Add a named mock token with decimals.
    pub fn with_token(mut self, symbol: &str, decimals: u32) -> Self {
        self.token_configs.push((symbol.to_string(), decimals));
        self
    }

    /// Stops the environment writing a test snapshot file when it is dropped.
    ///
    /// The Soroban test host writes a JSON snapshot per `Env` by default, which
    /// is useful for a handful of hand-written tests but produces one file per
    /// generated case under
    /// [`#[crucible::quickcheck]`](crate::quickcheck). Property tests should
    /// build their environments with this.
    ///
    /// # Example
    ///
    /// ```rust,ignore
    /// #[crucible::quickcheck]
    /// fn some_property(amount: SorobanAmount) {
    ///     let env = MockEnv::builder().without_snapshots().build();
    ///     // ...
    /// }
    /// ```
    pub fn without_snapshots(mut self) -> Self {
        self.env.inner.set_config(soroban_sdk::testutils::EnvTestConfig {
            capture_snapshot_at_drop: false,
        });
        self
    }

    /// Enable cost tracking for instruction counting.
    pub fn track_costs(mut self) -> Self {
        self.env.track_costs = true;
        self
    }

    /// Build the `MockEnv`.
    pub fn build(self) -> MockEnv {
        for (name, balance) in self.account_configs {
            crate::account::AccountBuilder::new(&self.env)
                .name(&name)
                .fund_xlm(balance)
                .build();
        }
        for (symbol, decimals) in self.token_configs {
            let token = if symbol.eq_ignore_ascii_case("xlm") {
                MockToken::xlm(&self.env)
            } else {
                MockToken::new(&self.env, &symbol, decimals)
            };
            self.env.register_token(&symbol, token);
        }
        self.env
    }
}

/// Supported cryptographic curves for mock signature generation.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum CryptoCurve {
    Ed25519,
    Secp256k1,
    Secp256r1,
}

/// A deterministic keypair representation for mock crypto testing.
#[derive(Debug, Clone)]
pub struct MockKeyPair {
    pub curve: CryptoCurve,
    pub seed: u64,
    pub public_key_bytes: std::vec::Vec<u8>,
    pub secret_key_bytes: std::vec::Vec<u8>,
}

impl MockKeyPair {
    /// Generates a deterministic Ed25519 keypair from a numeric seed.
    pub fn generate_ed25519(seed: u64) -> Self {
        let mut seed_bytes = [0_u8; 32];
        seed_bytes[..8].copy_from_slice(&seed.to_le_bytes());
        for i in 8..32 {
            seed_bytes[i] = seed_bytes[i % 8].wrapping_add(i as u8);
        }
        let signing_key = Ed25519SigningKey::from_bytes(&seed_bytes);
        let verifying_key = signing_key.verifying_key();
        Self {
            curve: CryptoCurve::Ed25519,
            seed,
            public_key_bytes: verifying_key.to_bytes().to_vec(),
            secret_key_bytes: signing_key.to_bytes().to_vec(),
        }
    }

    /// Generates a deterministic Secp256k1 keypair from a numeric seed.
    pub fn generate_secp256k1(seed: u64) -> Self {
        let mut rng = ChaCha8Rng::seed_from_u64(seed);
        let signing_key = K256SigningKey::random(&mut rng);
        let verifying_key = K256VerifyingKey::from(&signing_key);
        let uncompressed = verifying_key.to_encoded_point(false);
        let pub_bytes = uncompressed.as_bytes()[1..].to_vec();
        Self {
            curve: CryptoCurve::Secp256k1,
            seed,
            public_key_bytes: pub_bytes,
            secret_key_bytes: signing_key.to_bytes().to_vec(),
        }
    }

    /// Generates a deterministic Secp256r1 keypair from a numeric seed.
    pub fn generate_secp256r1(seed: u64) -> Self {
        let mut rng = ChaCha8Rng::seed_from_u64(seed);
        let signing_key = P256SigningKey::random(&mut rng);
        let verifying_key = P256VerifyingKey::from(&signing_key);
        let uncompressed = verifying_key.to_encoded_point(false);
        let pub_bytes = uncompressed.as_bytes()[1..].to_vec();
        Self {
            curve: CryptoCurve::Secp256r1,
            seed,
            public_key_bytes: pub_bytes,
            secret_key_bytes: signing_key.to_bytes().to_vec(),
        }
    }

    /// Returns the public key as a 32-byte Soroban `BytesN<32>` (used for Ed25519).
    pub fn public_key_bytes32(&self, env: &Env) -> soroban_sdk::BytesN<32> {
        let array: [u8; 32] = self
            .public_key_bytes
            .as_slice()
            .try_into()
            .expect("public key must be 32 bytes");
        soroban_sdk::BytesN::from_array(env, &array)
    }

    /// Returns the public key as a 64-byte Soroban `BytesN<64>` (used for Secp256k1 / Secp256r1).
    pub fn public_key_bytes64(&self, env: &Env) -> soroban_sdk::BytesN<64> {
        let array: [u8; 64] = self
            .public_key_bytes
            .as_slice()
            .try_into()
            .expect("public key must be 64 bytes");
        soroban_sdk::BytesN::from_array(env, &array)
    }

    /// Returns the public key in SEC-1 uncompressed form as `BytesN<65>`.
    ///
    /// The stored `public_key_bytes` holds the 64-byte `X || Y` affine
    /// coordinates; SEC-1 prefixes those with the `0x04` uncompressed tag.
    /// This is the encoding [`soroban_sdk::crypto::Crypto::secp256r1_verify`]
    /// expects.
    ///
    /// # Panics
    ///
    /// Panics if the keypair is not on an ECDSA curve (i.e. for Ed25519).
    pub fn public_key_sec1_bytes65(&self, env: &Env) -> soroban_sdk::BytesN<65> {
        let coords: [u8; 64] = self
            .public_key_bytes
            .as_slice()
            .try_into()
            .expect("SEC-1 encoding requires a 64-byte ECDSA public key");
        let mut sec1 = [0_u8; 65];
        sec1[0] = 0x04;
        sec1[1..].copy_from_slice(&coords);
        soroban_sdk::BytesN::from_array(env, &sec1)
    }

    /// Signs a pre-computed 32-byte message digest with an ECDSA curve.
    ///
    /// Unlike [`sign`](Self::sign), which hashes the message itself, this signs
    /// the digest as-is. Use it when the digest was produced separately — for
    /// example by `env.crypto().sha256(..)` — so that the signature matches
    /// what the Soroban host verifies.
    ///
    /// # Panics
    ///
    /// Panics for Ed25519 keypairs, which sign whole messages rather than digests.
    pub fn sign_prehash(&self, env: &Env, message_digest: &[u8; 32]) -> soroban_sdk::BytesN<64> {
        let sig_bytes = match self.curve {
            CryptoCurve::Ed25519 => {
                panic!("Ed25519 signs whole messages; use `sign` instead of `sign_prehash`")
            }
            CryptoCurve::Secp256k1 => {
                let signing_key = K256SigningKey::from_slice(&self.secret_key_bytes).unwrap();
                let signature: k256::ecdsa::Signature = signing_key
                    .sign_prehash_recoverable(message_digest)
                    .expect("signing a 32-byte digest cannot fail")
                    .0;
                signature.to_bytes().to_vec()
            }
            CryptoCurve::Secp256r1 => {
                use p256::ecdsa::signature::hazmat::PrehashSigner as _;
                let signing_key = P256SigningKey::from_slice(&self.secret_key_bytes).unwrap();
                let signature: p256::ecdsa::Signature = signing_key
                    .sign_prehash(message_digest)
                    .expect("signing a 32-byte digest cannot fail");
                signature.to_bytes().to_vec()
            }
        };
        let array: [u8; 64] = sig_bytes.as_slice().try_into().unwrap();
        soroban_sdk::BytesN::from_array(env, &array)
    }

    /// Signs a 32-byte message digest and returns the signature together with
    /// the ECDSA recovery id, as required by
    /// [`soroban_sdk::crypto::Crypto::secp256k1_recover`].
    ///
    /// # Panics
    ///
    /// Panics unless the keypair is on the secp256k1 curve.
    pub fn sign_prehash_recoverable(
        &self,
        env: &Env,
        message_digest: &[u8; 32],
    ) -> (soroban_sdk::BytesN<64>, u32) {
        assert_eq!(
            self.curve,
            CryptoCurve::Secp256k1,
            "recoverable signatures are only defined for secp256k1"
        );
        let signing_key = K256SigningKey::from_slice(&self.secret_key_bytes).unwrap();
        let (signature, recovery_id) = signing_key
            .sign_prehash_recoverable(message_digest)
            .expect("signing a 32-byte digest cannot fail");
        let array: [u8; 64] = signature.to_bytes().as_slice().try_into().unwrap();
        (
            soroban_sdk::BytesN::from_array(env, &array),
            recovery_id.to_byte() as u32,
        )
    }

    /// Returns the public key as a Soroban `Bytes`.
    pub fn public_key_bytes(&self, env: &Env) -> soroban_sdk::Bytes {
        soroban_sdk::Bytes::from_slice(env, &self.public_key_bytes)
    }

    /// Signs a message slice and produces a valid 64-byte signature `BytesN<64>`.
    pub fn sign(&self, env: &Env, message: &[u8]) -> soroban_sdk::BytesN<64> {
        let sig_bytes = match self.curve {
            CryptoCurve::Ed25519 => {
                let sk_array: [u8; 32] = self.secret_key_bytes.as_slice().try_into().unwrap();
                let signing_key = Ed25519SigningKey::from_bytes(&sk_array);
                let signature = signing_key.sign(message);
                signature.to_bytes().to_vec()
            }
            CryptoCurve::Secp256k1 => {
                use k256::ecdsa::signature::Signer as _;
                let signing_key = K256SigningKey::from_slice(&self.secret_key_bytes).unwrap();
                let signature: k256::ecdsa::Signature = signing_key.sign(message);
                signature.to_bytes().to_vec()
            }
            CryptoCurve::Secp256r1 => {
                use p256::ecdsa::signature::Signer as _;
                let signing_key = P256SigningKey::from_slice(&self.secret_key_bytes).unwrap();
                let signature: p256::ecdsa::Signature = signing_key.sign(message);
                signature.to_bytes().to_vec()
            }
        };
        let array: [u8; 64] = sig_bytes.as_slice().try_into().unwrap();
        soroban_sdk::BytesN::from_array(env, &array)
    }

    /// Signs a Soroban `Bytes` payload and produces a valid 64-byte signature `BytesN<64>`.
    pub fn sign_bytes(&self, env: &Env, message: &soroban_sdk::Bytes) -> soroban_sdk::BytesN<64> {
        let mut buf = std::vec::Vec::with_capacity(message.len() as usize);
        for i in 0..message.len() {
            buf.push(message.get(i).unwrap());
        }
        self.sign(env, &buf)
    }

    /// Generates a corrupt (invalid) signature for negative path verification testing.
    pub fn corrupt_signature(&self, env: &Env, message: &[u8]) -> soroban_sdk::BytesN<64> {
        let mut valid_sig = self.sign(env, message).to_array();
        valid_sig[63] ^= 0xff;
        soroban_sdk::BytesN::from_array(env, &valid_sig)
    }

    /// Generates a corrupt signature from a Soroban `Bytes` payload.
    pub fn corrupt_signature_bytes(
        &self,
        env: &Env,
        message: &soroban_sdk::Bytes,
    ) -> soroban_sdk::BytesN<64> {
        let mut valid_sig = self.sign_bytes(env, message).to_array();
        valid_sig[63] ^= 0xff;
        soroban_sdk::BytesN::from_array(env, &valid_sig)
    }
}

/// Deterministic mock cryptographic registry managing pre-built keypairs and signature synthesis.
#[derive(Debug, Clone, Default)]
pub struct MockCryptoRegistry {
    keypairs: HashMap<String, MockKeyPair>,
}

impl MockCryptoRegistry {
    /// Creates a new empty `MockCryptoRegistry`.
    pub fn new() -> Self {
        Self::default()
    }

    /// Registers a named keypair.
    pub fn register(&mut self, name: impl Into<String>, keypair: MockKeyPair) {
        self.keypairs.insert(name.into(), keypair);
    }

    /// Retrieves a registered keypair by name.
    pub fn get(&self, name: &str) -> Option<&MockKeyPair> {
        self.keypairs.get(name)
    }

    /// Gets or generates an Ed25519 keypair by name.
    pub fn ed25519_keypair(&mut self, name: &str, seed: u64) -> &MockKeyPair {
        self.keypairs
            .entry(name.to_string())
            .or_insert_with(|| MockKeyPair::generate_ed25519(seed))
    }

    /// Gets or generates a Secp256k1 keypair by name.
    pub fn secp256k1_keypair(&mut self, name: &str, seed: u64) -> &MockKeyPair {
        self.keypairs
            .entry(name.to_string())
            .or_insert_with(|| MockKeyPair::generate_secp256k1(seed))
    }

    /// Gets or generates a Secp256r1 keypair by name.
    pub fn secp256r1_keypair(&mut self, name: &str, seed: u64) -> &MockKeyPair {
        self.keypairs
            .entry(name.to_string())
            .or_insert_with(|| MockKeyPair::generate_secp256r1(seed))
    }
}

#[cfg(test)]
mod extra_tests {
    use super::*;
    use soroban_sdk::testutils::Address as _;
    // Use the counter contract from examples to test
    use soroban_sdk::{contractimpl, Env};

    // A simple test contract
    #[soroban_sdk::contract]
    #[derive(Default)]
    struct TestContract;

    #[contractimpl]
    impl TestContract {
        pub fn initialize(env: Env, value: u32) {
            env.storage()
                .instance()
                .set(&soroban_sdk::symbol_short!("val"), &value);
        }

        pub fn get(env: Env) -> u32 {
            env.storage()
                .instance()
                .get(&soroban_sdk::symbol_short!("val"))
                .unwrap_or(0)
        }

        pub fn increment(env: Env) -> u32 {
            let val = Self::get(env.clone());
            let new_val = val + 1;
            env.storage()
                .instance()
                .set(&soroban_sdk::symbol_short!("val"), &new_val);
            new_val
        }
    }

    #[test]
    fn test_clone_shares_accounts() {
        let env = MockEnv::builder()
            .with_account("alice", Stroops::xlm(100))
            .build();
        let env2 = env.clone();

        let bob = Address::generate(&env.inner);
        env2.register_account("bob", bob);

        assert!(env.accounts.borrow().contains_key("bob"));
    }

    #[test]
    fn test_clone_shares_contract_ids() {
        let env = MockEnv::default();
        let env2 = env.clone();

        let addr = Address::generate(&env.inner);
        env2.register_contract::<MockEnv>(addr.clone());

        assert_eq!(env.contract_id::<MockEnv>(), addr);
    }

    #[test]
    fn test_clone_shares_xlm_token_address() {
        let env = MockEnv::default();
        let env2 = env.clone();

        let addr = Address::generate(&env.inner);
        env2.set_xlm_token_address(addr.clone());

        assert_eq!(env.xlm_token_address(), Some(addr));
    }

    #[test]
    fn test_clone_independent_track_costs() {
        let mut env = MockEnv::default();
        env.track_costs = true;
        let env2 = env.clone();

        assert!(env2.track_costs);

        env.track_costs = false;
        assert!(env2.track_costs);
    }

    #[test]
    fn test_fork_creates_independent_accounts() {
        let env = MockEnv::builder()
            .with_account("alice", Stroops::xlm(100))
            .build();
        let forked = env.fork();

        assert!(forked.accounts.borrow().contains_key("alice"));

        let bob = Address::generate(&env.inner);
        forked.register_account("bob", bob);

        assert!(!env.accounts.borrow().contains_key("bob"));
    }

    #[test]
    fn test_fork_creates_independent_contract_ids() {
        let env = MockEnv::default();
        let addr = Address::generate(&env.inner);
        env.register_contract::<MockEnv>(addr.clone());

        let forked = env.fork();

        assert_eq!(forked.contract_id::<MockEnv>(), addr);

        let addr2 = Address::generate(&env.inner);
        forked.register_contract::<MockEnv>(addr2.clone());

        assert_ne!(env.contract_id::<MockEnv>(), addr2);
    }

    #[test]
    fn test_fork_creates_independent_xlm_token_address() {
        let env = MockEnv::default();
        let addr = Address::generate(&env.inner);
        env.set_xlm_token_address(addr.clone());

        let forked = env.fork();

        assert_eq!(forked.xlm_token_address(), Some(addr));

        forked.set_xlm_token_address(Address::generate(&env.inner));
        assert_ne!(
            forked.xlm_token_address(),
            env.xlm_token_address(),
            "forked and original xlm token addresses should differ"
        );
    }

    #[test]
    fn test_clone_and_fork_work_with_account_handle() {
        let env = MockEnv::builder()
            .with_account("alice", Stroops::xlm(100))
            .build();

        let alice = env.account("alice");
        alice.xlm_balance();
    }

    #[test]
    fn test_clone_shared_accounts_visible_through_account_handle() {
        let env1 = MockEnv::builder()
            .with_account("alice", Stroops::xlm(100))
            .build();
        let env2 = env1.clone();

        let alice = env2.account("alice");
        assert_eq!(alice.xlm_balance(), Stroops::xlm(100).as_stroops());
    }

    #[test]
    fn test_with_contract_at_deploys_real_contract() {
        let env = MockEnv::default();
        let specific_addr = Address::generate(&env.inner);
        TestContract::default().register(&env.inner, Some(&specific_addr), ());
        env.register_contract::<TestContract>(specific_addr.clone());

        assert_eq!(env.contract_id::<TestContract>(), specific_addr);

        // Create a client and test that we can call the contract
        let client = TestContractClient::new(&env.inner, &specific_addr);
        client.initialize(&42);
        assert_eq!(client.get(), 42);
        assert_eq!(client.increment(), 43);
    }

    #[test]
    fn test_with_contract_at_deterministic_address() {
        let env1 = MockEnv::default();
        let specific_addr = Address::generate(&env1.inner);
        TestContract::default().register(&env1.inner, Some(&specific_addr), ());
        env1.register_contract::<TestContract>(specific_addr.clone());
        assert_eq!(env1.contract_id::<TestContract>(), specific_addr);

        let env2 = MockEnv::default();
        let other_addr = Address::generate(&env2.inner);
        TestContract::default().register(&env2.inner, Some(&other_addr), ());
        env2.register_contract::<TestContract>(other_addr.clone());
        assert_eq!(env2.contract_id::<TestContract>(), other_addr);
    }

    #[test]
    fn test_multiple_contracts_distinct_addresses() {
        let env = MockEnv::default();
        let addr1 = Address::generate(&env.inner);
        let addr2 = Address::generate(&env.inner);

        TestContract::default().register(&env.inner, Some(&addr1), ());
        env.register_contract::<TestContract>(addr1.clone());

        // Deploy a second contract at addr2 using register
        TestContract::default().register(&env.inner, Some(&addr2), ());
        env.register_contract::<TestContract>(addr2.clone());

        assert_eq!(env.contract_id::<TestContract>(), addr2);
        assert_ne!(addr1, addr2);
    }

    #[test]
    fn test_contract_state_persists() {
        let env = MockEnv::default();
        let addr = Address::generate(&env.inner);
        TestContract::default().register(&env.inner, Some(&addr), ());
        env.register_contract::<TestContract>(addr.clone());

        let client = TestContractClient::new(&env.inner, &addr);
        client.initialize(&10);
        assert_eq!(client.increment(), 11);
        assert_eq!(client.increment(), 12);
        assert_eq!(client.get(), 12);
    }
}

#[cfg(test)]
mod time_advance_tests {
    use super::*;
    use crate::time::{add_months, datetime_to_unix};

    const JAN_31_2024: u64 = 1_706_704_245;
    const MAR_15_2024: u64 = 1_710_489_600;

    #[test]
    fn advance_time_by_months_updates_ledger() {
        let env = MockEnv::builder().at_timestamp(JAN_31_2024).build();
        env.advance_time_by_months(1);
        assert_eq!(env.timestamp(), datetime_to_unix(2024, 2, 29, 12, 30, 45));
    }

    #[test]
    fn advance_time_by_years_updates_ledger() {
        let env = MockEnv::builder().at_timestamp(MAR_15_2024).build();
        env.advance_time_by_years(1);
        assert_eq!(env.timestamp(), datetime_to_unix(2025, 3, 15, 8, 0, 0));
    }

    #[test]
    fn advance_time_by_months_chains_with_existing_timestamp() {
        let env = MockEnv::builder().at_timestamp(MAR_15_2024).build();
        env.advance_time_by_months(6);
        assert_eq!(env.timestamp(), add_months(MAR_15_2024, 6));
    }

    #[test]
    fn advance_time_zero_duration_is_noop() {
        let env = MockEnv::builder()
            .at_timestamp(1_700_000_000)
            .build();
        
        // Verify initial state
        assert_eq!(env.timestamp(), 1_700_000_000);
        
        // Advance by zero using Duration::ZERO equivalent
        env.advance_time(Duration::days(0));
        
        // Timestamp should remain unchanged
        assert_eq!(env.timestamp(), 1_700_000_000);
        
        // Also test with Duration::seconds(0)
        env.advance_time(Duration::seconds(0));
        assert_eq!(env.timestamp(), 1_700_000_000);
    }

    #[test]
    fn advance_sequence_zero_is_noop() {
        let env = MockEnv::builder().build();
        let initial_seq = env.ledger_sequence();
        
        env.advance_sequence(0);
        
        assert_eq!(env.ledger_sequence(), initial_seq);
    }
}

#[cfg(test)]
mod auth_scope_tests {
    use super::*;

    #[test]
    fn with_mock_all_auths_clears_after_block() {
        let env = MockEnv::default();
        env.with_mock_all_auths(|| {
            // no contract calls needed — just verify the side-effect
        });
        // mock_auths(&[]) must have been called; auths() should be empty
        let auths = env.inner().auths();
        assert!(
            auths.is_empty(),
            "auth bypass must be cleared after with_mock_all_auths"
        );
    }

    #[test]
    fn scoped_guard_clears_on_drop() {
        let env = MockEnv::default();
        {
            let _guard = env.mock_all_auths_scoped();
        } // dropped here
        let auths = env.inner().auths();
        assert!(
            auths.is_empty(),
            "auth bypass must be cleared after guard drop"
        );
    }

    #[test]
    fn successive_scopes_do_not_interfere() {
        let env = MockEnv::default();
        {
            let _g = env.mock_all_auths_scoped();
        }
        {
            let _g = env.mock_all_auths_scoped();
        }
        assert!(env.inner().auths().is_empty());
    }

    #[soroban_sdk::contract]
    struct DummyProtectedContract;

    #[soroban_sdk::contractimpl]
    impl DummyProtectedContract {
        pub fn protected_action(_env: soroban_sdk::Env, user: soroban_sdk::Address) -> u32 {
            user.require_auth();
            100
        }
    }

    #[test]
    fn protected_call_valid_auth_succeeds() {
        use soroban_sdk::testutils::Address as _;
        let env = MockEnv::default();
        let contract_id = env.inner().register(DummyProtectedContract, ());
        let client = DummyProtectedContractClient::new(env.inner(), &contract_id);
        let alice = soroban_sdk::Address::generate(env.inner());

        // 1. Global mock auth pattern
        env.mock_all_auths();
        assert_eq!(client.protected_action(&alice), 100);

        // 2. Specific mock auth pattern
        env.mock_auths(&[soroban_sdk::testutils::MockAuth {
            address: &alice,
            invoke: &soroban_sdk::testutils::MockAuthInvoke {
                contract: &contract_id,
                fn_name: "protected_action",
                args: (alice.clone(),).into_val(env.inner()),
                sub_invokes: &[],
            },
        }]);
        assert_eq!(client.protected_action(&alice), 100);
    }

    #[test]
    #[should_panic]
    fn protected_call_missing_auth_panics() {
        use soroban_sdk::testutils::Address as _;
        let env = MockEnv::default();
        let contract_id = env.inner().register(DummyProtectedContract, ());
        let client = DummyProtectedContractClient::new(env.inner(), &contract_id);
        let alice = soroban_sdk::Address::generate(env.inner());

        // Clear mock authorizations so require_auth fails
        env.mock_auths(&[]);
        client.protected_action(&alice);
    }

    #[test]
    #[should_panic]
    fn protected_call_wrong_signer_panics() {
        use soroban_sdk::testutils::Address as _;
        let env = MockEnv::default();
        let contract_id = env.inner().register(DummyProtectedContract, ());
        let client = DummyProtectedContractClient::new(env.inner(), &contract_id);
        let alice = soroban_sdk::Address::generate(env.inner());
        let bob = soroban_sdk::Address::generate(env.inner());

        // Provide authorization for bob when alice is the required caller argument
        env.mock_auths(&[soroban_sdk::testutils::MockAuth {
            address: &bob,
            invoke: &soroban_sdk::testutils::MockAuthInvoke {
                contract: &contract_id,
                fn_name: "protected_action",
                args: (alice.clone(),).into_val(env.inner()),
                sub_invokes: &[],
            },
        }]);
        client.protected_action(&alice);
    }
}

#[cfg(test)]
mod missing_lookup_tests {
    use super::*;
    use soroban_sdk::testutils::Address as _;

    #[test]
    #[should_panic(
        expected = "Account 'missing' not found. Available accounts: [admin, alice, bob]. Ensure it was registered via MockEnvBuilder or AccountBuilder."
    )]
    fn missing_account_shows_available() {
        let env = MockEnv::builder()
            .with_account("admin", Stroops::xlm(10))
            .with_account("alice", Stroops::xlm(10))
            .with_account("bob", Stroops::xlm(10))
            .build();
        env.account("missing");
    }

    #[test]
    #[should_panic(
        expected = "Contract 'alloc::string::String' not registered. Available contracts: [crucible::env::MockEnv]"
    )]
    fn missing_contract_shows_available() {
        let env = MockEnv::default();
        let addr = Address::generate(&env.inner);
        env.register_contract::<MockEnv>(addr);
        env.contract_id::<String>();
    }

    #[test]
    fn test_with_token_and_token_accessors() {
        let env = MockEnv::builder()
            .with_token("USDC", 6)
            .with_token("XLM", 7)
            .build();

        let usdc = env.token("USDC");
        assert_eq!(usdc.decimals(), 6);

        let usdc_opt = env.token_opt("USDC");
        assert!(usdc_opt.is_some());
        assert_eq!(usdc_opt.unwrap().decimals(), 6);

        assert!(env.token_opt("UNKNOWN").is_none());

        let xlm = env.token("XLM");
        assert_eq!(xlm.decimals(), 7);

        // Manually created MockToken works alongside registered tokens
        let manual_usdc = MockToken::new(&env, "USDC_MANUAL", 6);
        assert_eq!(manual_usdc.decimals(), 6);
    }

    #[test]
    #[should_panic(
        expected = "Token 'MISSING' not found in MockEnv. Available tokens: [USDC, XLM]. Ensure it was registered via MockEnvBuilder."
    )]
    fn test_missing_token_panics() {
        let env = MockEnv::builder()
            .with_token("USDC", 6)
            .with_token("XLM", 7)
            .build();
        env.token("MISSING");
    }
}

#[cfg(test)]
mod protocol_version_tests {
    use super::*;

    #[test]
    fn test_protocol_version_getter_returns_default() {
        let env = MockEnv::default();
        assert_eq!(env.protocol_version(), 26);
    }

    #[test]
    fn test_with_protocol_version_sets_ledger() {
        let env = MockEnv::builder()
            .with_protocol_version(26)
            .build();
        assert_eq!(env.protocol_version(), 26);
    }

    #[test]
    fn test_protocol_version_enum_values() {
        assert_eq!(ProtocolVersion::V20.value(), 20);
        assert_eq!(ProtocolVersion::V21.value(), 21);
        assert_eq!(ProtocolVersion::V22.value(), 22);
    }

    #[test]
    fn test_protocol_version_all_versions() {
        let versions: Vec<u32> = ProtocolVersion::all().map(|v| v.value()).collect();
        assert_eq!(versions, vec![20, 21, 22]);
    }

    #[test]
    fn test_protocol_version_max_supported() {
        assert_eq!(ProtocolVersion::max_supported(), ProtocolVersion::V22);
    }

    #[test]
    fn test_protocol_version_supports_host_function() {
        let v20 = ProtocolVersion::V20;
        let v21 = ProtocolVersion::V21;
        let v22 = ProtocolVersion::V22;

        assert!(v20.supports_host_function("v1_low_level_operations"));
        assert!(!v20.supports_host_function("v2_low_level_operations"));
        assert!(!v20.supports_host_function("v3_low_level_operations"));

        assert!(v21.supports_host_function("v1_low_level_operations"));
        assert!(v21.supports_host_function("v2_low_level_operations"));
        assert!(!v21.supports_host_function("v3_low_level_operations"));

        assert!(v22.supports_host_function("v1_low_level_operations"));
        assert!(v22.supports_host_function("v2_low_level_operations"));
        assert!(v22.supports_host_function("v3_low_level_operations"));
    }

    #[test]
    fn test_protocol_version_from_u32() {
        assert_eq!(ProtocolVersion::from(20), ProtocolVersion::V20);
        assert_eq!(ProtocolVersion::from(21), ProtocolVersion::V21);
        assert_eq!(ProtocolVersion::from(22), ProtocolVersion::V22);
    }

    #[test]
    #[should_panic(expected = "Unsupported protocol version: 99")]
    fn test_protocol_version_from_invalid_u32_panics() {
        let _ = ProtocolVersion::from(99);
    }

    #[test]
    fn test_protocol_version_display() {
        assert_eq!(format!("{}", ProtocolVersion::V20), "Protocol 20");
        assert_eq!(format!("{}", ProtocolVersion::V21), "Protocol 21");
        assert_eq!(format!("{}", ProtocolVersion::V22), "Protocol 22");
    }
}

// Location: contracts/crucible/src/env.rs // Production requirement: Zero-Knowledge Proof & BN254/BLS12-381 Verifier Mock Harness
#[cfg(test)]
mod zk_pairing_harness_tests {
    use super::*;
    use crate::zk::{self, encode_scalar, G1, G2, PairingCurve};
    use soroban_sdk::{contract, contractimpl, Bytes};

    #[test]
    fn bn254_host_mocks_add_and_mul() {
        let env = MockEnv::default();
        let p = G1::generator();
        let q = env.bn254_g1_mul(p, 3);
        let sum = env.bn254_g1_add(p, q);
        assert_eq!(sum, zk::g1_add(p, q));

        let g2 = G2::generator();
        let g2s = env.bn254_g2_mul(g2, 5);
        assert_eq!(env.bn254_g2_add(g2, g2s), zk::g2_add(g2, g2s));
    }

    #[test]
    fn bls12_381_host_mocks_pairing_check() {
        let env = MockEnv::default();
        let p = env.bls12_381_g1_mul(G1::generator(), 2);
        let q = env.bls12_381_g2_mul(G2::generator(), 3);
        // e(2G, 3H) * e(-6G, H) = 1
        let neg = zk::g1_neg(env.bls12_381_g1_mul(G1::generator(), 6));
        assert!(env.bls12_381_pairing_check(&[(p, q), (neg, G2::generator())]));
    }

    #[test]
    fn generate_and_verify_valid_groth16_on_bn254() {
        let env = MockEnv::default();
        let vk = env.groth16_verifying_key(PairingCurve::Bn254, 2);
        let inputs = [4u64, 9];
        let proof = env.generate_groth16_proof(&vk, &inputs);
        assert!(env.verify_groth16(&vk, &proof, &inputs));
    }

    #[test]
    fn negative_groth16_proof_is_rejected() {
        let env = MockEnv::default();
        let vk = env.groth16_verifying_key(PairingCurve::Bls12_381, 1);
        let inputs = [12u64];
        let proof = env.generate_groth16_proof(&vk, &inputs);
        let bad = env.tamper_groth16_proof(proof);
        assert!(!env.verify_groth16(&vk, &bad, &inputs));
        assert!(!env.verify_groth16(&vk, &proof, &[99]));
    }

    #[test]
    fn plonk_valid_and_negative_via_mock_env() {
        let env = MockEnv::default();
        let proof = env.generate_plonk_proof(PairingCurve::Bn254, 7);
        assert!(env.verify_plonk(PairingCurve::Bn254, &proof, 7));
        assert!(!env.verify_plonk(
            PairingCurve::Bn254,
            &zk::tamper_plonk_proof(proof),
            7
        ));
    }

    /// On-chain Groth16 verifier that consumes harness-encoded proof bytes
    /// for a single public input (IC[0] + pub·IC[1]).
    #[contract]
    struct OnChainGroth16Verifier;

    #[contractimpl]
    impl OnChainGroth16Verifier {
        pub fn verify_proof(
            env: soroban_sdk::Env,
            a: Bytes,
            b: Bytes,
            c: Bytes,
            alpha: Bytes,
            beta: Bytes,
            gamma: Bytes,
            delta: Bytes,
            ic0: Bytes,
            ic1: Bytes,
            public_input: Bytes,
        ) -> bool {
            let _ = env;
            groth16_pairing_equation(
                &a,
                &b,
                &c,
                &alpha,
                &beta,
                &gamma,
                &delta,
                &ic0,
                &ic1,
                &public_input,
            )
        }
    }

    fn groth16_pairing_equation(
        a: &Bytes,
        b: &Bytes,
        c: &Bytes,
        alpha: &Bytes,
        beta: &Bytes,
        gamma: &Bytes,
        delta: &Bytes,
        ic0: &Bytes,
        ic1: &Bytes,
        public_input: &Bytes,
    ) -> bool {
        let a_x = read_u64(a, 0);
        let b_x0 = read_u64(b, 0);
        let c_x = read_u64(c, 0);
        let alpha_x = read_u64(alpha, 0);
        let beta_x0 = read_u64(beta, 0);
        let gamma_x0 = read_u64(gamma, 0);
        let delta_x0 = read_u64(delta, 0);
        let input = read_u64(public_input, 0);
        let l_x = read_u64(ic0, 0).wrapping_add(input.wrapping_mul(read_u64(ic1, 0)));

        let lhs = a_x.wrapping_mul(b_x0);
        let rhs = alpha_x
            .wrapping_mul(beta_x0)
            .wrapping_add(l_x.wrapping_mul(gamma_x0))
            .wrapping_add(c_x.wrapping_mul(delta_x0));
        lhs == rhs
    }

    fn read_u64(bytes: &Bytes, offset: u32) -> u64 {
        let mut out = [0u8; 8];
        for i in 0..8u32 {
            out[i as usize] = bytes.get(offset + i).unwrap_or(0);
        }
        u64::from_le_bytes(out)
    }

    #[test]
    fn groth16_proof_verifies_on_chain() {
        let env = MockEnv::default();
        env.mock_all_auths();
        let inner = env.inner();
        let contract_id = inner.register(OnChainGroth16Verifier, ());
        let client = OnChainGroth16VerifierClient::new(inner, &contract_id);

        let vk = env.groth16_verifying_key(PairingCurve::Bn254, 1);
        let inputs = [5u64];
        let proof = env.generate_groth16_proof(&vk, &inputs);

        let ok = client.verify_proof(
            &proof.a.to_bytes(inner, proof.curve),
            &proof.b.to_bytes(inner, proof.curve),
            &proof.c.to_bytes(inner, proof.curve),
            &vk.alpha_g1.to_bytes(inner, vk.curve),
            &vk.beta_g2.to_bytes(inner, vk.curve),
            &vk.gamma_g2.to_bytes(inner, vk.curve),
            &vk.delta_g2.to_bytes(inner, vk.curve),
            &vk.ic[0].to_bytes(inner, vk.curve),
            &vk.ic[1].to_bytes(inner, vk.curve),
            &encode_scalar(inner, inputs[0]),
        );
        assert!(ok, "valid Groth16 proof must verify on-chain");
    }

    #[test]
    fn groth16_negative_proof_fails_on_chain() {
        let env = MockEnv::default();
        env.mock_all_auths();
        let inner = env.inner();
        let contract_id = inner.register(OnChainGroth16Verifier, ());
        let client = OnChainGroth16VerifierClient::new(inner, &contract_id);

        let vk = env.groth16_verifying_key(PairingCurve::Bls12_381, 1);
        let inputs = [5u64];
        let proof = env.tamper_groth16_proof(env.generate_groth16_proof(&vk, &inputs));

        let ok = client.verify_proof(
            &proof.a.to_bytes(inner, proof.curve),
            &proof.b.to_bytes(inner, proof.curve),
            &proof.c.to_bytes(inner, proof.curve),
            &vk.alpha_g1.to_bytes(inner, vk.curve),
            &vk.beta_g2.to_bytes(inner, vk.curve),
            &vk.gamma_g2.to_bytes(inner, vk.curve),
            &vk.delta_g2.to_bytes(inner, vk.curve),
            &vk.ic[0].to_bytes(inner, vk.curve),
            &vk.ic[1].to_bytes(inner, vk.curve),
            &encode_scalar(inner, inputs[0]),
        );
        assert!(!ok, "tampered Groth16 proof must fail on-chain");
    }
}
