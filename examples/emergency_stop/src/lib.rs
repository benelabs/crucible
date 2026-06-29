#![no_std]
use soroban_sdk::{contract, contractimpl, contracttype, symbol_short, Address, Env};

#[contracttype]
enum DataKey {
    /// Admin address.
    Admin,
    /// Whether the contract is currently stopped.
    Stopped,
    /// Addresses authorised to trigger an emergency stop.
    Guardian(Address),
}

/// An emergency stop (circuit-breaker) contract.
///
/// The admin can:
/// - Add / remove guardian addresses.
/// - Resume operations after a stop.
///
/// Any guardian (or the admin) can:
/// - Trigger an emergency stop, halting protected operations.
///
/// Protected operations check `is_stopped()` and panic when the circuit is open.
#[contract]
#[derive(Default)]
pub struct EmergencyStop;

#[contractimpl]
impl EmergencyStop {
    /// Initialize the contract with an admin.
    pub fn initialize(env: Env, admin: Address) {
        if env.storage().instance().has(&DataKey::Admin) {
            panic!("already initialized");
        }
        admin.require_auth();
        env.storage().instance().set(&DataKey::Admin, &admin);
        env.storage().instance().set(&DataKey::Stopped, &false);
    }

    /// Add a guardian address. Admin only.
    pub fn add_guardian(env: Env, guardian: Address) {
        Self::require_admin(&env);
        env.storage()
            .instance()
            .set(&DataKey::Guardian(guardian.clone()), &true);
        env.events().publish(
            (symbol_short!("guardian"),),
            (symbol_short!("added"), guardian),
        );
    }

    /// Remove a guardian address. Admin only.
    pub fn remove_guardian(env: Env, guardian: Address) {
        Self::require_admin(&env);
        env.storage()
            .instance()
            .remove(&DataKey::Guardian(guardian.clone()));
        env.events().publish(
            (symbol_short!("guardian"),),
            (symbol_short!("removed"), guardian),
        );
    }

    /// Trigger an emergency stop. Callable by admin or any guardian.
    pub fn stop(env: Env, caller: Address) {
        caller.require_auth();
        let is_admin = Self::get_admin(&env) == caller;
        let is_guardian: bool = env
            .storage()
            .instance()
            .get(&DataKey::Guardian(caller.clone()))
            .unwrap_or(false);

        if !is_admin && !is_guardian {
            panic!("unauthorized: caller is not admin or guardian");
        }
        if Self::is_stopped_internal(&env) {
            panic!("already stopped");
        }
        env.storage().instance().set(&DataKey::Stopped, &true);
        env.events().publish((symbol_short!("stopped"),), caller);
    }

    /// Resume operations. Admin only.
    pub fn resume(env: Env) {
        Self::require_admin(&env);
        if !Self::is_stopped_internal(&env) {
            panic!("not stopped");
        }
        env.storage().instance().set(&DataKey::Stopped, &false);
        env.events().publish((symbol_short!("resumed"),), ());
    }

    /// Return whether the contract is currently stopped.
    pub fn is_stopped(env: Env) -> bool {
        Self::is_stopped_internal(&env)
    }

    /// Example of a protected operation — panics when the circuit is open.
    pub fn protected_action(env: Env, caller: Address) {
        caller.require_auth();
        if Self::is_stopped_internal(&env) {
            panic!("contract is stopped");
        }
        env.events().publish((symbol_short!("action"),), caller);
    }

    // -----------------------------------------------------------------------
    // Internal helpers
    // -----------------------------------------------------------------------

    fn get_admin(env: &Env) -> Address {
        env.storage()
            .instance()
            .get(&DataKey::Admin)
            .expect("not initialized")
    }

    fn require_admin(env: &Env) {
        let admin = Self::get_admin(env);
        admin.require_auth();
    }

    fn is_stopped_internal(env: &Env) -> bool {
        env.storage()
            .instance()
            .get(&DataKey::Stopped)
            .unwrap_or(false)
    }
}

#[cfg(test)]
mod test;
