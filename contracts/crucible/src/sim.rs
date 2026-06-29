//! Simulated transaction dry-runs and fee estimation.
//!
//! **Host-only:** [`SimulatedTx`] is constructed by [`MockEnv::simulate`] and
//! depends on the Soroban host test utilities. It is intended exclusively for
//! use in `#[cfg(test)]` contexts on the host.
//!
//! [`MockEnv::simulate`]: crate::env::MockEnv::simulate
/// without committing the state changes.
use soroban_sdk::Address;


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

        let inspected = env.simulate_inspect(|| {
            env.account("alice").address()
        });

        assert!(inspected.would_succeed());
        assert!(inspected.fee() >= 0);
        assert!(inspected.instructions() >= 0);
        assert!(inspected.result().is_some());
    }
}
