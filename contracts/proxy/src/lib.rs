#![no_std]

use soroban_sdk::{
    contract, contracterror, contractimpl, contracttype, panic_with_error, symbol_short, Address,
    BytesN, Env, Symbol,
};

#[contracttype]
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum DataKey {
    Admin,
    Implementation,
}

#[contracterror]
#[derive(Copy, Clone, Debug, Eq, PartialEq, PartialOrd, Ord)]
#[repr(u32)]
pub enum ProxyError {
    AlreadyInitialized = 1,
    NotInitialized = 2,
    NotAdmin = 3,
    InvalidWasmHash = 4,
}

#[contract]
pub struct ProxyContract;

#[contractimpl]
impl ProxyContract {
    /// Initialize the Proxy contract with an admin and an initial implementation address or WASM hash.
    pub fn initialize(env: Env, admin: Address, implementation: BytesN<32>) {
        if env.storage().instance().has(&DataKey::Admin) {
            panic_with_error!(&env, ProxyError::AlreadyInitialized);
        }

        env.storage().instance().set(&DataKey::Admin, &admin);
        env.storage()
            .instance()
            .set(&DataKey::Implementation, &implementation);

        env.events().publish(
            (symbol_short!("init"),),
            (admin.clone(), implementation.clone()),
        );
    }

    /// Retrieve the current admin address.
    pub fn get_admin(env: Env) -> Address {
        env.storage()
            .instance()
            .get(&DataKey::Admin)
            .unwrap_or_else(|| panic_with_error!(&env, ProxyError::NotInitialized))
    }

    /// Retrieve the current logic implementation WASM hash.
    pub fn get_implementation(env: Env) -> BytesN<32> {
        env.storage()
            .instance()
            .get(&DataKey::Implementation)
            .unwrap_or_else(|| panic_with_error!(&env, ProxyError::NotInitialized))
    }

    /// Update the logic implementation WASM hash (Admin only).
    pub fn set_implementation(env: Env, new_implementation: BytesN<32>) {
        let admin = Self::get_admin(env.clone());
        admin.require_auth();

        env.storage()
            .instance()
            .set(&DataKey::Implementation, &new_implementation);

        env.events()
            .publish((symbol_short!("set_impl"),), new_implementation);
    }

    /// Perform a live upgrade of the contract WASM logic using Soroban's native contract deployer (Admin only).
    pub fn upgrade(env: Env, new_wasm_hash: BytesN<32>) {
        let admin = Self::get_admin(env.clone());
        admin.require_auth();

        // Update stored implementation hash reference
        env.storage()
            .instance()
            .set(&DataKey::Implementation, &new_wasm_hash);

        // Execute live WASM update for canonical address
        env.deployer()
            .update_current_contract_wasm(new_wasm_hash.clone());

        env.events()
            .publish((Symbol::new(&env, "upgrade"),), new_wasm_hash);
    }

    /// Transfer admin rights to a new address (Admin only).
    pub fn set_admin(env: Env, new_admin: Address) {
        let admin = Self::get_admin(env.clone());
        admin.require_auth();

        env.storage().instance().set(&DataKey::Admin, &new_admin);

        env.events()
            .publish((symbol_short!("set_admin"),), new_admin);
    }
}

#[cfg(test)]
mod test {
    use super::*;
    use soroban_sdk::{testutils::Address as _, Address, BytesN, Env};

    #[test]
    fn test_proxy_initialization_and_access() {
        let env = Env::default();
        let contract_id = env.register(ProxyContract, ());
        let client = ProxyContractClient::new(&env, &contract_id);

        let admin = Address::generate(&env);
        let impl_hash = BytesN::from_array(&env, &[1u8; 32]);

        client.initialize(&admin, &impl_hash);

        assert_eq!(client.get_admin(), admin);
        assert_eq!(client.get_implementation(), impl_hash);
    }

    #[test]
    #[should_panic]
    fn test_double_initialization_panics() {
        let env = Env::default();
        let contract_id = env.register(ProxyContract, ());
        let client = ProxyContractClient::new(&env, &contract_id);

        let admin = Address::generate(&env);
        let impl_hash = BytesN::from_array(&env, &[1u8; 32]);

        client.initialize(&admin, &impl_hash);
        client.initialize(&admin, &impl_hash);
    }

    #[test]
    fn test_set_implementation() {
        let env = Env::default();
        env.mock_all_auths();

        let contract_id = env.register(ProxyContract, ());
        let client = ProxyContractClient::new(&env, &contract_id);

        let admin = Address::generate(&env);
        let impl_hash_v1 = BytesN::from_array(&env, &[1u8; 32]);
        let impl_hash_v2 = BytesN::from_array(&env, &[2u8; 32]);

        client.initialize(&admin, &impl_hash_v1);
        client.set_implementation(&impl_hash_v2);

        assert_eq!(client.get_implementation(), impl_hash_v2);
    }
}
