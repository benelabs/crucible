#[cfg(test)]
mod tests {
    use crate::env::{CapturedEvent, MockEnv};
    use soroban_sdk::{contract, contractimpl, symbol_short, Env, FromVal as _};

    #[contract]
    #[derive(Default)]
    struct TopicEventContract;

    #[contractimpl]
    impl TopicEventContract {
        pub fn emit_two(env: Env) {
            env.events()
                .publish((symbol_short!("a"), symbol_short!("b")), 1_u32);
            env.events()
                .publish((symbol_short!("a"), symbol_short!("c")), 2_u32);
        }
    }

    fn map_parsed(
        env: &Env,
        evs: std::vec::Vec<CapturedEvent>,
    ) -> soroban_sdk::Vec<(
        soroban_sdk::Address,
        soroban_sdk::Vec<soroban_sdk::Val>,
        soroban_sdk::Val,
    )> {
        let mut out = soroban_sdk::Vec::new(env);
        for e in evs {
            out.push_back((e.contract, e.topics, e.data));
        }
        out
    }

    #[test]
    fn events_matching_and_events_parsed_agree_on_topic_filters() {
        let env = MockEnv::builder()
            .with_contract::<TopicEventContract>()
            .build();
        let id = env.contract_id::<TopicEventContract>();
        let client = TopicEventContractClient::new(env.inner(), &id);

        client.emit_two();

        // Filter selecting the second topic value.
        let filter = (symbol_short!("a"), symbol_short!("c"));

        let matching = env.events_matching(filter.clone());
        let parsed = env.events_parsed(filter);

        assert_eq!(
            matching,
            map_parsed(env.inner(), parsed),
            "events_matching and events_parsed must return identical matches for the same topic filter"
        );
    }

    #[test]
    fn events_matching_ergonomics_helpers() {
        let env = MockEnv::builder()
            .with_contract::<TopicEventContract>()
            .build();
        let id = env.contract_id::<TopicEventContract>();
        let client = TopicEventContractClient::new(env.inner(), &id);

        client.emit_two();

        let all_a_events = env.events_matching((symbol_short!("a"),));
        all_a_events.assert_count(2);
        all_a_events.assert_emitted();

        assert_eq!(all_a_events.len(), 2);
        assert!(!all_a_events.is_empty());

        let first = all_a_events
            .first_event()
            .expect("first event should exist");
        let last = all_a_events.last_event().expect("last event should exist");
        assert_eq!(first.0, id);
        assert_eq!(last.0, id);

        // Test typed decoding helpers
        let data_0: u32 = all_a_events.data_as(0);
        let data_1: u32 = all_a_events.data_as(1);
        assert_eq!(data_0, 1_u32);
        assert_eq!(data_1, 2_u32);

        let topic_0_1: soroban_sdk::Symbol = all_a_events.topic_as(0, 1);
        let topic_1_1: soroban_sdk::Symbol = all_a_events.topic_as(1, 1);
        assert_eq!(topic_0_1, symbol_short!("b"));
        assert_eq!(topic_1_1, symbol_short!("c"));

        // Test by_contract filter
        let filtered = all_a_events.by_contract(&id);
        assert_eq!(filtered.len(), 2);

        // Test conversion to captured events
        let captured = all_a_events.to_captured();
        assert_eq!(captured.len(), 2);
        assert_eq!(captured[0].contract, id);
        let decoded: u32 = captured[0].data_as();
        assert_eq!(decoded, 1_u32);
    }

    #[contract]
    #[derive(Default)]
    struct TokenEventContract;

    #[contractimpl]
    impl TokenEventContract {
        pub fn transfer(env: Env, from: soroban_sdk::Address, to: soroban_sdk::Address, amount: i128) {
            env.events().publish((symbol_short!("transfer"), from, to), amount);
        }

        pub fn mint(env: Env, to: soroban_sdk::Address, amount: i128) {
            env.events().publish((symbol_short!("mint"), to), amount);
        }

        pub fn burn(env: Env, from: soroban_sdk::Address, amount: i128) {
            env.events().publish((symbol_short!("burn"), from), amount);
        }
    }

    #[test]
    fn test_wildcard_topic_matching_and_macro_assertions() {
        let env = MockEnv::builder()
            .with_contract::<TokenEventContract>()
            .build();
        let id = env.contract_id::<TokenEventContract>();
        let client = TokenEventContractClient::new(env.inner(), &id);

        let alice = crate::account::AccountBuilder::new(&env).name("alice").build();
        let bob = crate::account::AccountBuilder::new(&env).name("bob").build();

        // The Soroban test host exposes only the most recent invocation's
        // events through `events().all()`, so each call is asserted in turn
        // rather than querying a cumulative log at the end.

        client.mint(&alice.address(), &1000_i128);

        // Wildcard match using the `_` symbol.
        let mint_wildcards = env.events_parsed((symbol_short!("mint"), symbol_short!("_")));
        assert_eq!(mint_wildcards.len(), 1);
        assert_eq!(mint_wildcards[0].data_as::<i128>(), 1000_i128);

        client.transfer(&alice.address(), &bob.address(), &300_i128);

        // Wildcard match for any transfer event regardless of sender/receiver.
        let transfer_wildcards = env.events_parsed((
            symbol_short!("transfer"),
            symbol_short!("_"),
            symbol_short!("_"),
        ));
        assert_eq!(transfer_wildcards.len(), 1);
        let ev = &transfer_wildcards[0];
        assert_eq!(ev.contract, id);
        assert_eq!(ev.data_as::<i128>(), 300_i128);
        assert_eq!(
            ev.topic_as::<soroban_sdk::Symbol>(0),
            Some(symbol_short!("transfer"))
        );
        assert_eq!(ev.topic_as::<soroban_sdk::Address>(1), Some(alice.address()));
        assert_eq!(ev.topic_as::<soroban_sdk::Address>(2), Some(bob.address()));

        // Schema validation assertions.
        assert!(ev.assert_schema(3, |data| {
            i128::from_val(env.inner(), data) == 300_i128
        }));

        // A concrete topic segment still has to match exactly alongside a wildcard.
        crate::assert_event_matches!(
            env,
            id,
            (symbol_short!("transfer"), symbol_short!("_"), bob.address())
        );
        crate::assert_event_matches!(
            env,
            id,
            (symbol_short!("transfer"), alice.address(), symbol_short!("_")),
            300_i128
        );

        client.burn(&bob.address(), &50_i128);

        crate::assert_event_matches!(
            env,
            id,
            (symbol_short!("burn"), symbol_short!("_")),
            50_i128
        );
    }
}

