//! Deterministic ledger state checkpointing and rollback.
//!
//! Multi-stage dry runs — speculative DeFi liquidation paths, branching
//! "what if" simulations, staged migrations — need to try something, look at
//! the result, and then put the ledger back exactly as it was. Rebuilding a
//! whole [`MockEnv`] for each branch is both slow and lossy: contract
//! addresses, registered accounts and token handles all change.
//!
//! [`MockEnv::checkpoint`] captures the current contract data of the live
//! environment and returns a [`CheckpointId`]. [`MockEnv::rollback_to`]
//! restores it. Registered contracts, accounts and tokens are untouched, so
//! every client and handle taken before the checkpoint stays valid afterwards.
//!
//! # Copy-on-write
//!
//! Snapshots share their captured entries through [`Rc`], and the host's own
//! `StorageMap` is a persistent (immutable, structurally shared) map. Taking a
//! checkpoint therefore copies key/value *handles*, not ledger entry payloads,
//! which keeps nested checkpoints cheap even in long simulation trees.
//!
//! # Nesting
//!
//! Checkpoints form a stack. Rolling back to a checkpoint discards every
//! checkpoint taken after it, so the following is a hard error rather than a
//! silent no-op:
//!
//! ```rust,ignore
//! let outer = env.checkpoint();
//! let inner = env.checkpoint();
//! env.rollback_to(outer);   // `inner` is now invalid …
//! env.rollback_to(inner);   // … so this panics.
//! ```
//!
//! # Example
//!
//! ```rust,ignore
//! use crucible::prelude::*;
//!
//! let before = env.checkpoint();
//!
//! // Speculatively execute a liquidation path.
//! vault.liquidate(&borrower.address());
//! assert_eq!(vault.collateral(&borrower.address()), 0);
//!
//! // Put everything back and try a different path.
//! env.rollback_to(before);
//! assert_eq!(vault.collateral(&borrower.address()), 1_000);
//! ```
//!
//! **Host-only:** checkpointing reaches into the Soroban host's storage map and
//! is intended exclusively for `#[cfg(test)]` use on the host.
//!
//! [`MockEnv`]: crate::env::MockEnv
//! [`MockEnv::checkpoint`]: crate::env::MockEnv::checkpoint
//! [`MockEnv::rollback_to`]: crate::env::MockEnv::rollback_to

use soroban_env_host::storage::AccessType;
use soroban_env_host::xdr::{ContractDataDurability, LedgerKey};
use soroban_env_host::Host;
use std::collections::HashSet;
use std::rc::Rc;

/// A ledger key together with the entry stored under it, as the host holds it.
type StoredEntry = (
    Rc<LedgerKey>,
    Option<(
        Rc<soroban_env_host::xdr::LedgerEntry>,
        Option<u32>,
    )>,
);

/// An opaque handle to a ledger state checkpoint.
///
/// Returned by [`MockEnv::checkpoint`](crate::env::MockEnv::checkpoint) and
/// consumed by [`MockEnv::rollback_to`](crate::env::MockEnv::rollback_to).
///
/// Ids are unique within a single environment and are **not** transferable
/// between environments; passing an id to a different `MockEnv` panics rather
/// than restoring unrelated state.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct CheckpointId {
    /// Identifies the owning environment, so foreign ids can be rejected.
    env_id: u64,
    /// Monotonically increasing position within that environment.
    seq: u64,
}

impl CheckpointId {
    /// Returns the sequence number of this checkpoint within its environment.
    ///
    /// Useful for ordering assertions in tests; it carries no other meaning.
    pub fn sequence(&self) -> u64 {
        self.seq
    }
}

impl std::fmt::Display for CheckpointId {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "checkpoint#{}", self.seq)
    }
}

/// How many entries of each durability a checkpoint captured.
///
/// Obtained from [`MockEnv::checkpoint_stats`](crate::env::MockEnv::checkpoint_stats).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct CheckpointStats {
    /// Contract instance entries (`env.storage().instance()`).
    pub instance: usize,
    /// Persistent entries (`env.storage().persistent()`).
    pub persistent: usize,
    /// Temporary entries (`env.storage().temporary()`).
    pub temporary: usize,
    /// Entries that are neither contract data nor contract code — for example
    /// account entries created by the test harness.
    pub other: usize,
}

impl CheckpointStats {
    /// Total number of captured entries.
    pub fn total(&self) -> usize {
        self.instance + self.persistent + self.temporary + self.other
    }
}

/// One captured ledger state, plus the id it was filed under.
struct Snapshot {
    id: CheckpointId,
    /// The full storage map at capture time.
    ///
    /// Wrapped in an [`Rc`] so cloning a snapshot — which happens whenever the
    /// stack is inspected — never copies the entry list itself.
    entries: Rc<Vec<StoredEntry>>,
}

/// The checkpoint stack belonging to one [`MockEnv`](crate::env::MockEnv).
///
/// Held behind an `Rc<RefCell<..>>` in `MockEnv`, so clones of an environment
/// share one stack while [`fork`](crate::env::MockEnv::fork)s get their own.
#[derive(Default)]
pub(crate) struct CheckpointStack {
    /// Identifies this stack's environment, stamped into every id it hands out.
    env_id: u64,
    /// Next sequence number to allocate.
    next_seq: u64,
    /// Live checkpoints, oldest first.
    snapshots: Vec<Snapshot>,
}

impl std::fmt::Debug for CheckpointStack {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("CheckpointStack")
            .field("env_id", &self.env_id)
            .field("depth", &self.snapshots.len())
            .finish()
    }
}

impl CheckpointStack {
    /// Creates an empty stack owned by a freshly allocated environment id.
    pub(crate) fn new() -> Self {
        Self {
            env_id: next_env_id(),
            next_seq: 0,
            snapshots: Vec::new(),
        }
    }

    /// Creates an empty stack that belongs to a *different* environment than
    /// `self`, used when forking so that ids never cross between the two.
    pub(crate) fn forked(&self) -> Self {
        Self::new()
    }

    /// Number of live checkpoints.
    pub(crate) fn depth(&self) -> usize {
        self.snapshots.len()
    }

    /// Captures the current ledger state of `host` and files it under a new id.
    pub(crate) fn capture(&mut self, host: &Host) -> CheckpointId {
        let id = CheckpointId {
            env_id: self.env_id,
            seq: self.next_seq,
        };
        self.next_seq += 1;

        let entries = host
            .get_stored_entries()
            .expect("reading the host storage map cannot fail in the test environment");

        self.snapshots.push(Snapshot {
            id,
            entries: Rc::new(entries),
        });
        id
    }

    /// Returns the entry counts captured by `id`, by durability.
    pub(crate) fn stats(&self, id: CheckpointId) -> CheckpointStats {
        let snapshot = self.lookup(id);
        let mut stats = CheckpointStats::default();
        for (key, _) in snapshot.entries.iter() {
            match key.as_ref() {
                LedgerKey::ContractData(data) => match data.durability {
                    ContractDataDurability::Persistent => {
                        // The instance entry is the one keyed by `ScVal::LedgerKeyContractInstance`.
                        if matches!(
                            data.key,
                            soroban_env_host::xdr::ScVal::LedgerKeyContractInstance
                        ) {
                            stats.instance += 1;
                        } else {
                            stats.persistent += 1;
                        }
                    }
                    ContractDataDurability::Temporary => stats.temporary += 1,
                },
                _ => stats.other += 1,
            }
        }
        stats
    }

    /// Restores `host` to the state captured by `id`.
    ///
    /// Every checkpoint taken after `id` is discarded. `id` itself is kept, so
    /// the same checkpoint can be rolled back to repeatedly — which is what
    /// makes speculative branching from one point practical.
    pub(crate) fn rollback(&mut self, host: &Host, id: CheckpointId) {
        let position = self.position_of(id);

        // Everything above `id` is unreachable once we rewind past it.
        self.snapshots.truncate(position + 1);
        let entries = Rc::clone(&self.snapshots[position].entries);

        // Keys that exist now but did not exist at capture time have to be
        // cleared explicitly; re-inserting the captured entries alone would
        // leave them behind.
        let captured: HashSet<Rc<LedgerKey>> =
            entries.iter().map(|(key, _)| Rc::clone(key)).collect();
        let current = host
            .get_stored_entries()
            .expect("reading the host storage map cannot fail in the test environment");

        for (key, _) in current {
            if !captured.contains(&key) {
                write_entry(host, key, None);
            }
        }
        for (key, value) in entries.iter() {
            write_entry(host, Rc::clone(key), value.clone());
        }
    }

    /// Discards `id` and every checkpoint taken after it, without restoring.
    ///
    /// Use this to release a checkpoint whose branch was accepted rather than
    /// rolled back.
    pub(crate) fn release(&mut self, id: CheckpointId) {
        let position = self.position_of(id);
        self.snapshots.truncate(position);
    }

    /// Returns the snapshot filed under `id`, panicking with a helpful message
    /// if the id is foreign or has already been discarded.
    fn lookup(&self, id: CheckpointId) -> &Snapshot {
        &self.snapshots[self.position_of(id)]
    }

    /// Returns the index of `id` in the stack.
    ///
    /// # Panics
    ///
    /// Panics if `id` came from another environment, or if it was invalidated
    /// by an earlier rollback.
    fn position_of(&self, id: CheckpointId) -> usize {
        assert_eq!(
            id.env_id, self.env_id,
            "{id} belongs to a different MockEnv. Checkpoint ids are not \
             transferable between environments, including across `fork()`."
        );
        self.snapshots
            .iter()
            .position(|snapshot| snapshot.id == id)
            .unwrap_or_else(|| {
                panic!(
                    "{id} is no longer valid: it was discarded by a rollback to \
                     an earlier checkpoint, or released. Live checkpoints: {live:?}.",
                    live = self
                        .snapshots
                        .iter()
                        .map(|snapshot| snapshot.id.seq)
                        .collect::<Vec<_>>(),
                )
            })
    }
}

/// Writes one entry into the host's storage map.
///
/// `None` removes the key. The access type is always `ReadWrite`: the mock
/// environment records rather than enforces footprints, and a restored entry
/// must remain writable for the rest of the test.
fn write_entry(
    host: &Host,
    key: Rc<LedgerKey>,
    value: Option<(Rc<soroban_env_host::xdr::LedgerEntry>, Option<u32>)>,
) {
    host.setup_storage_entry(key, value, AccessType::ReadWrite)
        .expect("writing the host storage map cannot fail in the test environment");
}

/// Allocates a process-unique environment id.
fn next_env_id() -> u64 {
    use std::sync::atomic::{AtomicU64, Ordering};
    static NEXT: AtomicU64 = AtomicU64::new(0);
    NEXT.fetch_add(1, Ordering::Relaxed)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn checkpoint_ids_from_different_stacks_never_collide() {
        let a = CheckpointStack::new();
        let b = CheckpointStack::new();
        assert_ne!(a.env_id, b.env_id);
    }

    #[test]
    fn a_fresh_stack_has_no_checkpoints() {
        assert_eq!(CheckpointStack::new().depth(), 0);
    }

    #[test]
    fn stats_total_sums_every_durability() {
        let stats = CheckpointStats {
            instance: 1,
            persistent: 2,
            temporary: 3,
            other: 4,
        };
        assert_eq!(stats.total(), 10);
    }

    #[test]
    fn checkpoint_ids_render_readably() {
        let id = CheckpointId { env_id: 0, seq: 7 };
        assert_eq!(id.to_string(), "checkpoint#7");
        assert_eq!(id.sequence(), 7);
    }
}
