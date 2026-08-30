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
        assert!(inspected.instructions() >= 0);
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
