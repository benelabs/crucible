//! Account management for Soroban testing.
//!
//! Provides `AccountHandle` - a wrapper around a Soroban `Address` with
//! keypair signing support and balance helpers, and `AccountBuilder` for
//! easy pre-funded account creation.
//!
//! **Host-only:** All types in this module depend on [`MockEnv`] and `std`
//! and are intended exclusively for use in `#[cfg(test)]` contexts on the host.
//!
//! [`MockEnv`]: crate::env::MockEnv

use crate::env::{MockEnv, Stroops};
use crate::token::MockToken;
use soroban_sdk::Address;

/// A handle to a Soroban account used in tests.
#[derive(Clone)]
pub struct AccountHandle {
    mock_env: MockEnv,
    name: String,
    address: Address,
}

impl AccountHandle {
    /// Internal constructor for use by `AccountBuilder` or `MockEnv`.
    pub(crate) fn new(mock_env: MockEnv, name: String, address: Address) -> Self {
        Self {
            mock_env,
            name,
            address,
        }
    }

    /// Returns the account's name.
    pub fn name(&self) -> &str {
        &self.name
    }

    /// Returns the account's address.
    pub fn address(&self) -> Address {
        self.address.clone()
    }

    /// Returns the account's XLM balance (in stroops).
    pub fn xlm_balance(&self) -> i128 {
        let xlm_address = self.mock_env.xlm_token_address()
            .expect("XLM token address not set in environment. Use MockToken::xlm(&env) or MockEnvBuilder::with_account to set it.");
        let xlm_token = MockToken::from_address(self.mock_env.inner(), xlm_address);
        xlm_token.balance(&self.address)
    }

    /// Returns the account's balance in a given token.
    pub fn token_balance(&self, token: &MockToken) -> i128 {
        token.balance(&self.address)
    }
}

impl core::ops::Deref for AccountHandle {
    type Target = Address;

    fn deref(&self) -> &Self::Target {
        &self.address
    }
}

impl AsRef<Address> for AccountHandle {
    fn as_ref(&self) -> &Address {
        &self.address
    }
}

impl std::fmt::Debug for AccountHandle {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        // Intentionally omit `mock_env` and any future signing material from debug output.
        f.debug_struct("AccountHandle")
            .field("name", &self.name)
            .field("address", &self.address)
            .finish()
    }
}

/// A builder for creating pre-funded accounts.
pub struct AccountBuilder<'env> {
    env: &'env MockEnv,
    name: String,
    xlm_balance: Stroops,
    token_balances: Vec<(&'env MockToken, i128)>,
}

impl<'env> AccountBuilder<'env> {
    /// Creates a new `AccountBuilder` for the given environment.
    pub fn new(env: &'env MockEnv) -> Self {
        Self {
            env,
            name: "unnamed".to_string(),
            xlm_balance: Stroops::from(0),
            token_balances: Vec::new(),
        }
    }

    /// Sets the name of the account for later retrieval via `MockEnv::account(name)`.
    pub fn name(mut self, name: &str) -> Self {
        self.name = name.to_string();
        self
    }

    /// Funds the account with the given amount of XLM.
    pub fn fund_xlm(mut self, amount: Stroops) -> Self {
        self.xlm_balance = amount;
        self
    }

    /// Funds the account with the given amount of a specific token.
    pub fn fund_token(mut self, token: &'env MockToken, amount: i128) -> Self {
        self.token_balances.push((token, amount));
        self
    }

    /// Builds the account, registering it in the environment and funding it.
    pub fn build(self) -> AccountHandle {
        // 1. Create a mock auth contract for the account (represented as an address)
        let address = self
            .env
            .inner()
            .register(soroban_sdk::testutils::MockAuthContract {}, ());

        // 2. Fund XLM if requested
        if self.xlm_balance.as_stroops() > 0 {
            let xlm = MockToken::xlm(self.env);
            xlm.mint(&address, self.xlm_balance.as_stroops());
        }

        // 3. Fund other tokens
        for (token, amount) in self.token_balances {
            token.mint(&address, amount);
        }

        // 4. Register in MockEnv
        self.env.register_account(&self.name, address.clone());

        AccountHandle::new(self.env.clone(), self.name, address)
    }
}

#[cfg(test)]
mod tests {
    use crate::prelude::*;

    #[test]
    fn test_account_creation_and_funding() {
        let env = MockEnv::builder()
            .with_account("alice", Stroops::xlm(100))
            .with_account("bob", Stroops::xlm(50))
            .build();

        let alice = env.account("alice");
        let bob = env.account("bob");

        assert_eq!(alice.xlm_balance(), Stroops::xlm(100).as_stroops());
        assert_eq!(bob.xlm_balance(), Stroops::xlm(50).as_stroops());
    }

    #[test]
    fn test_token_transfer_between_accounts() {
        let env = MockEnv::builder()
            .with_account("alice", Stroops::xlm(100))
            .with_account("bob", Stroops::xlm(100))
            .build();
        env.mock_all_auths();

        let alice = env.account("alice");
        let bob = env.account("bob");
        let xlm = MockToken::xlm(&env);

        // Perform transfer using the addresses
        xlm.transfer(&*alice, &*bob, 20_000_000);

        assert_eq!(alice.xlm_balance(), 980_000_000);
        assert_eq!(bob.xlm_balance(), 1_020_000_000);
    }

    #[test]
    fn test_account_handle_clone() {
        let env = MockEnv::builder()
            .with_account("alice", Stroops::xlm(100))
            .build();

        let alice = env.account("alice");
        let cloned = alice.clone();

        assert_eq!(cloned.name(), alice.name());
        assert_eq!(cloned.address(), alice.address());
        assert_eq!(cloned.xlm_balance(), alice.xlm_balance());
    }

    #[test]
    fn test_account_handle_debug_omits_sensitive_fields() {
        let env = MockEnv::builder()
            .with_account("alice", Stroops::xlm(100))
            .build();

        let alice = env.account("alice");
        let debug = format!("{alice:?}");

        assert!(debug.contains("alice"));
        assert!(debug.contains("AccountHandle"));
        assert!(!debug.contains("mock_env"));
    }

    #[test]
    fn test_account_builder_fluent() {
        let env = MockEnv::builder().build();
        let usdc = MockToken::new(&env, "USDC", 6);

        let charlie = AccountBuilder::new(&env)
            .name("charlie")
            .fund_xlm(Stroops::xlm(10))
            .fund_token(&usdc, 1000)
            .build();

        assert_eq!(charlie.xlm_balance(), Stroops::xlm(10).as_stroops());
        assert_eq!(charlie.token_balance(&usdc), 1000);

        // Should be retrievable from env
        let charlie_ref = env.account("charlie");
        assert_eq!(charlie_ref.address(), charlie.address());
    }
}

#[cfg(test)]
mod fixture_tests {
    use crate::fixture;
    use crate::prelude::*;
    use soroban_sdk::Address;

    #[fixture]
    struct AccountsFixture {
        pub env: MockEnv,
        pub alice: AccountHandle,
        pub bob: AccountHandle,
    }

    impl AccountsFixture {
        pub fn setup() -> Self {
            let env = MockEnv::builder()
                .with_account("alice", Stroops::xlm(100))
                .with_account("bob", Stroops::xlm(50))
                .build();

            Self {
                alice: env.account("alice"),
                bob: env.account("bob"),
                env,
            }
        }
    }

    fn account_address(handle: AccountHandle) -> Address {
        handle.address()
    }

    #[test]
    fn fixture_with_account_handles_supports_debug_clone_and_reset() {
        let mut fixture = AccountsFixture::setup();

        let alice_addr = account_address(fixture.alice.clone());
        assert_eq!(alice_addr, fixture.alice.address());
        assert_eq!(fixture.env.account("alice").address(), fixture.alice.address());

        let debug = format!("{fixture:?}");
        assert!(debug.contains("alice"));
        assert!(debug.contains("bob"));

        fixture.reset();
        assert_eq!(fixture.alice.xlm_balance(), Stroops::xlm(100).as_stroops());
        assert_eq!(fixture.bob.xlm_balance(), Stroops::xlm(50).as_stroops());
    }
}
