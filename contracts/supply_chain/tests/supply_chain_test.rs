#[cfg(test)]
mod tests {
    use supply_chain::*;
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
    fn test_valid_status_transition_sequence() {
        let env = setup_env();
        env.mock_all_auths();
        let id = SupplyChain::create_item(&env, Map::new(&env));
        assert_eq!(SupplyChain::get_item(&env, id).status, Status::Created);

        // Created -> InTransit
        SupplyChain::update_status(&env, id, Status::InTransit);
        assert_eq!(SupplyChain::get_item(&env, id).status, Status::InTransit);

        // InTransit -> Received
        SupplyChain::update_status(&env, id, Status::Received);
        assert_eq!(SupplyChain::get_item(&env, id).status, Status::Received);

        // Received -> Inspected
        SupplyChain::update_status(&env, id, Status::Inspected);
        assert_eq!(SupplyChain::get_item(&env, id).status, Status::Inspected);

        // Inspected -> Completed
        SupplyChain::update_status(&env, id, Status::Completed);
        assert_eq!(SupplyChain::get_item(&env, id).status, Status::Completed);
    }

    #[test]
    fn test_valid_status_transition_to_rejected() {
        let env = setup_env();
        env.mock_all_auths();
        let id = SupplyChain::create_item(&env, Map::new(&env));

        SupplyChain::update_status(&env, id, Status::InTransit);
        SupplyChain::update_status(&env, id, Status::Received);
        SupplyChain::update_status(&env, id, Status::Inspected);

        // Inspected -> Rejected
        SupplyChain::update_status(&env, id, Status::Rejected);
        assert_eq!(SupplyChain::get_item(&env, id).status, Status::Rejected);
    }

    #[test]
    fn test_invalid_status_transition_created_to_completed() {
        let env = setup_env();
        env.mock_all_auths();
        let id = SupplyChain::create_item(&env, Map::new(&env));

        // Created directly to Completed must revert
        assert_reverts!(SupplyChain::update_status(&env, id, Status::Completed));
    }

    #[test]
    fn test_invalid_status_transition_created_to_received() {
        let env = setup_env();
        env.mock_all_auths();
        let id = SupplyChain::create_item(&env, Map::new(&env));

        // Created directly to Received must revert
        assert_reverts!(SupplyChain::update_status(&env, id, Status::Received));
    }

    #[test]
    fn test_invalid_status_transition_rejected_to_intransit() {
        let env = setup_env();
        env.mock_all_auths();
        let id = SupplyChain::create_item(&env, Map::new(&env));

        SupplyChain::update_status(&env, id, Status::InTransit);
        SupplyChain::update_status(&env, id, Status::Received);
        SupplyChain::update_status(&env, id, Status::Inspected);
        SupplyChain::update_status(&env, id, Status::Rejected);

        // Rejected back to InTransit must revert
        assert_reverts!(SupplyChain::update_status(&env, id, Status::InTransit));
    }

    #[test]
    fn test_invalid_status_transition_completed_is_terminal() {
        let env = setup_env();
        env.mock_all_auths();
        let id = SupplyChain::create_item(&env, Map::new(&env));

        SupplyChain::update_status(&env, id, Status::InTransit);
        SupplyChain::update_status(&env, id, Status::Received);
        SupplyChain::update_status(&env, id, Status::Inspected);
        SupplyChain::update_status(&env, id, Status::Completed);

        // Completed cannot transition to InTransit or Created
        assert_reverts!(SupplyChain::update_status(&env, id, Status::InTransit));
        assert_reverts!(SupplyChain::update_status(&env, id, Status::Created));
    }

    #[test]
    fn test_transition_validator_logic() {
        // Valid transitions
        assert!(Status::Created.is_valid_transition(&Status::InTransit));
        assert!(Status::InTransit.is_valid_transition(&Status::Received));
        assert!(Status::Received.is_valid_transition(&Status::Inspected));
        assert!(Status::Inspected.is_valid_transition(&Status::Completed));
        assert!(Status::Inspected.is_valid_transition(&Status::Rejected));

        // Associated function on SupplyChain
        assert!(SupplyChain::is_valid_transition(
            &Status::Created,
            &Status::InTransit
        ));

        // Invalid transitions
        assert!(!Status::Created.is_valid_transition(&Status::Completed));
        assert!(!Status::Created.is_valid_transition(&Status::Received));
        assert!(!Status::Created.is_valid_transition(&Status::Inspected));
        assert!(!Status::Created.is_valid_transition(&Status::Rejected));
        assert!(!Status::Created.is_valid_transition(&Status::Created));

        assert!(!Status::InTransit.is_valid_transition(&Status::Created));
        assert!(!Status::InTransit.is_valid_transition(&Status::Completed));
        assert!(!Status::InTransit.is_valid_transition(&Status::Rejected));

        assert!(!Status::Received.is_valid_transition(&Status::InTransit));
        assert!(!Status::Received.is_valid_transition(&Status::Completed));

        assert!(!Status::Completed.is_valid_transition(&Status::InTransit));
        assert!(!Status::Rejected.is_valid_transition(&Status::InTransit));
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
