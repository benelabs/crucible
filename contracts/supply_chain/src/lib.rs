#![no_std]
use soroban_sdk::{contract, contractimpl, contracttype, symbol_short, Env, Symbol, Address, Map, RawVal, panic_with_error, panic};

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

impl Status {
    /// Validates whether a transition from `self` to `next` is allowed.
    /// Strictly allowable transitions:
    /// Created -> InTransit -> Received -> Inspected -> Completed / Rejected
    pub fn is_valid_transition(&self, next: &Status) -> bool {
        matches!(
            (self, next),
            (Status::Created, Status::InTransit)
                | (Status::InTransit, Status::Received)
                | (Status::Received, Status::Inspected)
                | (Status::Inspected, Status::Completed)
                | (Status::Inspected, Status::Rejected)
        )
    }
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

impl SupplyChain {
    /// Validates whether a status transition is allowable.
    pub fn is_valid_transition(current: &Status, next: &Status) -> bool {
        current.is_valid_transition(next)
    }

    /// Enforces strictly allowable transitions, panicking if invalid.
    pub fn validate_transition(current: &Status, next: &Status) {
        if !current.is_valid_transition(next) {
            panic!("invalid status transition");
        }
    }
}

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
        // State transition validation.
        Self::validate_transition(&item.status, &new_status);
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_is_valid_transition() {
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
        assert!(!Status::Rejected.is_valid_transition(&Status::InTransit));
        assert!(!Status::Completed.is_valid_transition(&Status::InTransit));
        assert!(!Status::Created.is_valid_transition(&Status::Created));
    }
}

