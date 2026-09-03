//! Re-exports for use with `crucible::prelude::*`.
//!
//! This module provides convenient access to all commonly used types
//! and utilities from the crucible testing framework.

pub use crate::account::AccountBuilder;
pub use crate::account::AccountHandle;
pub use crate::assertions::RevertAssertion;
pub use crate::auth_tree::{
    AuthMismatch, AuthTreeReport, ExpectedAuth, ExpectedInvocation, verify_auth_tree,
};
pub use crate::call_graph::{CallFrame, CallTrace};
pub use crate::checkpoint::{CheckpointId, CheckpointStats};
pub use crate::cost::CostReport;
pub use crate::env::CapturedEvent;
pub use crate::env::Duration;
pub use crate::env::EntryTtlSettings;
pub use crate::env::DEFAULT_SECONDS_PER_LEDGER;
pub use crate::env::EventMatches;
pub use crate::env::FailedCallResult;
pub use crate::env::MockAuthGuard;
pub use crate::env::MockEnv;
pub use crate::env::MockEnvBuilder;
pub use crate::env::ProtocolVersion;
pub use crate::env::Stroops;
pub use crate::sim::ContractError;
pub use crate::sim::IngressLock;
pub use crate::sim::IngressLockValidator;
pub use crate::sim::PreparedTx;
pub use crate::sim::ReentrancyProbe;
pub use crate::sim::ReentrancyProbeResult;
pub use crate::sim::SimulatedTx;
pub use crate::time::{add_months, add_years};
pub use crate::env::CryptoCurve;
pub use crate::env::MockCryptoRegistry;
pub use crate::env::MockKeyPair;
pub use crate::token::MockToken;
pub use crate::profiler::{export_flamegraph_svg, export_speedscope, Frame, GasProfiler, Profile, Sample};
pub use crate::zk::{
    G1, G2, Groth16Proof, Groth16VerifyingKey, Gt, PairingCurve, PlonkProof,
};

pub use crate::quickcheck::{
    Arbitrary, Config as QuickcheckConfig, SorobanAmount, SorobanI128, SorobanTimestamp,
    SorobanU32,
};

#[cfg(feature = "derive")]
pub use crucible_macros::fixture;
