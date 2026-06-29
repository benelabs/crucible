//! Mock environment for Soroban contract testing.
//!
//! Provides `MockEnv` - a wrapper around `soroban_sdk::Env` with convenient
//! helpers for testing, and `MockEnvBuilder` for fluent environment construction.

use crate::account::AccountHandle;
use crate::cost::CostReport;
use crate::sim::SimulatedTx;
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
    pub fn from(amount: i128) -> Self {
        Self { amount }
    }

    /// Creates stroops from XLM (1 XLM = 10,000,000 stroops).
    pub fn xlm(xlm: i128) -> Self {
        Self {
            amount: xlm * 10_000_000,
        }
    }

    /// Creates stroops with fractional XLM (e.g., 0.5 XLM).
    pub fn xlm_frac(xlm: f64) -> Self {
        Self {
            amount: (xlm * 10_000_000.0) as i128,
        }
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

/// A wrapper around the Soroban test environment with additional helpers.
#[derive(Clone)]
pub struct MockEnv {
    inner: Env,
    accounts: Rc<RefCell<HashMap<String, Address>>>,
    contract_ids: Rc<RefCell<HashMap<String, Address>>>,
    xlm_token_address: Rc<RefCell<Option<Address>>>,
    track_costs: bool,
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
    /// This causes all `require_auth()` calls to succeed without valid signatures.
    pub fn mock_all_auths(&self) {
        self.inner.mock_all_auths();
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
    pub fn events_from_contract(&self, contract_id: &Address) -> SorobanVec<(Address, SorobanVec<Val>, Val)> {
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
    pub fn events_from_contracts(&self, contract_ids: &[&Address]) -> SorobanVec<(Address, SorobanVec<Val>, Val)> {
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
            let xdr::ContractEventBody::V0(body) = &event.body;
            let event_topics: SorobanVec<Val> = body.topics.clone().into_val(&self.inner);
            if event_topics.len() < filter_topics.len() {
                continue;
            }
            let mut matches = true;
            for (i, filter_topic) in filter_topics.iter().enumerate() {
                if format!("{:?}", filter_topic)
                    != format!("{:?}", event_topics.get(i as u32).unwrap())
                {
                    matches = false;
                    break;
                }
            }
            if matches {
                let sc_addr = ScAddress::Contract(event.contract_id.as_ref().unwrap().clone());
                let contract_id = Address::from_val(&self.inner, &sc_addr);
                let data: Val = body.data.clone().into_val(&self.inner);
                matching.push_back((contract_id, event_topics, data));
            }
        }
        matching
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
        let _ = f();
        CostReport::new(budget.cpu_instruction_cost(), budget.memory_bytes_cost())
    }

    /// Simulate a contract call and return a dry-run result.
    pub fn simulate<F, T>(&self, f: F) -> SimulatedTx<T>
    where
        F: Fn() -> T + 'static,
        T: 'static,
    {
        let mut budget = self.inner.budget();
        budget.reset_default();

        self.inner.mock_auths(&[]);
        self.inner.mock_all_auths();

        let result = f();

        let instructions = budget.cpu_instruction_cost();
        let auths = self.inner.auths().iter().map(|(a, _)| a.clone()).collect();

        SimulatedTx::new(
            (instructions / 100) as i64,
            instructions,
            auths,
            true,
            Some(result),
            Some(Box::new(f)),
        )
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

/// Builder for constructing a `MockEnv` with custom configuration.
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
