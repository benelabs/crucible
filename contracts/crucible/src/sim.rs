//! Simulated transaction dry-runs, fee estimation, and reentrancy probes.
//!
//! **Host-only:** [`SimulatedTx`] is constructed by [`MockEnv::simulate`] and
//! depends on the Soroban host test utilities. It is intended exclusively for
//! use in `#[cfg(test)]` contexts on the host.
//!
//! [`MockEnv::simulate`]: crate::env::MockEnv::simulate
/// without committing the state changes.
use soroban_sdk::Address;
use std::collections::HashMap;

pub struct SimulatedTx<T> {
    fee: i64,
    instructions: u64,
    required_auths: Vec<Address>,
    success: bool,
    result: Option<T>,
}

impl<T> SimulatedTx<T> {
    /// Internal constructor used by `MockEnv`.
    pub(crate) fn new(
        fee: i64,
        instructions: u64,
        required_auths: Vec<Address>,
        success: bool,
        result: Option<T>,
    ) -> Self {
        Self {
            fee,
            instructions,
            required_auths,
            success,
            result,
        }
    }

    /// Returns the estimated network fee in stroops.
    pub fn fee(&self) -> i64 {
        self.fee
    }

    /// Returns the total instruction count consumed by the call.
    pub fn instructions(&self) -> u64 {
        self.instructions
    }

    /// Returns the list of addresses that required authorization during the call.
    pub fn required_auths(&self) -> Vec<Address> {
        self.required_auths.clone()
    }

    /// Returns whether the transaction would succeed if committed.
    pub fn would_succeed(&self) -> bool {
        self.success
    }

    /// Returns the result of the call if it succeeded, or `None` if it failed.
    pub fn result(&self) -> Option<&T> {
        self.result.as_ref()
    }

    /// Consumes the simulation and returns the owned dry-run result, if any.
    pub fn into_result(self) -> Option<T> {
        self.result
    }

    /// Returns whether the call's instruction count fits within the
    /// per-transaction CPU limit of the given network budget profile.
    pub fn fits_budget(&self, profile: &BudgetProfile) -> bool {
        self.instructions <= profile.tx_max_instructions
    }
}

/// A commit-capable dry-run of a contract call.
///
/// In addition to the inspection data of a [`SimulatedTx`], a `PreparedTx`
/// retains the call closure so the call can be re-executed and its state
/// changes applied via [`commit`](PreparedTx::commit).
///
/// # When the code executes
///
/// The call closure runs **exactly twice** over the lifetime of a prepared
/// transaction, and at precisely these points:
///
/// 1. Once eagerly inside [`MockEnv::prepare`](crate::env::MockEnv::prepare),
///    to produce the dry-run estimate. Auth is mocked only for this run and is
///    cleared before `prepare` returns.
/// 2. Once inside [`commit`](PreparedTx::commit), to apply the state changes.
///    This run uses the environment's real auth state.
///
/// It never runs at any other time — inspecting fields between `prepare` and
/// `commit` does not re-execute the call.
///
/// ```ignore
/// // Commit-capable: inspect, then apply if the estimate looks good.
/// let prepared = env.prepare(|| client.transfer(&from, &to, &100));
/// assert!(prepared.would_succeed());
/// if prepared.fee() < budget {
///     prepared.commit(); // re-runs the call and applies state changes
/// }
/// ```
pub struct PreparedTx<F, T>
where
    F: Fn() -> T,
{
    simulation: SimulatedTx<T>,
    commit_fn: F,
}

impl<F, T> PreparedTx<F, T>
where
    F: Fn() -> T,
{
    /// Internal constructor used by `MockEnv`.
    pub(crate) fn new(simulation: SimulatedTx<T>, commit_fn: F) -> Self {
        Self {
            simulation,
            commit_fn,
        }
    }

    /// Borrow the underlying inspect-only dry-run.
    pub fn simulation(&self) -> &SimulatedTx<T> {
        &self.simulation
    }

    /// Returns the estimated network fee in stroops.
    pub fn fee(&self) -> i64 {
        self.simulation.fee()
    }

    /// Returns the total instruction count consumed by the call.
    pub fn instructions(&self) -> u64 {
        self.simulation.instructions()
    }

    /// Returns the list of addresses that required authorization during the call.
    pub fn required_auths(&self) -> Vec<Address> {
        self.simulation.required_auths()
    }

    /// Returns whether the transaction would succeed if committed.
    pub fn would_succeed(&self) -> bool {
        self.simulation.would_succeed()
    }

    /// Returns the dry-run result of the call, if it succeeded.
    pub fn result(&self) -> Option<&T> {
        self.simulation.result()
    }

    /// Re-runs the call and commits the state changes.
    ///
    /// This is the **only** API on a prepared transaction that mutates state.
    /// See the [type-level docs](PreparedTx#when-the-code-executes) for exactly
    /// when the closure executes.
    ///
    /// # Panics
    ///
    /// Panics if the dry-run indicated the transaction would not succeed.
    pub fn commit(self) -> T {
        if !self.simulation.would_succeed() {
            panic!("Cannot commit a failed transaction simulation.");
        }
        (self.commit_fn)()
    }
}

// Location: contracts/crucible/src/sim.rs // Production requirement: Reentrancy Guard & Ingress Lock Validator

/// Contract error codes used by the reentrancy / ingress-lock validator.
#[derive(Copy, Clone, Debug, Eq, PartialEq)]
#[repr(u32)]
pub enum ContractError {
    /// A callee was re-entered while its ingress lock was held.
    ReentrancyDetected = 1,
}

/// Per-callee mutex held for the duration of an external (ingress) call.
#[derive(Clone, Debug, Default)]
pub struct IngressLock {
    depth: u32,
}

impl IngressLock {
    pub fn new() -> Self {
        Self { depth: 0 }
    }

    /// Acquire the lock. Nested `enter` returns [`ContractError::ReentrancyDetected`].
    pub fn enter(&mut self) -> Result<(), ContractError> {
        if self.depth != 0 {
            return Err(ContractError::ReentrancyDetected);
        }
        self.depth = 1;
        Ok(())
    }

    /// Release the lock after the ingress returns.
    pub fn exit(&mut self) {
        self.depth = 0;
    }

    pub fn is_locked(&self) -> bool {
        self.depth != 0
    }
}

/// Multi-contract ingress lock table used by [`ReentrancyProbe`].
#[derive(Clone, Debug, Default)]
pub struct IngressLockValidator {
    locks: HashMap<String, IngressLock>,
}

impl IngressLockValidator {
    pub fn new() -> Self {
        Self {
            locks: HashMap::new(),
        }
    }

    pub fn enter(&mut self, contract_id: &str) -> Result<(), ContractError> {
        self.locks
            .entry(contract_id.to_string())
            .or_default()
            .enter()
    }

    pub fn exit(&mut self, contract_id: &str) {
        if let Some(lock) = self.locks.get_mut(contract_id) {
            lock.exit();
        }
    }

    pub fn is_locked(&self, contract_id: &str) -> bool {
        self.locks.get(contract_id).is_some_and(|l| l.is_locked())
    }
}

/// Outcome of a nested re-invocation attempted by [`ReentrancyProbe`].
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ReentrancyProbeResult {
    nested_error: Option<ContractError>,
    nested_reverted: bool,
    nested_succeeded: bool,
}

impl ReentrancyProbeResult {
    /// `true` when the nested call was rejected with `ReentrancyDetected` or reverted.
    pub fn is_guarded(&self) -> bool {
        self.nested_error == Some(ContractError::ReentrancyDetected) || self.nested_reverted
    }

    pub fn nested_error(&self) -> Option<ContractError> {
        self.nested_error
    }

    pub fn nested_reverted(&self) -> bool {
        self.nested_reverted
    }

    pub fn nested_succeeded(&self) -> bool {
        self.nested_succeeded
    }

    /// Assert that reentrant calls trigger [`ContractError::ReentrancyDetected`] or revert.
    pub fn assert_guarded(&self) {
        assert!(
            self.is_guarded(),
            "reentrant call was not rejected: expected ContractError::ReentrancyDetected or revert"
        );
        if let Some(err) = self.nested_error {
            assert_eq!(err, ContractError::ReentrancyDetected);
        }
    }
}

/// Automated probe that re-invokes a callee while an ingress lock is held.
#[derive(Debug, Default)]
pub struct ReentrancyProbe {
    validator: IngressLockValidator,
}

impl ReentrancyProbe {
    pub fn new() -> Self {
        Self {
            validator: IngressLockValidator::new(),
        }
    }

    pub fn validator(&mut self) -> &mut IngressLockValidator {
        &mut self.validator
    }

    /// Hold an ingress lock on `contract_id` and recursively re-invoke `callee`.
    ///
    /// A guarded callee must call [`IngressLockValidator::enter`] (or panic). The
    /// nested invocation is expected to yield [`ContractError::ReentrancyDetected`].
    pub fn attempt_recursive_invoke<F, T>(
        &mut self,
        contract_id: &str,
        mut callee: F,
    ) -> ReentrancyProbeResult
    where
        F: FnMut(&mut IngressLockValidator) -> Result<T, ContractError>,
    {
        self.validator
            .enter(contract_id)
            .expect("outer ingress must be free before the probe starts");

        let nested = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
            callee(&mut self.validator)
        }));

        self.validator.exit(contract_id);

        match nested {
            Ok(Ok(_)) => ReentrancyProbeResult {
                nested_error: None,
                nested_reverted: false,
                nested_succeeded: true,
            },
            Ok(Err(e)) => ReentrancyProbeResult {
                nested_error: Some(e),
                nested_reverted: e == ContractError::ReentrancyDetected,
                nested_succeeded: false,
            },
            Err(_) => ReentrancyProbeResult {
                nested_error: None,
                nested_reverted: true,
                nested_succeeded: false,
            },
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::cell::Cell;

    #[test]
    fn simulated_tx_exposes_inspection_data() {
        let sim = SimulatedTx::new(100, 50, Vec::new(), true, Some(7u32));
        assert_eq!(sim.fee(), 100);
        assert_eq!(sim.instructions(), 50);
        assert!(sim.required_auths().is_empty());
        assert!(sim.would_succeed());
        assert_eq!(sim.result(), Some(&7));
        assert_eq!(sim.into_result(), Some(7));
    }

    #[test]
    fn prepared_tx_does_not_rerun_until_commit() {
        let runs = Cell::new(0u32);
        let call = || {
            runs.set(runs.get() + 1);
            42u32
        };

        // The dry-run already happened in `prepare`; here we model that result.
        let sim = SimulatedTx::new(10, 5, Vec::new(), true, Some(42u32));
        let prepared = PreparedTx::new(sim, call);

        // Inspecting must not execute the closure.
        assert!(prepared.would_succeed());
        assert_eq!(prepared.fee(), 10);
        assert_eq!(prepared.result(), Some(&42));
        assert_eq!(runs.get(), 0);

        // Commit executes the closure exactly once.
        let out = prepared.commit();
        assert_eq!(out, 42);
        assert_eq!(runs.get(), 1);
    }

    #[test]
    #[should_panic(expected = "Cannot commit a failed transaction simulation.")]
    fn commit_panics_when_simulation_failed() {
        let sim = SimulatedTx::new(0, 0, Vec::new(), false, None::<u32>);
        let prepared = PreparedTx::new(sim, || 0u32);
        let _ = prepared.commit();
    }
}

#[cfg(test)]
mod extra_tests {
    use super::*;
    use crate::env::{MockEnv, MockEnvBuilder, Stroops};
    use soroban_sdk::Address;

    #[test]
    fn test_inspected_tx_borrows_local_client() {
        let env = MockEnv::builder()
            .with_account("alice", Stroops::xlm(10_000))
            .build();

        let alice = env.account("alice");
        let address = alice.address();

        // This works because simulate_inspect doesn't require 'static
        let inspected = env.simulate_inspect(|| {
            // We can borrow the address here
            address.clone()
        });

        assert!(inspected.would_succeed());
        assert_eq!(inspected.result(), Some(&address));
    }

    #[test]
    fn test_simulated_tx_requires_static() {
        let env = MockEnv::builder()
            .with_account("alice", Stroops::xlm(10_000))
            .build();

        let alice = env.account("alice");
        let address = alice.address();

        // This requires 'static, so we need to clone or use Arc
        let address_clone = address.clone();
        let sim = env.simulate(move || {
            // Must use owned data, not borrowed
            address_clone
        });

        assert!(sim.would_succeed());
        assert_eq!(sim.result(), Some(&address));
    }

    #[test]
    fn test_inspected_tx_inspection_methods() {
        let env = MockEnv::builder()
            .with_account("alice", Stroops::xlm(10_000))
            .build();

        let inspected = env.simulate_inspect(|| env.account("alice").address());

        assert!(inspected.would_succeed());
        assert!(inspected.fee() >= 0);
        assert!(inspected.result().is_some());
    }
}

// Location: contracts/crucible/src/sim.rs // Production requirement: Reentrancy Guard & Ingress Lock Validator
#[cfg(test)]
mod reentrancy_probe_tests {
    use super::*;
    use crate::env::MockEnv;
    use soroban_sdk::{
        contract, contracterror, contractimpl, contracttype, panic_with_error, Address, Env,
    };

    #[test]
    fn ingress_lock_rejects_nested_enter() {
        let mut lock = IngressLock::new();
        assert!(lock.enter().is_ok());
        assert!(lock.is_locked());
        assert_eq!(lock.enter(), Err(ContractError::ReentrancyDetected));
        lock.exit();
        assert!(!lock.is_locked());
        assert!(lock.enter().is_ok());
    }

    #[test]
    fn probe_guarded_callee_reports_reentrancy_detected() {
        let mut probe = ReentrancyProbe::new();
        let result = probe.attempt_recursive_invoke("lending-vault", |lock| {
            lock.enter("lending-vault")?;
            lock.exit("lending-vault");
            Ok(())
        });
        result.assert_guarded();
        assert_eq!(
            result.nested_error(),
            Some(ContractError::ReentrancyDetected)
        );
    }

    #[test]
    fn probe_vulnerable_callee_succeeds_on_reentry() {
        let mut probe = ReentrancyProbe::new();
        let result = probe.attempt_recursive_invoke("lending-vault", |_lock| Ok(()));
        assert!(result.nested_succeeded());
        assert!(!result.is_guarded());
    }

    #[test]
    fn probe_treats_panic_as_revert() {
        let mut probe = ReentrancyProbe::new();
        let result = probe.attempt_recursive_invoke("vault", |_lock| -> Result<(), ContractError> {
            panic!("reentrancy guard tripped");
        });
        assert!(result.nested_reverted());
        result.assert_guarded();
    }

    #[contracterror]
    #[derive(Copy, Clone, Debug, Eq, PartialEq, PartialOrd, Ord)]
    #[repr(u32)]
    pub enum VaultError {
        ReentrancyDetected = 1,
        InsufficientBalance = 2,
    }

    #[contracttype]
    #[derive(Clone)]
    enum DataKey {
        Balance(Address),
        Lock,
        Callback,
        Vault,
        Attacked,
    }

    /// Lending vault that updates balances after the cross-contract callback.
    #[contract]
    struct VulnerableLendingVault;

    #[contractimpl]
    impl VulnerableLendingVault {
        pub fn deposit(env: Env, user: Address, amount: i128) {
            user.require_auth();
            let key = DataKey::Balance(user.clone());
            let bal: i128 = env.storage().instance().get(&key).unwrap_or(0);
            env.storage().instance().set(&key, &(bal + amount));
        }

        pub fn set_callback(env: Env, callback: Address) {
            env.storage()
                .instance()
                .set(&DataKey::Callback, &callback);
        }

        pub fn balance(env: Env, user: Address) -> i128 {
            env.storage()
                .instance()
                .get(&DataKey::Balance(user))
                .unwrap_or(0)
        }

        pub fn withdraw(env: Env, user: Address, amount: i128) {
            user.require_auth();
            let key = DataKey::Balance(user.clone());
            let bal: i128 = env.storage().instance().get(&key).unwrap_or(0);
            if bal < amount {
                panic_with_error!(&env, VaultError::InsufficientBalance);
            }
            if let Some(cb) = env.storage().instance().get::<_, Address>(&DataKey::Callback) {
                ReentrantAttackerClient::new(&env, &cb).on_funds(&user, &amount);
            }
            let after: i128 = env.storage().instance().get(&key).unwrap_or(0);
            env.storage().instance().set(&key, &(after - amount));
        }
    }

    /// Lending vault with an ingress lock around withdraw.
    #[contract]
    struct GuardedLendingVault;

    #[contractimpl]
    impl GuardedLendingVault {
        fn lock(env: &Env) {
            let locked: bool = env
                .storage()
                .instance()
                .get(&DataKey::Lock)
                .unwrap_or(false);
            if locked {
                panic_with_error!(env, VaultError::ReentrancyDetected);
            }
            env.storage().instance().set(&DataKey::Lock, &true);
        }

        fn unlock(env: &Env) {
            env.storage().instance().set(&DataKey::Lock, &false);
        }

        pub fn deposit(env: Env, user: Address, amount: i128) {
            user.require_auth();
            let key = DataKey::Balance(user.clone());
            let bal: i128 = env.storage().instance().get(&key).unwrap_or(0);
            env.storage().instance().set(&key, &(bal + amount));
        }

        pub fn set_callback(env: Env, callback: Address) {
            env.storage()
                .instance()
                .set(&DataKey::Callback, &callback);
        }

        pub fn balance(env: Env, user: Address) -> i128 {
            env.storage()
                .instance()
                .get(&DataKey::Balance(user))
                .unwrap_or(0)
        }

        pub fn withdraw(env: Env, user: Address, amount: i128) {
            Self::lock(&env);
            user.require_auth();
            let key = DataKey::Balance(user.clone());
            let bal: i128 = env.storage().instance().get(&key).unwrap_or(0);
            if bal < amount {
                Self::unlock(&env);
                panic_with_error!(&env, VaultError::InsufficientBalance);
            }
            env.storage().instance().set(&key, &(bal - amount));
            if let Some(cb) = env.storage().instance().get::<_, Address>(&DataKey::Callback) {
                ReentrantAttackerClient::new(&env, &cb).on_funds(&user, &amount);
            }
            Self::unlock(&env);
        }

        /// Same-frame nested lock: proves the ingress lock fires `ReentrancyDetected`
        /// even when the Soroban host would also reject cross-contract re-entry.
        pub fn nested_lock_probe(env: Env) {
            Self::lock(&env);
            Self::lock(&env);
            Self::unlock(&env);
        }
    }

    #[contract]
    struct ReentrantAttacker;

    #[contractimpl]
    impl ReentrantAttacker {
        pub fn bind(env: Env, vault: Address) {
            env.storage().instance().set(&DataKey::Vault, &vault);
            env.storage().instance().set(&DataKey::Attacked, &false);
        }

        pub fn on_funds(env: Env, user: Address, amount: i128) {
            let attacked: bool = env
                .storage()
                .instance()
                .get(&DataKey::Attacked)
                .unwrap_or(false);
            if attacked {
                return;
            }
            env.storage().instance().set(&DataKey::Attacked, &true);
            let vault: Address = env.storage().instance().get(&DataKey::Vault).unwrap();
            // Recursive re-invocation of the callee withdraw.
            GuardedLendingVaultClient::new(&env, &vault).withdraw(&user, &amount);
        }
    }

    fn setup_user(env: &MockEnv) -> Address {
        use soroban_sdk::testutils::Address as _;
        Address::generate(env.inner())
    }

    #[test]
    fn guarded_lending_vault_rejects_reentrant_withdraw() {
        let env = MockEnv::default();
        env.mock_all_auths();
        let inner = env.inner();

        let vault_id = inner.register(GuardedLendingVault, ());
        let attacker_id = inner.register(ReentrantAttacker, ());
        let vault = GuardedLendingVaultClient::new(inner, &vault_id);
        let attacker = ReentrantAttackerClient::new(inner, &attacker_id);

        attacker.bind(&vault_id);
        vault.set_callback(&attacker_id);

        let user = setup_user(&env);
        vault.deposit(&user, &100);
        assert_eq!(vault.balance(&user), 100);

        let result = vault.try_withdraw(&user, &40);
        assert!(
            result.is_err(),
            "guarded vault must reject reentrant withdraw"
        );

        // Whole transaction reverts: user still has the original deposit.
        assert_eq!(vault.balance(&user), 100);
    }

    #[test]
    fn guarded_lending_vault_withdraw_succeeds_without_callback() {
        let env = MockEnv::default();
        env.mock_all_auths();
        let inner = env.inner();
        let vault_id = inner.register(GuardedLendingVault, ());
        let vault = GuardedLendingVaultClient::new(inner, &vault_id);
        let user = setup_user(&env);
        vault.deposit(&user, &100);
        vault.withdraw(&user, &40);
        assert_eq!(vault.balance(&user), 60);
    }

    #[test]
    fn guarded_lending_vault_nested_lock_is_reentrancy_detected() {
        let env = MockEnv::default();
        env.mock_all_auths();
        let inner = env.inner();
        let vault_id = inner.register(GuardedLendingVault, ());
        let vault = GuardedLendingVaultClient::new(inner, &vault_id);
        let result = vault.try_nested_lock_probe();
        assert!(
            result.is_err(),
            "same-frame nested lock must revert with ReentrancyDetected"
        );
    }

    #[test]
    fn vulnerable_lending_vault_reentry_reverts_at_host() {
        let env = MockEnv::default();
        env.mock_all_auths();
        let inner = env.inner();

        let vault_id = inner.register(VulnerableLendingVault, ());
        let attacker_id = inner.register(ReentrantAttacker, ());
        let vault = VulnerableLendingVaultClient::new(inner, &vault_id);
        let attacker = ReentrantAttackerClient::new(inner, &attacker_id);

        attacker.bind(&vault_id);
        vault.set_callback(&attacker_id);

        let user = setup_user(&env);
        vault.deposit(&user, &100);

        // CEI-unsafe vault still cannot be drained: Soroban forbids contract re-entry.
        let result = vault.try_withdraw(&user, &100);
        assert!(
            result.is_err(),
            "vulnerable vault re-entry must revert (host ingress lock)"
        );
        assert_eq!(vault.balance(&user), 100);
    }
}

// Location: contracts/crucible/src/sim.rs // Production requirement: Wasm Memory Allocator Leak & Stack Depth Tester

/// Size of a single Soroban Wasm linear-memory page (64 KiB).
pub const WASM_PAGE_SIZE_BYTES: usize = 64 * 1024;

/// Default Soroban host memory ceiling (~40 MiB).
pub const DEFAULT_MAX_MEMORY_BYTES: usize = 40 * 1024 * 1024;

/// Default recursion / call-stack depth budget for simulated execution.
pub const DEFAULT_MAX_STACK_DEPTH: u32 = 256;

/// Configured memory and stack limits for a simulated contract run.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct MemoryLimitConfig {
    /// Maximum linear memory in bytes (must be a multiple of [`WASM_PAGE_SIZE_BYTES`] conceptually).
    pub max_memory_bytes: usize,
    /// Maximum nested call / recursion depth.
    pub max_stack_depth: u32,
}

impl Default for MemoryLimitConfig {
    fn default() -> Self {
        Self {
            max_memory_bytes: DEFAULT_MAX_MEMORY_BYTES,
            max_stack_depth: DEFAULT_MAX_STACK_DEPTH,
        }
    }
}

impl MemoryLimitConfig {
    pub fn max_pages(&self) -> usize {
        self.max_memory_bytes / WASM_PAGE_SIZE_BYTES
    }
}

/// Violation raised when a simulated allocation or stack push exceeds limits.
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum MemoryLimitViolation {
    /// Requested pages would exceed the configured memory ceiling.
    OutOfMemory {
        requested_pages: usize,
        peak_pages: usize,
        max_pages: usize,
    },
    /// Recursion / nesting depth exceeded the configured stack budget.
    StackOverflow {
        depth: u32,
        max_depth: u32,
    },
}

/// Monitors peak Wasm memory page allocations and recursion stack depth
/// during simulated contract execution.
#[derive(Clone, Debug)]
pub struct WasmMemoryMonitor {
    config: MemoryLimitConfig,
    current_pages: usize,
    peak_pages: usize,
    stack_depth: u32,
    peak_stack_depth: u32,
}

impl WasmMemoryMonitor {
    pub fn new(config: MemoryLimitConfig) -> Self {
        Self {
            config,
            current_pages: 0,
            peak_pages: 0,
            stack_depth: 0,
            peak_stack_depth: 0,
        }
    }

    pub fn with_defaults() -> Self {
        Self::new(MemoryLimitConfig::default())
    }

    pub fn config(&self) -> &MemoryLimitConfig {
        &self.config
    }

    pub fn current_pages(&self) -> usize {
        self.current_pages
    }

    pub fn peak_pages(&self) -> usize {
        self.peak_pages
    }

    pub fn peak_memory_bytes(&self) -> usize {
        self.peak_pages.saturating_mul(WASM_PAGE_SIZE_BYTES)
    }

    pub fn stack_depth(&self) -> u32 {
        self.stack_depth
    }

    pub fn peak_stack_depth(&self) -> u32 {
        self.peak_stack_depth
    }

    /// Grow linear memory by `pages` (64 KiB each). Returns the previous page count.
    pub fn grow_pages(&mut self, pages: usize) -> Result<usize, MemoryLimitViolation> {
        let next = self.current_pages.saturating_add(pages);
        if next > self.config.max_pages() {
            return Err(MemoryLimitViolation::OutOfMemory {
                requested_pages: pages,
                peak_pages: self.peak_pages.max(next),
                max_pages: self.config.max_pages(),
            });
        }
        let prev = self.current_pages;
        self.current_pages = next;
        self.peak_pages = self.peak_pages.max(self.current_pages);
        Ok(prev)
    }

    /// Account for an allocation of `bytes`, rounding up to whole Wasm pages.
    pub fn allocate_bytes(&mut self, bytes: usize) -> Result<usize, MemoryLimitViolation> {
        let pages = bytes.div_ceil(WASM_PAGE_SIZE_BYTES).max(1);
        self.grow_pages(pages)
    }

    /// Release `pages` of linear memory (simulates free / drop of large buffers).
    pub fn free_pages(&mut self, pages: usize) {
        self.current_pages = self.current_pages.saturating_sub(pages);
    }

    /// Push one frame onto the simulated call stack.
    pub fn push_frame(&mut self) -> Result<u32, MemoryLimitViolation> {
        let next = self.stack_depth.saturating_add(1);
        if next > self.config.max_stack_depth {
            return Err(MemoryLimitViolation::StackOverflow {
                depth: next,
                max_depth: self.config.max_stack_depth,
            });
        }
        self.stack_depth = next;
        self.peak_stack_depth = self.peak_stack_depth.max(self.stack_depth);
        Ok(self.stack_depth)
    }

    /// Pop one frame from the simulated call stack.
    pub fn pop_frame(&mut self) {
        self.stack_depth = self.stack_depth.saturating_sub(1);
    }

    /// Assert peak memory and stack stayed strictly within configured limits.
    pub fn assert_within_limits(&self) {
        assert!(
            self.peak_pages <= self.config.max_pages(),
            "peak memory pages {} exceed limit {} ({} bytes > {} bytes)",
            self.peak_pages,
            self.config.max_pages(),
            self.peak_memory_bytes(),
            self.config.max_memory_bytes
        );
        assert!(
            self.peak_stack_depth <= self.config.max_stack_depth,
            "peak stack depth {} exceeds limit {}",
            self.peak_stack_depth,
            self.config.max_stack_depth
        );
        assert!(
            self.peak_memory_bytes() <= self.config.max_memory_bytes,
            "peak memory bytes must remain strictly within configured limits"
        );
    }

    /// Recursively walk a nested `Vec` tree, charging stack frames and page growth
    /// proportional to element counts — used to stress deeply nested payloads.
    pub fn stress_nested_vectors<T>(
        &mut self,
        depth: u32,
        branching: usize,
        leaf_bytes: usize,
        _marker: &T,
    ) -> Result<(), MemoryLimitViolation> {
        self.push_frame()?;
        if depth == 0 {
            let _ = self.allocate_bytes(leaf_bytes.max(1))?;
            self.pop_frame();
            return Ok(());
        }
        // Charge a page for the vector spine at this level.
        let spine_bytes = branching.saturating_mul(std::mem::size_of::<usize>()).max(1);
        let _ = self.allocate_bytes(spine_bytes)?;
        for _ in 0..branching {
            self.stress_nested_vectors(depth - 1, branching, leaf_bytes, _marker)?;
        }
        self.pop_frame();
        Ok(())
    }
}

#[cfg(test)]
mod wasm_memory_monitor_tests {
    use super::*;

    #[test]
    fn tracks_peak_pages_under_grow_and_free() {
        let mut mon = WasmMemoryMonitor::new(MemoryLimitConfig {
            max_memory_bytes: 256 * 1024, // 4 pages
            max_stack_depth: 32,
        });

        assert_eq!(mon.grow_pages(2).unwrap(), 0);
        assert_eq!(mon.current_pages(), 2);
        assert_eq!(mon.peak_pages(), 2);

        mon.free_pages(1);
        assert_eq!(mon.current_pages(), 1);
        assert_eq!(mon.peak_pages(), 2, "peak must not decrease on free");

        assert_eq!(mon.grow_pages(2).unwrap(), 1);
        assert_eq!(mon.peak_pages(), 3);
        mon.assert_within_limits();
    }

    #[test]
    fn rejects_allocations_beyond_configured_limit() {
        let mut mon = WasmMemoryMonitor::new(MemoryLimitConfig {
            max_memory_bytes: 128 * 1024, // 2 pages
            max_stack_depth: 8,
        });

        assert!(mon.grow_pages(2).is_ok());
        let err = mon.grow_pages(1).unwrap_err();
        assert!(matches!(
            err,
            MemoryLimitViolation::OutOfMemory { max_pages: 2, .. }
        ));
    }

    #[test]
    fn stack_depth_guard_trips_on_deep_recursion() {
        let mut mon = WasmMemoryMonitor::new(MemoryLimitConfig {
            max_memory_bytes: DEFAULT_MAX_MEMORY_BYTES,
            max_stack_depth: 4,
        });

        for _ in 0..4 {
            mon.push_frame().unwrap();
        }
        let err = mon.push_frame().unwrap_err();
        assert_eq!(
            err,
            MemoryLimitViolation::StackOverflow {
                depth: 5,
                max_depth: 4
            }
        );
        assert_eq!(mon.peak_stack_depth(), 4);
    }

    #[test]
    fn maximal_payload_stays_within_soroban_40mb_ceiling() {
        let mut mon = WasmMemoryMonitor::with_defaults();
        // ~39 MiB payload — just under the 40 MiB Soroban limit.
        let bytes = 39 * 1024 * 1024;
        mon.allocate_bytes(bytes).unwrap();
        mon.assert_within_limits();
        assert!(mon.peak_memory_bytes() <= DEFAULT_MAX_MEMORY_BYTES);
        assert!(mon.peak_pages() <= MemoryLimitConfig::default().max_pages());
    }

    #[test]
    fn stress_deeply_nested_vector_structures() {
        let mut mon = WasmMemoryMonitor::new(MemoryLimitConfig {
            max_memory_bytes: 8 * 1024 * 1024,
            max_stack_depth: 64,
        });

        // depth=6, branching=2 → 2^6 leaves; exercises stack + page accounting.
        mon.stress_nested_vectors(6, 2, 1024, &0u8).unwrap();
        mon.assert_within_limits();
        assert!(mon.peak_stack_depth() >= 6);
        assert!(mon.peak_pages() > 0);
    }

    #[test]
    fn nested_vector_stress_can_hit_stack_limit() {
        let mut mon = WasmMemoryMonitor::new(MemoryLimitConfig {
            max_memory_bytes: DEFAULT_MAX_MEMORY_BYTES,
            max_stack_depth: 5,
        });

        let err = mon
            .stress_nested_vectors(10, 1, 64, &0u8)
            .unwrap_err();
        assert!(matches!(err, MemoryLimitViolation::StackOverflow { .. }));
    }

    #[test]
    fn allocate_bytes_rounds_up_to_whole_pages() {
        let mut mon = WasmMemoryMonitor::with_defaults();
        mon.allocate_bytes(1).unwrap();
        assert_eq!(mon.current_pages(), 1);
        mon.allocate_bytes(WASM_PAGE_SIZE_BYTES + 1).unwrap();
        assert_eq!(mon.current_pages(), 1 + 2);
    }
}

// Location: contracts/crucible/src/sim.rs // Production requirement: CPU & Memory Fuel Simulation Variance from Protocol 22 Network

/// Network resource limits and fee rates used to evaluate simulated calls.
///
/// Mirrors the Soroban network configuration settings
/// (`ConfigSettingContractComputeV0` / `ConfigSettingContractLedgerCostV0`)
/// so local profiling flags calls that would exceed Testnet / Mainnet limits.
/// Fields are public so a profile can be adjusted to match any network.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct BudgetProfile {
    /// Maximum CPU instructions a single transaction may consume.
    pub tx_max_instructions: u64,
    /// Maximum memory (bytes) a single transaction may consume.
    pub tx_memory_limit: u64,
    /// Fee in stroops charged per 10,000 CPU instructions.
    pub fee_per_instructions_increment: i64,
}

/// Instruction increment used by the network when charging compute fees.
pub const INSTRUCTIONS_INCREMENT: u64 = 10_000;

impl BudgetProfile {
    /// Current Soroban network configuration (Protocol 22, Testnet and Mainnet).
    pub const fn current() -> Self {
        Self::protocol_22()
    }

    /// Soroban Protocol 21 network configuration.
    pub const fn protocol_21() -> Self {
        Self {
            tx_max_instructions: 100_000_000,
            tx_memory_limit: 41_943_040,
            fee_per_instructions_increment: 25,
        }
    }

    /// Soroban Protocol 22 network configuration.
    pub const fn protocol_22() -> Self {
        Self {
            tx_max_instructions: 100_000_000,
            tx_memory_limit: 41_943_040,
            fee_per_instructions_increment: 25,
        }
    }

    /// Computes the compute fee in stroops for `instructions`, rounding up to
    /// the next instruction increment as the network does.
    pub fn instruction_fee(&self, instructions: u64) -> i64 {
        let increments = instructions.div_ceil(INSTRUCTIONS_INCREMENT);
        (increments as i64).saturating_mul(self.fee_per_instructions_increment)
    }

    /// Returns whether `instructions` and `memory_bytes` both fit within the
    /// per-transaction limits of this profile.
    pub fn fits(&self, instructions: u64, memory_bytes: u64) -> bool {
        instructions <= self.tx_max_instructions && memory_bytes <= self.tx_memory_limit
    }
}

impl Default for BudgetProfile {
    fn default() -> Self {
        Self::current()
    }
}

#[cfg(test)]
mod budget_profile_tests {
    use super::*;

    #[test]
    fn current_matches_protocol_22() {
        assert_eq!(BudgetProfile::current(), BudgetProfile::protocol_22());
        assert_eq!(BudgetProfile::default(), BudgetProfile::current());
    }

    #[test]
    fn instruction_fee_rounds_up_to_increment() {
        let profile = BudgetProfile::current();
        assert_eq!(profile.instruction_fee(0), 0);
        assert_eq!(profile.instruction_fee(1), 25);
        assert_eq!(profile.instruction_fee(10_000), 25);
        assert_eq!(profile.instruction_fee(10_001), 50);
    }

    #[test]
    fn fits_respects_network_limits() {
        let profile = BudgetProfile::current();
        assert!(profile.fits(100_000_000, 41_943_040));
        assert!(!profile.fits(100_000_001, 0));
        assert!(!profile.fits(0, 41_943_041));
    }

    #[test]
    fn simulated_tx_fits_budget_checks_instruction_limit() {
        let profile = BudgetProfile::current();
        let ok = SimulatedTx::new(0, 100_000_000, Vec::new(), true, Some(()));
        let over = SimulatedTx::new(0, 100_000_001, Vec::new(), true, Some(()));
        assert!(ok.fits_budget(&profile));
        assert!(!over.fits_budget(&profile));
    }
}

// Location: contracts/crucible/src/sim.rs // Production requirement: Automated Storage Entry Footprint & Rent Cost Calculator

/// Storage lifetime class of a contract data entry.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum StorageDurability {
    /// `env.storage().instance()` — stored in the contract instance entry and
    /// rented at the persistent rate.
    Instance,
    /// `env.storage().persistent()` — archived (restorable) when its TTL expires.
    Persistent,
    /// `env.storage().temporary()` — deleted for good when its TTL expires.
    Temporary,
}

/// Byte footprint of one key/value pair written to contract storage.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct StorageEntryFootprint {
    /// Storage lifetime class the entry was written to.
    pub durability: StorageDurability,
    /// XDR-encoded size of the key in bytes.
    pub key_bytes: u32,
    /// XDR-encoded size of the value in bytes.
    pub value_bytes: u32,
}

impl StorageEntryFootprint {
    /// Total ledger bytes occupied by the entry (key + value).
    pub fn total_bytes(&self) -> u32 {
        self.key_bytes.saturating_add(self.value_bytes)
    }
}

/// Soroban network rent parameters (`ConfigSettingContractLedgerCostV0` /
/// `StateArchivalSettings`).
///
/// Rent for an entry over `ledgers` is
/// `ceil(bytes * write_fee_per_1kb * ledgers / (1024 * rent_rate_denominator))`,
/// where the denominator depends on the entry's durability. Fields are public
/// so the config can be aligned with any network's live settings.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct RentConfig {
    /// Fee in stroops for writing 1 KB to the ledger.
    pub write_fee_per_1kb: i64,
    /// Rent rate denominator for persistent and instance entries.
    pub persistent_rent_rate_denominator: i64,
    /// Rent rate denominator for temporary entries.
    pub temp_rent_rate_denominator: i64,
}

impl RentConfig {
    /// Current Soroban network rent configuration (Protocol 22).
    pub const fn current() -> Self {
        Self {
            write_fee_per_1kb: 11_800,
            persistent_rent_rate_denominator: 2_103,
            temp_rent_rate_denominator: 4_206,
        }
    }

    /// Computes the rent in stroops for `bytes` of `durability` storage kept
    /// alive for `ledgers` ledgers.
    pub fn rent_fee(&self, durability: StorageDurability, bytes: u32, ledgers: u32) -> i64 {
        let denominator = match durability {
            StorageDurability::Instance | StorageDurability::Persistent => {
                self.persistent_rent_rate_denominator
            }
            StorageDurability::Temporary => self.temp_rent_rate_denominator,
        };
        let numerator = (bytes as i128) * (self.write_fee_per_1kb as i128) * (ledgers as i128);
        let divisor = 1024_i128 * (denominator.max(1) as i128);
        let fee = (numerator + divisor - 1) / divisor;
        i64::try_from(fee).unwrap_or(i64::MAX)
    }
}

impl Default for RentConfig {
    fn default() -> Self {
        Self::current()
    }
}

/// Per-durability bytes and rent produced by [`StorageFootprint::rent_cost`].
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct RentCostReport {
    /// Bytes held in instance storage.
    pub instance_bytes: u64,
    /// Bytes held in persistent storage.
    pub persistent_bytes: u64,
    /// Bytes held in temporary storage.
    pub temporary_bytes: u64,
    /// Rent in stroops for instance storage.
    pub instance_rent: i64,
    /// Rent in stroops for persistent storage.
    pub persistent_rent: i64,
    /// Rent in stroops for temporary storage.
    pub temporary_rent: i64,
}

impl RentCostReport {
    /// Total bytes across all durabilities.
    pub fn total_bytes(&self) -> u64 {
        self.instance_bytes + self.persistent_bytes + self.temporary_bytes
    }

    /// Total rent in stroops across all durabilities.
    pub fn total_rent(&self) -> i64 {
        self.instance_rent
            .saturating_add(self.persistent_rent)
            .saturating_add(self.temporary_rent)
    }
}

/// Suggests moving a persistent entry to temporary storage to save rent.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct StorageRecommendation {
    /// Index of the entry in [`StorageFootprint::entries`].
    pub entry_index: usize,
    /// Rent in stroops if the entry stays persistent.
    pub persistent_rent: i64,
    /// Rent in stroops if the entry is stored as temporary instead.
    pub temporary_rent: i64,
}

impl StorageRecommendation {
    /// Stroops saved by switching the entry to temporary storage.
    pub fn savings(&self) -> i64 {
        self.persistent_rent - self.temporary_rent
    }
}

impl std::fmt::Display for StorageRecommendation {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(
            f,
            "entry #{} costs {} stroops as Persistent but {} as Temporary (saves {}); \
             use Temporary if the data can be recreated after it expires",
            self.entry_index,
            self.persistent_rent,
            self.temporary_rent,
            self.savings()
        )
    }
}

/// Records the keys and values written to contract storage during a
/// simulated transaction and estimates their rent.
///
/// ```ignore
/// let mut footprint = StorageFootprint::new();
/// footprint.record_entry(env.inner(), StorageDurability::Persistent, &key, &balance);
/// let report = footprint.rent_cost(&RentConfig::current(), 535_680);
/// for rec in footprint.recommendations(&RentConfig::current(), 17_280) {
///     println!("{rec}");
/// }
/// ```
#[derive(Debug, Clone, Default)]
pub struct StorageFootprint {
    entries: Vec<StorageEntryFootprint>,
}

impl StorageFootprint {
    /// Creates an empty footprint.
    pub fn new() -> Self {
        Self::default()
    }

    /// Records an entry by its XDR-encoded key and value sizes.
    pub fn record_entry<K, V>(
        &mut self,
        env: &soroban_sdk::Env,
        durability: StorageDurability,
        key: &K,
        value: &V,
    ) where
        K: soroban_sdk::IntoVal<soroban_sdk::Env, soroban_sdk::Val> + Clone,
        V: soroban_sdk::IntoVal<soroban_sdk::Env, soroban_sdk::Val> + Clone,
    {
        use soroban_sdk::xdr::ToXdr;
        let key_val: soroban_sdk::Val = key.clone().into_val(env);
        let value_val: soroban_sdk::Val = value.clone().into_val(env);
        self.record(
            durability,
            key_val.to_xdr(env).len(),
            value_val.to_xdr(env).len(),
        );
    }

    /// Records an entry from already-known key and value byte sizes.
    pub fn record(&mut self, durability: StorageDurability, key_bytes: u32, value_bytes: u32) {
        self.entries.push(StorageEntryFootprint {
            durability,
            key_bytes,
            value_bytes,
        });
    }

    /// Returns every recorded entry, in recording order.
    pub fn entries(&self) -> &[StorageEntryFootprint] {
        &self.entries
    }

    /// Total bytes recorded for `durability`.
    pub fn total_bytes(&self, durability: StorageDurability) -> u64 {
        self.entries
            .iter()
            .filter(|e| e.durability == durability)
            .map(|e| e.total_bytes() as u64)
            .sum()
    }

    /// Computes per-durability rent for keeping every entry alive `ledgers` ledgers.
    pub fn rent_cost(&self, config: &RentConfig, ledgers: u32) -> RentCostReport {
        let mut report = RentCostReport::default();
        for entry in &self.entries {
            let bytes = entry.total_bytes();
            let rent = config.rent_fee(entry.durability, bytes, ledgers);
            match entry.durability {
                StorageDurability::Instance => {
                    report.instance_bytes += bytes as u64;
                    report.instance_rent = report.instance_rent.saturating_add(rent);
                }
                StorageDurability::Persistent => {
                    report.persistent_bytes += bytes as u64;
                    report.persistent_rent = report.persistent_rent.saturating_add(rent);
                }
                StorageDurability::Temporary => {
                    report.temporary_bytes += bytes as u64;
                    report.temporary_rent = report.temporary_rent.saturating_add(rent);
                }
            }
        }
        report
    }

    /// Lists persistent entries that would be cheaper as temporary storage
    /// for a `ledgers`-long lifetime.
    pub fn recommendations(&self, config: &RentConfig, ledgers: u32) -> Vec<StorageRecommendation> {
        self.entries
            .iter()
            .enumerate()
            .filter(|(_, e)| e.durability == StorageDurability::Persistent)
            .filter_map(|(entry_index, e)| {
                let bytes = e.total_bytes();
                let persistent_rent = config.rent_fee(StorageDurability::Persistent, bytes, ledgers);
                let temporary_rent = config.rent_fee(StorageDurability::Temporary, bytes, ledgers);
                (temporary_rent < persistent_rent).then_some(StorageRecommendation {
                    entry_index,
                    persistent_rent,
                    temporary_rent,
                })
            })
            .collect()
    }
}

#[cfg(test)]
mod storage_rent_tests {
    use super::*;
    use soroban_sdk::{symbol_short, Env};

    #[test]
    fn rent_fee_follows_the_network_formula() {
        let config = RentConfig::current();
        // 1024 bytes for one persistent "rent period" of 2103 ledgers costs
        // exactly one 1 KB write fee.
        assert_eq!(config.rent_fee(StorageDurability::Persistent, 1024, 2_103), 11_800);
        assert_eq!(config.rent_fee(StorageDurability::Instance, 1024, 2_103), 11_800);
        assert_eq!(config.rent_fee(StorageDurability::Temporary, 1024, 4_206), 11_800);
        assert_eq!(config.rent_fee(StorageDurability::Persistent, 0, 2_103), 0);
        // Partial stroops round up.
        assert_eq!(config.rent_fee(StorageDurability::Persistent, 1, 1), 1);
    }

    #[test]
    fn record_entry_measures_xdr_sizes() {
        let env = Env::default();
        let mut footprint = StorageFootprint::new();
        footprint.record_entry(&env, StorageDurability::Persistent, &symbol_short!("bal"), &500_i128);
        let entry = &footprint.entries()[0];
        assert!(entry.key_bytes > 0);
        assert!(entry.value_bytes > 0);
        assert_eq!(
            footprint.total_bytes(StorageDurability::Persistent),
            entry.total_bytes() as u64
        );
    }

    #[test]
    fn rent_cost_splits_by_durability() {
        let config = RentConfig::current();
        let mut footprint = StorageFootprint::new();
        footprint.record(StorageDurability::Instance, 24, 1000);
        footprint.record(StorageDurability::Persistent, 24, 1000);
        footprint.record(StorageDurability::Temporary, 24, 1000);

        let report = footprint.rent_cost(&config, 2_103);
        assert_eq!(report.total_bytes(), 3 * 1024);
        assert_eq!(report.instance_rent, 11_800);
        assert_eq!(report.persistent_rent, 11_800);
        assert_eq!(report.temporary_rent, 5_900);
        assert_eq!(report.total_rent(), 29_500);
    }

    #[test]
    fn recommends_temporary_for_persistent_entries_only() {
        let config = RentConfig::current();
        let mut footprint = StorageFootprint::new();
        footprint.record(StorageDurability::Instance, 24, 1000);
        footprint.record(StorageDurability::Persistent, 24, 1000);
        footprint.record(StorageDurability::Temporary, 24, 1000);

        let recs = footprint.recommendations(&config, 2_103);
        assert_eq!(recs.len(), 1);
        assert_eq!(recs[0].entry_index, 1);
        assert_eq!(recs[0].persistent_rent, 11_800);
        assert_eq!(recs[0].temporary_rent, 5_900);
        assert_eq!(recs[0].savings(), 5_900);
        assert!(recs[0].to_string().contains("saves 5900"));
    }

    #[test]
    fn no_recommendation_when_rent_is_zero() {
        let config = RentConfig::current();
        let mut footprint = StorageFootprint::new();
        footprint.record(StorageDurability::Persistent, 24, 1000);
        assert!(footprint.recommendations(&config, 0).is_empty());
    }
}
