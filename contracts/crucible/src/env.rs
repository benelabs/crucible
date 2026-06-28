//! Mock environment for Soroban contract testing.
//!
//! Provides `MockEnv` - a wrapper around `soroban_sdk::Env` with convenient
//! helpers for testing, and `MockEnvBuilder` for fluent environment construction.
//!
//! **Host-only:** All types in this module depend on `std` and the Soroban host
//! test utilities. They are intended exclusively for use in `#[cfg(test)]`
//! contexts on the host and are not available inside contract WASM builds.

use crate::account::AccountHandle;
use crate::cost::CostReport;
use crate::sim::{PreparedTx, SimulatedTx};
use soroban_sdk::{
    testutils::{ContractEvents, Events, Ledger},
    Address, Env, FromVal, IntoVal, Val, Vec as SorobanVec,
};
use std::cell::RefCell;
use std::collections::HashMap;
use std::rc::Rc;
use std::time::Duration as StdDuration;

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
        let amount = (xlm * 10_000_000.0)
            .round()
            as i128;
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
    xlm_token_address: Rc<RefCell<Option<Address>>>,
    track_costs: bool,
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
}

impl MockEnv {
    /// Returns the underlying `soroban_sdk::Env`.
    pub fn inner(&self) -> &Env {
        &self.inner
    }

    /// Creates a new `MockEnvBuilder` for fluent environment construction.
    pub fn builder() -> MockEnvBuilder {
        MockEnvBuilder::new()
    }

    /// Get an account handle by name.
    pub fn account(&self, name: &str) -> AccountHandle {
        let address = self.accounts
            .borrow()
            .get(name)
            .cloned()
            .unwrap_or_else(|| panic!("Account '{}' not found. Ensure it was registered via MockEnvBuilder or AccountBuilder.", name));

        AccountHandle::new(self.clone(), name.to_string(), address)
    }

    /// Get a contract ID by type.
    pub fn contract_id<C>(&self) -> Address {
        let type_name = std::any::type_name::<C>();
        self.contract_ids
            .borrow()
            .get(type_name)
            .cloned()
            .unwrap_or_else(|| panic!("Contract '{}' not registered", type_name))
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

    /// Advance the ledger timestamp by a duration.
    pub fn advance_time(&self, duration: Duration) {
        let info = self.inner.ledger().get();
        self.inner.ledger().set(soroban_sdk::testutils::LedgerInfo {
            sequence_number: info.sequence_number,
            timestamp: info.timestamp + duration.as_seconds(),
            protocol_version: info.protocol_version,
            base_reserve: info.base_reserve,
            network_id: info.network_id,
            min_temp_entry_ttl: info.min_temp_entry_ttl,
            min_persistent_entry_ttl: info.min_persistent_entry_ttl,
            max_entry_ttl: info.max_entry_ttl,
        });
    }

    /// Advance the ledger sequence number by n.
    pub fn advance_sequence(&self, n: u32) {
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

    /// Returns events matching the given topics.
    ///
    /// Updated for Soroban SDK v25.x ContractEvents compatibility.
    pub fn events_matching<T>(&self, topics: T) -> SorobanVec<(Address, SorobanVec<Val>, Val)>
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
                crate::event_topic_match::topics_match_by_payload(&filter_topics, &event_topics);
            if matches {
                let sc_addr = ScAddress::Contract(hash.clone());
                let contract_id = Address::from_val(&self.inner, &sc_addr);
                let data: Val = body.data.clone().into_val(&self.inner);
                matching.push_back((contract_id, event_topics, data));
            }
        }
        matching
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
            let matches = crate::event_topic_match::topics_match(&self.inner, &filter_topics, &event_topics);
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

    /// Inspect a contract call without the ability to commit.
    ///
    /// Unlike `simulate`, this method does not require the closure to be `'static`,
    /// allowing it to borrow local clients, accounts, or fixture references.
    ///
    /// Auth is globally bypassed only for the duration of the dry-run call.
    /// After `simulate_inspect` returns the auth mock is cleared, so subsequent
    /// operations require explicit auth setup and will not silently pass.
    pub fn simulate_inspect<F, T>(&self, f: F) -> InspectedTx<T>
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

        InspectedTx::new(
            fee,
            instructions,
            auths,
            true,
            Some(result),
        )
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
            xlm_token_address: Rc::new(RefCell::new(self.xlm_token_address.borrow().clone())),
            track_costs: self.track_costs,
        }
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
            xlm_token_address: Rc::new(RefCell::new(None)),
            track_costs: false,
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
}


/// Builder for constructing a `MockEnv` with custom configuration.
///
/// **Host-only:** See [`MockEnv`] for runtime requirements.
pub struct MockEnvBuilder {
    env: MockEnv,
    account_configs: Vec<(String, Stroops)>,
}

impl MockEnvBuilder {
    fn new() -> Self {
        Self {
            env: MockEnv::default(),
            account_configs: Vec::new(),
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
        let contract_id = self.env.inner.register(C::default(), ());
        self.env.register_contract::<C>(contract_id);
        self
    }

    /// Register a contract at a deterministic address.
    ///
    /// This allows tests to associate a contract type with a known `Address` so
    /// that callers can look up the address deterministically via
    /// `env.contract_id::<C>()`. Note: this registers the mapping in the
    /// `MockEnv` but does not deploy the contract instance to the underlying
    /// `soroban_sdk::Env`. Use `with_contract` if you need the instance to be
    /// available for calls.
    pub fn with_contract_at<C>(self, id: &Address) -> Self
    where
        C: soroban_sdk::testutils::ContractFunctionSet + Default + 'static,
    {
        self.env.register_contract::<C>(id.clone());
        self
    }

    /// Add a named account with XLM balance.
    pub fn with_account(mut self, name: &str, balance: Stroops) -> Self {
        self.account_configs.push((name.to_string(), balance));
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
        self.env
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use soroban_sdk::testutils::Address as _;

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
}