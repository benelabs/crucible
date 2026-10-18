//! Tests for contract error demangling via [`MockEnv::register_errors`] and
//! the `err = ..` form of [`assert_reverts!`](crate::assert_reverts).
//!
//! [`MockEnv::register_errors`]: crate::env::MockEnv::register_errors

#[cfg(test)]
mod tests {
    use crate::env::{ErrorRegistry, MockEnv};
    use soroban_sdk::{contract, contracterror, contractimpl, Address, Env};

    #[contracterror]
    #[derive(Copy, Clone, Debug, Eq, PartialEq, PartialOrd, Ord)]
    #[repr(u32)]
    pub enum VaultError {
        Unauthorized = 1,
        InsufficientBalance = 2,
    }

    #[contracterror]
    #[derive(Copy, Clone, Debug, Eq, PartialEq, PartialOrd, Ord)]
    #[repr(u32)]
    pub enum RouterError {
        Paused = 1,
        UnknownRoute = 3,
    }

    #[contract]
    #[derive(Default)]
    struct Vault;

    #[contractimpl]
    impl Vault {
        /// Fails with [`VaultError::InsufficientBalance`] when withdrawing too much.
        pub fn withdraw(_env: Env, balance: i128, amount: i128) -> Result<i128, VaultError> {
            if amount > balance {
                return Err(VaultError::InsufficientBalance);
            }
            Ok(balance - amount)
        }
    }

    #[contract]
    #[derive(Default)]
    struct Router;

    #[contractimpl]
    impl Router {
        /// Forwards a withdrawal to the vault, bubbling up any vault error.
        pub fn forward(env: Env, vault: Address, balance: i128, amount: i128) -> i128 {
            VaultClient::new(&env, &vault).withdraw(&balance, &amount)
        }

        /// Always fails with [`RouterError::UnknownRoute`].
        pub fn route(_env: Env) -> Result<(), RouterError> {
            Err(RouterError::UnknownRoute)
        }
    }

    /// Builds a `MockEnv` with both contracts and their error enums registered.
    fn demangling_env() -> MockEnv {
        let env = MockEnv::builder()
            .with_contract::<Vault>()
            .with_contract::<Router>()
            .build();
        env.register_errors(&[VaultError::Unauthorized, VaultError::InsufficientBalance]);
        env.register_errors(&[RouterError::Paused, RouterError::UnknownRoute]);
        env
    }

    #[test]
    fn variant_name_uses_the_enum_and_variant() {
        assert_eq!(
            ErrorRegistry::variant_name(&VaultError::InsufficientBalance),
            "VaultError::InsufficientBalance"
        );
    }

    #[test]
    fn demangles_codes_to_registered_variants() {
        let env = demangling_env();
        assert_eq!(
            env.demangle_error("HostError: Error(Contract, #3)").as_deref(),
            Some("RouterError::UnknownRoute")
        );
        assert_eq!(
            env.demangle_error("HostError: Error(Contract, #2)").as_deref(),
            Some("VaultError::InsufficientBalance")
        );
        assert_eq!(env.demangle_error("HostError: Error(Contract, #9)"), None);
        assert_eq!(env.demangle_error("plain old panic"), None);
    }

    #[test]
    fn shared_codes_list_every_contract_variant() {
        let env = demangling_env();
        assert_eq!(
            env.demangle_error("HostError: Error(Contract, #1)").as_deref(),
            Some("VaultError::Unauthorized | RouterError::Paused")
        );
    }

    #[test]
    fn registering_a_variant_twice_is_a_no_op() {
        let env = demangling_env();
        env.register_errors(&[VaultError::InsufficientBalance]);
        assert_eq!(
            env.error_registry().lookup(2).as_deref(),
            Some("VaultError::InsufficientBalance")
        );
    }

    #[test]
    fn assert_reverts_matches_an_error_bubbled_from_a_nested_contract() {
        let env = demangling_env();
        let vault = env.contract_id::<Vault>();
        let router = RouterClient::new(env.inner(), &env.contract_id::<Router>());

        crate::assert_reverts!(
            env,
            router.forward(&vault, &100_i128, &500_i128),
            err = VaultError::InsufficientBalance
        );
        crate::assert_reverts!(
            router.forward(&vault, &100_i128, &500_i128),
            err = VaultError::InsufficientBalance
        );
    }

    #[test]
    fn assert_reverts_matches_the_calling_contracts_own_error() {
        let env = demangling_env();
        let router = RouterClient::new(env.inner(), &env.contract_id::<Router>());
        crate::assert_reverts!(env, router.route(), err = RouterError::UnknownRoute);
    }

    #[test]
    #[should_panic(expected = "Actual     : VaultError::InsufficientBalance (contract error #2)")]
    fn assert_reverts_failure_names_the_demangled_actual_variant() {
        let env = demangling_env();
        let vault = env.contract_id::<Vault>();
        let router = RouterClient::new(env.inner(), &env.contract_id::<Router>());
        crate::assert_reverts!(
            env,
            router.forward(&vault, &100_i128, &500_i128),
            err = RouterError::UnknownRoute
        );
    }

    #[test]
    #[should_panic(expected = "Expected   : VaultError::InsufficientBalance (contract error #2)")]
    fn assert_reverts_failure_reports_the_expected_variant_when_nothing_reverts() {
        let env = demangling_env();
        let vault = env.contract_id::<Vault>();
        let router = RouterClient::new(env.inner(), &env.contract_id::<Router>());
        crate::assert_reverts!(
            env,
            router.forward(&vault, &500_i128, &100_i128),
            err = VaultError::InsufficientBalance
        );
    }
}
