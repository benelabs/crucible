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
        evs: Vec<CapturedEvent>,
    ) -> Vec<(
        soroban_sdk::Address,
        soroban_sdk::Vec<soroban_sdk::Val>,
        soroban_sdk::Val,
    )> {
        evs.into_iter()
            .map(|e| (e.contract, e.topics, e.data))
            .collect()
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
            map_parsed(parsed),
            "events_matching and events_parsed must return identical matches for the same topic filter"
        );
    }
}
