//! Integration tests for the [`MockEnv::expect_revert`] fluent assertion API.
//!
//! [`MockEnv::expect_revert`]: crate::env::MockEnv::expect_revert

#[cfg(test)]
mod tests {
    use crate::env::MockEnv;
    use soroban_sdk::{contract, contracterror, contractimpl, Env};

    #[contracterror]
    #[derive(Copy, Clone, Debug, Eq, PartialEq, PartialOrd, Ord)]
    #[repr(u32)]
    pub enum VaultError {
        Unauthorized = 1,
        InsufficientBalance = 2,
    }

    #[contract]
    #[derive(Default)]
    struct Vault;

    #[contractimpl]
    impl Vault {
        /// Always succeeds, returning the balance it was given.
        pub fn deposit(_env: Env, amount: i128) -> i128 {
            amount
        }

        /// Always fails with [`VaultError::Unauthorized`].
        pub fn admin_only(_env: Env) -> Result<(), VaultError> {
            Err(VaultError::Unauthorized)
        }

        /// Fails with [`VaultError::InsufficientBalance`] when withdrawing too much.
        pub fn withdraw(_env: Env, balance: i128, amount: i128) -> Result<i128, VaultError> {
            if amount > balance {
                return Err(VaultError::InsufficientBalance);
            }
            Ok(balance - amount)
        }
    }

    /// Builds a `MockEnv` with [`Vault`] registered.
    ///
    /// The client is created per-test rather than returned here, because it
    /// borrows the environment it was built from.
    fn vault_env() -> MockEnv {
        MockEnv::builder().with_contract::<Vault>().build()
    }

    /// Returns a client for the [`Vault`] registered in `env`.
    fn vault_client(env: &MockEnv) -> VaultClient<'_> {
        VaultClient::new(env.inner(), &env.contract_id::<Vault>())
    }

    #[test]
    fn with_any_error_accepts_a_contract_revert() {
        let env = vault_env();
        let client = vault_client(&env);
        env.expect_revert(|| client.admin_only())
            .with_any_error()
            .verify();
    }

    #[test]
    fn verify_without_an_expectation_accepts_any_revert() {
        let env = vault_env();
        let client = vault_client(&env);
        env.expect_revert(|| client.admin_only()).verify();
    }

    #[test]
    fn with_error_matches_the_specific_variant() {
        let env = vault_env();
        let client = vault_client(&env);
        env.expect_revert(|| client.admin_only())
            .with_error(VaultError::Unauthorized)
            .verify();

        env.expect_revert(|| client.withdraw(&100_i128, &500_i128))
            .with_error(VaultError::InsufficientBalance)
            .verify();
    }

    #[test]
    fn with_error_code_matches_a_raw_code() {
        let env = vault_env();
        let client = vault_client(&env);
        env.expect_revert(|| client.admin_only())
            .with_error_code(VaultError::Unauthorized as u32)
            .verify();
    }

    #[test]
    #[should_panic(expected = "reverted with a different error")]
    fn with_error_rejects_a_different_variant() {
        let env = vault_env();
        let client = vault_client(&env);
        env.expect_revert(|| client.admin_only())
            .with_error(VaultError::InsufficientBalance)
            .verify();
    }

    #[test]
    #[should_panic(expected = "the call did not revert")]
    fn verify_fails_when_the_call_succeeds() {
        let env = vault_env();
        let client = vault_client(&env);
        env.expect_revert(|| client.deposit(&1_i128))
            .with_any_error()
            .verify();
    }

    #[test]
    fn and_assert_runs_post_revert_assertions() {
        let env = vault_env();
        let client = vault_client(&env);

        // The failed withdrawal must leave the caller's view of the balance intact.
        let balance = 500_i128;
        let observed = env
            .expect_revert(|| client.withdraw(&balance, &999_i128))
            .with_error(VaultError::InsufficientBalance)
            .and_assert(|| client.withdraw(&balance, &100_i128));

        assert_eq!(observed, 400_i128);
    }

    #[test]
    #[should_panic(expected = "post-revert check ran")]
    fn and_assert_propagates_panics_from_its_closure() {
        let env = vault_env();
        let client = vault_client(&env);
        env.expect_revert(|| client.admin_only())
            .with_any_error()
            .and_assert(|| panic!("post-revert check ran"));
    }

    #[test]
    #[should_panic(expected = "the call did not revert")]
    fn and_assert_does_not_run_when_the_call_succeeded() {
        let env = vault_env();
        let client = vault_client(&env);
        env.expect_revert(|| client.deposit(&1_i128))
            .with_any_error()
            .and_assert(|| panic!("this closure must not run"));
    }

    #[test]
    fn the_assertion_is_a_value_that_can_be_stored_and_inspected() {
        let env = vault_env();
        let client = vault_client(&env);

        // The whole point of the fluent API: the assertion is an ordinary value.
        let assertion = env.expect_revert(|| client.admin_only());

        assert!(assertion.reverted());
        assert_eq!(assertion.error_code(), Some(VaultError::Unauthorized as u32));
        assert!(assertion.message().is_some());

        assertion.with_error(VaultError::Unauthorized).verify();
    }

    #[test]
    #[should_panic(expected = "dropped without being checked")]
    fn dropping_an_unchecked_assertion_fails_the_test() {
        let env = vault_env();
        let client = vault_client(&env);
        let _assertion = env.expect_revert(|| client.admin_only());
    }

    #[test]
    fn with_message_containing_matches_the_panic_text() {
        let env = MockEnv::default();
        env.expect_revert(|| panic!("time lock has not expired"))
            .with_message_containing("time lock")
            .verify();
    }

    #[test]
    #[should_panic(expected = "did not contain the expected text")]
    fn with_message_containing_rejects_a_different_message() {
        let env = MockEnv::default();
        env.expect_revert(|| panic!("time lock has not expired"))
            .with_message_containing("insufficient balance")
            .verify();
    }

    #[test]
    fn the_assert_reverts_macro_still_works_unchanged() {
        let env = vault_env();
        let client = vault_client(&env);
        crate::assert_reverts!(client.admin_only());
        crate::assert_reverts!(client.admin_only(), "admin-gated entry point");
    }
}
