#[cfg(test)]
mod tests {
    use super::*;
    use crucible::prelude::*;
    use soroban_sdk::testutils::Address as TestAddress;
    use soroban_sdk::{symbol_short, Bytes, Map, RawVal};

    fn setup_env() -> MockEnv {
        MockEnv::builder()
            .with_contract::<SupplyChain>()
            .with_account("alice", Stroops::xlm(10_000))
            .with_account("bob", Stroops::xlm(10_000))
            .build()
    }

    #[test]
    fn test_item_creation_and_query() {
        let env = setup_env();
        env.mock_all_auths();
        // metadata: simple key/value
        let mut meta = Map::<Symbol, RawVal>::new(&env);
        meta.set(&symbol_short!("sku"), &RawVal::from_val(&env, &"ABC123"));
        let id = SupplyChain::create_item(&env, meta.clone());
        let item = SupplyChain::get_item(&env, id);
        assert_eq!(item.creator, env.account("alice").address());
        assert_eq!(item.current_holder, env.account("alice").address());
        assert_eq!(item.status, Status::Created);
        assert_eq!(item.metadata, meta);
    }

    #[test]
    fn test_status_update_authorized() {
        let env = setup_env();
        env.mock_all_auths();
        let id = SupplyChain::create_item(&env, Map::new(&env));
        // alice is current holder
        SupplyChain::update_status(&env, id, Status::InTransit);
        let item = SupplyChain::get_item(&env, id);
        assert_eq!(item.status, Status::InTransit);
    }

    #[test]
    fn test_status_update_unauthorized() {
        let env = setup_env();
        env.mock_all_auths();
        let id = SupplyChain::create_item(&env, Map::new(&env));
        // clear auth so the holder's require_auth() is enforced and fails
        env.mock_auths(&[]);
        assert_reverts!(SupplyChain::update_status(&env, id, Status::InTransit));
    }

    #[test]
    fn test_transfer_holder() {
        let env = setup_env();
        env.mock_all_auths();
        let id = SupplyChain::create_item(&env, Map::new(&env));
        let bob_addr = env.account("bob").address();
        SupplyChain::transfer_holder(&env, id, bob_addr.clone());
        let item = SupplyChain::get_item(&env, id);
        assert_eq!(item.current_holder, bob_addr);
    }

    #[test]
    fn test_transfer_unauthorized() {
        let env = setup_env();
        env.mock_all_auths();
        let id = SupplyChain::create_item(&env, Map::new(&env));
        // clear auth so the holder's require_auth() is enforced and fails
        env.mock_auths(&[]);
        let bob_addr = env.account("bob").address();
        assert_reverts!(SupplyChain::transfer_holder(&env, id, bob_addr));
    }
}
