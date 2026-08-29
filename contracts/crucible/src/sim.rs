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
