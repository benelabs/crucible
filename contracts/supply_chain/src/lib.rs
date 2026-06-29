#![no_std]
use soroban_sdk::{contract, contractimpl, contracttype, Env, Symbol, Address, Map, RawVal, panic_with_error, panic};

/// Unique identifier for an item in the supply chain.
pub type ItemId = u64;

/// Enumerated status of a tracked item.
#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord, Hash, contracttype)]
pub enum Status {
    Created,
    InTransit,
    Received,
    Inspected,
    Completed,
    Rejected,
}

/// Data stored for each item.
#[derive(Clone, Debug, PartialEq, Eq, contracttype)]
pub struct ItemInfo {
    pub creator: Address,
    pub current_holder: Address,
    pub status: Status,
    pub metadata: Map<Symbol, RawVal>, // arbitrary key/value pairs supplied by creator
}

#[contract]
pub struct SupplyChain;

#[contractimpl]
impl SupplyChain {
    /// Initialise a new item. Returns the generated id.
    pub fn create_item(env: Env, metadata: Map<Symbol, RawVal>) -> ItemId {
        // Increment a global counter stored under a well‑known key.
        let id: ItemId = match env.storage().instance().get(&symbol_short!("counter")) {
            Some(val) => val + 1,
            None => 1,
        };
        env.storage().instance().set(&symbol_short!("counter"), &id);

        let creator = env.sender();
        let item = ItemInfo {
            creator: creator.clone(),
            current_holder: creator.clone(),
            status: Status::Created,
            metadata,
        };
        // Store under a composite key ("item", id).
        let key = (symbol_short!("item"), id).into_val(&env);
        env.storage().instance().set(&key, &item);
        id
    }

    /// Update the status of an existing item. Only the current holder may call.
    pub fn update_status(env: Env, id: ItemId, new_status: Status) {
        let key = (symbol_short!("item"), id).into_val(&env);
        let mut item: ItemInfo = env.storage().instance().get_unchecked(&key);
        // Access control: only current holder can update.
        if env.sender() != item.current_holder {
            panic!("unauthorized");
        }
        item.status = new_status;
        env.storage().instance().set(&key, &item);
    }

    /// Transfer custody to another participant.
    pub fn transfer_holder(env: Env, id: ItemId, new_holder: Address) {
        let key = (symbol_short!("item"), id).into_val(&env);
        let mut item: ItemInfo = env.storage().instance().get_unchecked(&key);
        if env.sender() != item.current_holder {
            panic!("unauthorized");
        }
        item.current_holder = new_holder;
        env.storage().instance().set(&key, &item);
    }

    /// Query public information about an item.
    pub fn get_item(env: Env, id: ItemId) -> ItemInfo {
        let key = (symbol_short!("item"), id).into_val(&env);
        env.storage().instance().get_unchecked(&key)
    }
}
