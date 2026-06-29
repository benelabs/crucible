//! Re-exports for use with `crucible::prelude::*`.
//!
//! This module provides convenient access to all commonly used types
//! and utilities from the crucible testing framework.

pub use crate::account::AccountBuilder;
pub use crate::account::AccountHandle;
pub use crate::cost::CostReport;
pub use crate::env::CapturedEvent;
pub use crate::env::Duration;
pub use crate::env::MockAuthGuard;
pub use crate::env::MockEnv;
pub use crate::env::MockEnvBuilder;
pub use crate::env::Stroops;
pub use crate::sim::PreparedTx;
pub use crate::sim::SimulatedTx;
pub use crate::token::MockToken;

#[cfg(feature = "derive")]
pub use crucible_macros::fixture;
