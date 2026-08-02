//! Assertion macros for Soroban contract testing.
//!
//! These macros provide ergonomic assertions for common test patterns:
//! - `assert_reverts!` — assert a contract call panics (reverts)
//! - `assert_emitted!` — assert a specific event was emitted
//! - `assert_not_emitted!` — assert no events were emitted
//! - `assert_approx_eq!` — assert approximate numeric equality within tolerance

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
            "assert_reverts! failed: the expression did not revert (panic).\n\
             \n\
             Expression: {expr}",
            expr = stringify!($expr),
        );
    }};
    ($expr:expr, $msg:literal) => {{
        extern crate std;
        let __result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
            $expr;
        }));
        assert!(
            __result.is_err(),
            "assert_reverts! failed: the expression did not revert (panic).\n\
             \n\
             Expression : {expr}\n\
             Context    : {ctx}",
            expr = stringify!($expr),
            ctx = $msg,
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
        extern crate std;
        use soroban_sdk::testutils::Events as _;
        use soroban_sdk::IntoVal as _;
        use soroban_sdk::TryFromVal as _;
        use std::string::ToString as _;
        let __env = $env.inner();
        let __all = __env.events().all();
        let __want_contract: soroban_sdk::Address = $contract_id.clone();
        let __want_topics: soroban_sdk::Vec<soroban_sdk::Val> = ($topics).into_val(__env);
        let __want_data: soroban_sdk::Val = ($data).into_val(__env);
        let __want_data_xdr = soroban_sdk::xdr::ScVal::try_from_val(__env, &__want_data).unwrap();
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
            "assert_emitted! failed: expected event was not found.\n\
             \n\
             Contract : {contract:?}\n\
             Topics   : {topics:?}\n\
             Data     : {data:?}\n\
             \n\
             Events emitted by this contract ({count}):\n\
             {actual}",
            contract = __want_contract,
            topics = __want_topics,
            data = __want_data_xdr,
            count = __filtered.events().len(),
            actual = {
                let lines: std::vec::Vec<std::string::String> = __filtered
                    .events()
                    .iter()
                    .enumerate()
                    .map(|(i, ev)| {
                        let soroban_sdk::xdr::ContractEventBody::V0(ref body) = ev.body;
                        std::format!("  [{i}] topics={:?} data={:?}", body.topics, body.data)
                    })
                    .collect();
                if lines.is_empty() {
                    "  (none)".to_string()
                } else {
                    lines.join("\n")
                }
            },
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
        extern crate std;
        use soroban_sdk::testutils::Events as _;
        use std::string::ToString as _;
        let __events = $env.inner().events().all();
        assert!(
            __events.events().is_empty(),
            "assert_not_emitted! failed: expected no events, but {count} event(s) were emitted.\n\
             \n\
             Emitted events:\n\
             {list}",
            count = __events.events().len(),
            list = {
                let lines: std::vec::Vec<std::string::String> = __events
                    .events()
                    .iter()
                    .enumerate()
                    .map(|(i, ev)| {
                        let soroban_sdk::xdr::ContractEventBody::V0(ref body) = ev.body;
                        std::format!(
                            "  [{i}] contract={:?} topics={:?} data={:?}",
                            ev.contract_id,
                            body.topics,
                            body.data
                        )
                    })
                    .collect();
                lines.join("\n")
            },
        );
    }};
}

/// Asserts two numeric values are approximately equal within a tolerance.
///
/// This is useful for fee and reward calculations where rounding can produce
/// small expected deltas.
///
/// # Example
///
/// ```ignore
/// crate::assert_approx_eq!(100_i128, 101_i128, 1_i128);
/// crate::assert_approx_eq!(10.0_f64, 10.01_f64, 0.02_f64);
/// ```
#[macro_export]
macro_rules! assert_approx_eq {
    ($actual:expr, $expected:expr, $tolerance:expr) => {{
        let __actual = $actual;
        let __expected = $expected;
        let __tolerance = $tolerance;
        let __zero = __tolerance - __tolerance;

        assert!(
            __tolerance >= __zero,
            "assert_approx_eq! failed: tolerance must be non-negative.\n\
             \n\
             actual    = {:?}\n\
             expected  = {:?}\n\
             tolerance = {:?}",
            __actual,
            __expected,
            __tolerance,
        );

        let __diff = if __actual >= __expected {
            __actual - __expected
        } else {
            __expected - __actual
        };

        assert!(
            __diff <= __tolerance,
            "assert_approx_eq! failed: difference exceeds tolerance.\n\
             \n\
             actual     = {:?}\n\
             expected   = {:?}\n\
             difference = {:?}\n\
             tolerance  = {:?}",
            __actual,
            __expected,
            __diff,
            __tolerance,
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

    #[test]
    fn test_assert_approx_eq_accepts_values_within_tolerance() {
        crate::assert_approx_eq!(100_i128, 102_i128, 2_i128);
        crate::assert_approx_eq!(10.0_f64, 10.01_f64, 0.02_f64);
    }

    #[test]
    fn test_assert_approx_eq_panics_when_outside_tolerance() {
        let result = std::panic::catch_unwind(|| {
            crate::assert_approx_eq!(100_i128, 105_i128, 2_i128);
        });
        assert!(result.is_err());
    }
}
