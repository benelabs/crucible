#[cfg(test)]
mod tests {
    use crate::env::{CapturedEvent, MockEnv};
    use soroban_sdk::{contract, contractimpl, symbol_short, Env};

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
}
