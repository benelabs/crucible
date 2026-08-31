//! Regression tests for the ledger checkpoint and rollback engine.

#[cfg(test)]
mod tests {
    use crate::env::MockEnv;
    use soroban_sdk::{contract, contractimpl, symbol_short, Env, Symbol};

    /// A store exercising all three storage durabilities.
    #[contract]
    #[derive(Default)]
    struct Store;

    #[contractimpl]
    impl Store {
        pub fn set_instance(env: Env, key: Symbol, value: u32) {
            env.storage().instance().set(&key, &value);
        }

        pub fn get_instance(env: Env, key: Symbol) -> u32 {
            env.storage().instance().get(&key).unwrap_or(0)
        }

        pub fn has_instance(env: Env, key: Symbol) -> bool {
            env.storage().instance().has(&key)
        }

        pub fn set_persistent(env: Env, key: Symbol, value: u32) {
            env.storage().persistent().set(&key, &value);
        }

        pub fn get_persistent(env: Env, key: Symbol) -> u32 {
            env.storage().persistent().get(&key).unwrap_or(0)
        }

        pub fn has_persistent(env: Env, key: Symbol) -> bool {
            env.storage().persistent().has(&key)
        }

        pub fn set_temporary(env: Env, key: Symbol, value: u32) {
            env.storage().temporary().set(&key, &value);
        }

        pub fn get_temporary(env: Env, key: Symbol) -> u32 {
            env.storage().temporary().get(&key).unwrap_or(0)
        }
    }

    /// A second contract type, so cross-contract isolation can be asserted.
    #[contract]
    #[derive(Default)]
    struct OtherStore;

    #[contractimpl]
    impl OtherStore {
        pub fn set_persistent(env: Env, key: Symbol, value: u32) {
            env.storage().persistent().set(&key, &value);
        }

        pub fn get_persistent(env: Env, key: Symbol) -> u32 {
            env.storage().persistent().get(&key).unwrap_or(0)
        }
    }

    fn store_env() -> MockEnv {
        MockEnv::builder()
            .with_contract::<Store>()
            .with_contract::<OtherStore>()
            .build()
    }

    fn store(env: &MockEnv) -> StoreClient<'_> {
        StoreClient::new(env.inner(), &env.contract_id::<Store>())
    }

    fn other(env: &MockEnv) -> OtherStoreClient<'_> {
        OtherStoreClient::new(env.inner(), &env.contract_id::<OtherStore>())
    }

    #[test]
    fn rollback_restores_an_overwritten_instance_entry() {
        let env = store_env();
        let client = store(&env);
        let k = symbol_short!("k");

        client.set_instance(&k, &1);
        let before = env.checkpoint();

        client.set_instance(&k, &2);
        assert_eq!(client.get_instance(&k), 2);

        env.rollback_to(before);
        assert_eq!(client.get_instance(&k), 1);
    }

    #[test]
    fn rollback_restores_persistent_and_temporary_entries_too() {
        let env = store_env();
        let client = store(&env);
        let k = symbol_short!("k");

        client.set_persistent(&k, &10);
        client.set_temporary(&k, &100);
        let before = env.checkpoint();

        client.set_persistent(&k, &20);
        client.set_temporary(&k, &200);
        assert_eq!(client.get_persistent(&k), 20);
        assert_eq!(client.get_temporary(&k), 200);

        env.rollback_to(before);
        assert_eq!(client.get_persistent(&k), 10);
        assert_eq!(client.get_temporary(&k), 100);
    }

    #[test]
    fn rollback_removes_entries_created_after_the_checkpoint() {
        let env = store_env();
        let client = store(&env);
        let existing = symbol_short!("old");
        let created = symbol_short!("new");

        client.set_persistent(&existing, &1);
        let before = env.checkpoint();

        client.set_persistent(&created, &2);
        assert!(client.has_persistent(&created));

        env.rollback_to(before);
        assert!(
            !client.has_persistent(&created),
            "an entry created after the checkpoint must not survive the rollback"
        );
        assert!(client.has_persistent(&existing));
    }

    #[test]
    fn rollback_removes_instance_entries_created_after_the_checkpoint() {
        let env = store_env();
        let client = store(&env);
        let created = symbol_short!("new");

        let before = env.checkpoint();
        client.set_instance(&created, &7);
        assert!(client.has_instance(&created));

        env.rollback_to(before);
        assert!(!client.has_instance(&created));
    }

    #[test]
    fn the_same_checkpoint_can_be_rolled_back_to_repeatedly() {
        let env = store_env();
        let client = store(&env);
        let k = symbol_short!("k");

        client.set_persistent(&k, &1);
        let before = env.checkpoint();

        // Branch one.
        client.set_persistent(&k, &2);
        env.rollback_to(before);
        assert_eq!(client.get_persistent(&k), 1);

        // Branch two, from the very same point.
        client.set_persistent(&k, &3);
        env.rollback_to(before);
        assert_eq!(client.get_persistent(&k), 1);
    }

    #[test]
    fn nested_checkpoints_unwind_one_level_at_a_time() {
        let env = store_env();
        let client = store(&env);
        let k = symbol_short!("k");

        client.set_persistent(&k, &0);
        let outer = env.checkpoint();
        client.set_persistent(&k, &1);
        let inner = env.checkpoint();
        client.set_persistent(&k, &2);

        assert_eq!(env.checkpoint_depth(), 2);

        env.rollback_to(inner);
        assert_eq!(client.get_persistent(&k), 1);

        env.rollback_to(outer);
        assert_eq!(client.get_persistent(&k), 0);
    }

    #[test]
    fn rolling_back_to_an_outer_checkpoint_discards_the_inner_ones() {
        let env = store_env();
        let client = store(&env);
        let k = symbol_short!("k");

        let outer = env.checkpoint();
        client.set_persistent(&k, &1);
        let _inner = env.checkpoint();
        assert_eq!(env.checkpoint_depth(), 2);

        env.rollback_to(outer);
        assert_eq!(
            env.checkpoint_depth(),
            1,
            "the inner checkpoint described state that no longer exists"
        );
    }

    #[test]
    #[should_panic(expected = "no longer valid")]
    fn a_discarded_checkpoint_cannot_be_rolled_back_to() {
        let env = store_env();
        let outer = env.checkpoint();
        let inner = env.checkpoint();

        env.rollback_to(outer);
        env.rollback_to(inner);
    }

    #[test]
    #[should_panic(expected = "belongs to a different MockEnv")]
    fn a_checkpoint_from_another_environment_is_rejected() {
        let a = store_env();
        let b = store_env();
        let id = a.checkpoint();
        b.rollback_to(id);
    }

    #[test]
    #[should_panic(expected = "belongs to a different MockEnv")]
    fn a_fork_does_not_honour_its_parents_checkpoints() {
        let env = store_env();
        let id = env.checkpoint();
        env.fork().rollback_to(id);
    }

    #[test]
    fn rollback_isolates_state_across_contracts() {
        let env = store_env();
        let store_client = store(&env);
        let other_client = other(&env);
        let k = symbol_short!("k");

        store_client.set_persistent(&k, &1);
        other_client.set_persistent(&k, &10);
        let before = env.checkpoint();

        // Both contracts move.
        store_client.set_persistent(&k, &2);
        other_client.set_persistent(&k, &20);
        assert_eq!(store_client.get_persistent(&k), 2);
        assert_eq!(other_client.get_persistent(&k), 20);

        // One rollback restores both, and neither contract's key leaks into
        // the other despite sharing the same symbol.
        env.rollback_to(before);
        assert_eq!(store_client.get_persistent(&k), 1);
        assert_eq!(other_client.get_persistent(&k), 10);
    }

    #[test]
    fn a_checkpoint_taken_between_two_contracts_writes_only_rolls_back_the_later_one() {
        let env = store_env();
        let store_client = store(&env);
        let other_client = other(&env);
        let k = symbol_short!("k");

        store_client.set_persistent(&k, &1);
        let before = env.checkpoint();
        other_client.set_persistent(&k, &10);

        env.rollback_to(before);
        assert_eq!(
            store_client.get_persistent(&k),
            1,
            "a write made before the checkpoint must survive"
        );
        assert_eq!(
            other_client.get_persistent(&k),
            0,
            "a write made after the checkpoint must be undone"
        );
    }

    #[test]
    fn clients_taken_before_a_checkpoint_stay_usable_afterwards() {
        let env = store_env();
        let client = store(&env);
        let k = symbol_short!("k");

        let before = env.checkpoint();
        client.set_persistent(&k, &5);
        env.rollback_to(before);

        // The contract registration is not part of the snapshot, so the client
        // built before the rollback still addresses a live contract.
        client.set_persistent(&k, &6);
        assert_eq!(client.get_persistent(&k), 6);
    }

    #[test]
    fn releasing_a_checkpoint_keeps_the_work_and_drops_the_snapshot() {
        let env = store_env();
        let client = store(&env);
        let k = symbol_short!("k");

        let before = env.checkpoint();
        client.set_persistent(&k, &42);
        env.release_checkpoint(before);

        assert_eq!(env.checkpoint_depth(), 0);
        assert_eq!(client.get_persistent(&k), 42);
    }

    #[test]
    fn speculate_discards_writes_but_returns_the_value() {
        let env = store_env();
        let client = store(&env);
        let k = symbol_short!("k");

        client.set_persistent(&k, &1);

        let observed = env.speculate(|| {
            client.set_persistent(&k, &2);
            client.get_persistent(&k)
        });

        assert_eq!(observed, 2, "the closure sees its own writes");
        assert_eq!(client.get_persistent(&k), 1, "but they do not survive");
        assert_eq!(env.checkpoint_depth(), 0);
    }

    #[test]
    fn speculate_rolls_back_even_when_the_closure_panics() {
        let env = store_env();
        let client = store(&env);
        let k = symbol_short!("k");

        client.set_persistent(&k, &1);

        let outcome = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
            env.speculate(|| {
                client.set_persistent(&k, &2);
                panic!("speculative branch failed");
            })
        }));

        assert!(outcome.is_err(), "the panic must propagate to the caller");
        assert_eq!(client.get_persistent(&k), 1);
        assert_eq!(env.checkpoint_depth(), 0);
    }

    #[test]
    fn stats_report_entries_by_durability() {
        let env = store_env();
        let client = store(&env);

        client.set_instance(&symbol_short!("i"), &1);
        client.set_persistent(&symbol_short!("p1"), &1);
        client.set_persistent(&symbol_short!("p2"), &2);
        client.set_temporary(&symbol_short!("t"), &1);

        let id = env.checkpoint();
        let stats = env.checkpoint_stats(id);

        // Two contracts are registered, each with an instance entry.
        assert_eq!(stats.instance, 2);
        assert_eq!(stats.persistent, 2);
        assert_eq!(stats.temporary, 1);
        assert_eq!(stats.total(), stats.instance + stats.persistent + stats.temporary + stats.other);

        env.release_checkpoint(id);
    }

    #[test]
    fn checkpoint_ids_are_ordered_by_creation() {
        let env = store_env();
        let first = env.checkpoint();
        let second = env.checkpoint();
        assert!(second.sequence() > first.sequence());
    }

    #[test]
    fn a_deep_checkpoint_tree_unwinds_correctly() {
        let env = store_env();
        let client = store(&env);
        let k = symbol_short!("k");

        // Build a chain of ten checkpoints, writing a distinct value at each.
        let mut ids = Vec::new();
        for value in 0..10_u32 {
            ids.push(env.checkpoint());
            client.set_persistent(&k, &value);
        }
        assert_eq!(env.checkpoint_depth(), 10);
        assert_eq!(client.get_persistent(&k), 9);

        // Unwind from the innermost outwards; each level restores the value
        // that was current when that checkpoint was taken.
        for value in (1..10_u32).rev() {
            env.rollback_to(ids[value as usize]);
            assert_eq!(client.get_persistent(&k), value - 1);
        }

        // The outermost checkpoint predates every write.
        env.rollback_to(ids[0]);
        assert_eq!(client.get_persistent(&k), 0);
        assert_eq!(env.checkpoint_depth(), 1);
    }
}
