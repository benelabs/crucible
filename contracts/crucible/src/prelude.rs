//! Re-exports for use with `crucible::prelude::*`.
//!
//! This module provides convenient access to all commonly used types
//! and utilities from the crucible testing framework.

#[cfg(feature = "std")]
pub use crate::account::AccountBuilder;
#[cfg(feature = "std")]
pub use crate::account::AccountHandle;
#[cfg(feature = "std")]
pub use crate::assertions::{assert_balance_decreased, assert_balance_increased, RevertAssertion};
#[cfg(feature = "std")]
pub use crate::auth_tree::{
    AuthMismatch, AuthTreeReport, ExpectedAuth, ExpectedInvocation, verify_auth_tree,
};
#[cfg(feature = "std")]
pub use crate::call_graph::{CallFrame, CallTrace};
#[cfg(feature = "std")]
pub use crate::checkpoint::{CheckpointId, CheckpointStats};
#[cfg(feature = "std")]
pub use crate::cost::CostReport;
#[cfg(feature = "std")]
pub use crate::env::CapturedEvent;
#[cfg(feature = "std")]
pub use crate::env::Duration;
#[cfg(feature = "std")]
pub use crate::env::EntryTtlSettings;
#[cfg(feature = "std")]
pub use crate::env::DEFAULT_SECONDS_PER_LEDGER;
#[cfg(feature = "std")]
pub use crate::env::EventMatches;
#[cfg(feature = "std")]
pub use crate::env::FailedCallResult;
#[cfg(feature = "std")]
pub use crate::env::MockAuthGuard;
#[cfg(feature = "std")]
pub use crate::env::MockEnv;
#[cfg(feature = "std")]
pub use crate::env::MockEnvBuilder;
#[cfg(feature = "std")]
pub use crate::env::ProtocolVersion;
#[cfg(feature = "std")]
pub use crate::env::Stroops;
#[cfg(feature = "std")]
pub use crate::sim::BudgetProfile;
#[cfg(feature = "std")]
pub use crate::sim::ContractError;
#[cfg(feature = "std")]
pub use crate::sim::IngressLock;
#[cfg(feature = "std")]
pub use crate::sim::IngressLockValidator;
#[cfg(feature = "std")]
pub use crate::sim::PreparedTx;
#[cfg(feature = "std")]
pub use crate::sim::ReentrancyProbe;
#[cfg(feature = "std")]
pub use crate::sim::ReentrancyProbeResult;
#[cfg(feature = "std")]
pub use crate::sim::SimulatedTx;
#[cfg(feature = "std")]
pub use crate::sim::{RentConfig, StorageDurability, StorageFootprint};
#[cfg(feature = "std")]
pub use crate::time::{LedgerEpoch, add_months, add_years, advance_epoch};
#[cfg(feature = "std")]
pub use crate::env::CryptoCurve;
#[cfg(feature = "std")]
pub use crate::env::MockCryptoRegistry;
#[cfg(feature = "std")]
pub use crate::env::MockKeyPair;
#[cfg(feature = "std")]
pub use crate::token::MockToken;
#[cfg(feature = "std")]
pub use crate::profiler::{
    export_flamegraph_svg, export_speedscope, Frame, GasProfiler, GasProfilerResult, Profile,
    Sample,
};
#[cfg(feature = "std")]
pub use crate::zk::{
    G1, G2, Groth16Proof, Groth16VerifyingKey, Gt, PairingCurve, PlonkProof,
};

#[cfg(feature = "std")]
pub use crate::quickcheck::{
    Arbitrary, Config as QuickcheckConfig, SorobanAmount, SorobanI128, SorobanTimestamp,
    SorobanU32,
};

pub use crate::fixture_graph::{AcyclicFixture, FixtureDeps};

/// Re-exported so `#[fixture(requires = [..])]` expansions resolve their
/// generated trait paths wherever the prelude is imported.
pub use crate::fixture_graph as crucible_fixture_graph;

#[cfg(all(feature = "derive", feature = "std"))]
pub use crucible_macros::fixture;
