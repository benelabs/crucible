//! Assertion macros for Soroban contract testing.
//!
//! These macros provide ergonomic assertions for common test patterns:
//! - `assert_reverts!` — assert a contract call panics (reverts)
//! - `assert_emitted!` — assert a specific event was emitted
//! - `assert_not_emitted!` — assert no events were emitted

/// Asserts that a contract invocation panics (reverts).
///
/// In Soroban's test environment, contract errors manifest as panics.
/// This macro wraps the expression in [`std::panic::catch_unwind`] and
/// asserts the panic occurred.
///
/// # Example
///
/// ```ignore
/// assert_reverts!(client.transfer(&alice, &bob, &(-1_i128)));
/// assert_reverts!(client.claim(), "too early");
/// ```
#[macro_export]
macro_rules! assert_reverts {
    ($expr:expr) => {{
        extern crate std;
        let __result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
            $expr;
        }));
        assert!(
            __result.is_err(),
            "Expected contract call to revert (panic), but it succeeded"
        );
    }};
    ($expr:expr, $msg:literal) => {{
        extern crate std;
        let __result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
            $expr;
        }));
        assert!(
            __result.is_err(),
            concat!(
                "Expected contract call to revert, but it succeeded. Context: ",
                $msg
            )
        );
    }};
}

/// Asserts that a specific event was emitted (among any others).
///
/// Searches the event log for at least one entry matching the given contract
/// address, topics tuple, and data value. Other events may also be present.
/// Topics are passed as a tuple and converted to `Vec<Val>` via
/// [`soroban_sdk::IntoVal`].
///
/// # Example
///
/// ```ignore
/// client.increment();
/// assert_emitted!(
///     env,
///     contract_id,
///     (symbol_short!("incr"),),
///     1_u32
/// );
/// ```
#[macro_export]
macro_rules! assert_emitted {
    ($env:expr, $contract_id:expr, $topics:expr, $data:expr) => {{
        use soroban_sdk::testutils::Events as _;
        use soroban_sdk::IntoVal as _;
        use soroban_sdk::TryFromVal as _;
        let __env = $env.inner();
        let __all = __env.events().all();
        let __want_contract: soroban_sdk::Address = $contract_id.clone();
        let __want_topics: soroban_sdk::Vec<soroban_sdk::Val> = ($topics).into_val(__env);
        let __want_data: soroban_sdk::Val = ($data).into_val(__env);
        let __want_data_xdr =
            soroban_sdk::xdr::ScVal::try_from_val(__env, &__want_data).unwrap();
        let __want_topics_xdr: soroban_sdk::xdr::VecM<soroban_sdk::xdr::ScVal> = __want_topics
            .iter()
            .map(|v| soroban_sdk::xdr::ScVal::try_from_val(__env, &v).unwrap())
            .collect::<std::vec::Vec<_>>()
            .try_into()
            .unwrap();
        let __filtered = __all.filter_by_contract(&__want_contract);
        let __found = __filtered.events().iter().any(|ev| {
            let soroban_sdk::xdr::ContractEventBody::V0(ref body) = ev.body;
            body.topics == __want_topics_xdr && body.data == __want_data_xdr
        });
        assert!(
            __found,
            "Expected event not found.\n  contract: {:?}\n  topics:   {:?}\n  data:     {:?}\n  actual events: {:?}",
            __want_contract,
            __want_topics,
            __want_data_xdr,
            __all,
        );
    }};
}

/// Asserts that no events were emitted.
///
/// # Example
///
/// ```ignore
/// client.get(); // read-only, no events
/// assert_not_emitted!(env);
/// ```
#[macro_export]
macro_rules! assert_not_emitted {
    ($env:expr) => {{
        use soroban_sdk::testutils::Events as _;
        let __events = $env.inner().events().all();
        assert!(
            __events.events().is_empty(),
            "Expected no events to be emitted, but {} were emitted. Events: {:?}",
            __events.events().len(),
            __events
        );
    }};
}

#[cfg(test)]
mod tests {
    use crate::env::MockEnv;
    use soroban_sdk::{contract, contractimpl, symbol_short, Env};

    // A minimal contract that publishes two events in one call.
    #[contract]
    #[derive(Default)]
    struct MultiEventContract;

    #[contractimpl]
    impl MultiEventContract {
        pub fn fire_two(env: Env) {
            env.events().publish((symbol_short!("first"),), 1_u32);
            env.events().publish((symbol_short!("second"),), 2_u32);
        }
    }

    #[test]
    fn test_assert_emitted_finds_event_among_others() {
        let env = MockEnv::builder()
            .with_contract::<MultiEventContract>()
            .build();
        let id = env.contract_id::<MultiEventContract>();
        let client = MultiEventContractClient::new(env.inner(), &id);

        client.fire_two();

        // Each event should be found even though two events are present.
        crate::assert_emitted!(env, id, (symbol_short!("first"),), 1_u32);
        crate::assert_emitted!(env, id, (symbol_short!("second"),), 2_u32);
    }
}
