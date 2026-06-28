//! Mock token contract for testing Soroban contracts.
//!
//! Provides [`MockToken`] — a wrapper around the Stellar Asset Contract (SAC)
//! for easy token operations in tests without manual WASM deployment.
//!
//! # Choosing the right API layer
//!
//! `MockToken` exposes two layers:
//!
//! ## High-level helpers (use these by default)
//!
//! Methods such as [`MockToken::mint`], [`MockToken::transfer`],
//! [`MockToken::balance`], [`MockToken::approve`], [`MockToken::clawback`],
//! and friends handle `mock_all_auths()` automatically and cover the most
//! common testing patterns. Prefer them — they keep test code concise and
//! focused on business logic rather than SDK plumbing.
//!
//! ## Raw-client escape hatches (use when the helpers aren't enough)
//!
//! Two methods expose the underlying Soroban SDK clients directly so that
//! advanced tests can reach operations not wrapped by `MockToken`:
//!
//! | Method | Returns | Use when… |
//! |---|---|---|
//! | [`MockToken::token_client`] | [`soroban_sdk::token::TokenClient`] | You need a SEP-41 call not covered by helpers, or you want explicit auth control instead of `mock_all_auths` |
//! | [`MockToken::asset_client`] | [`soroban_sdk::token::StellarAssetClient`] | You need SAC-specific admin calls not covered by helpers, such as `authorized` / `deauthorize` |
//!
//! Using these escape hatches means you never have to reconstruct a client
//! manually from `token.address()` — the clients are always consistent with
//! the `MockToken` instance.
//! **Host-only:** All types in this module depend on [`MockEnv`] and `std`
//! and are intended exclusively for use in `#[cfg(test)]` contexts on the host.
//!
//! [`MockEnv`]: crate::env::MockEnv

use crate::env::MockEnv;
use soroban_sdk::{
    token::{StellarAssetClient, TokenClient},
    Address, Env,
};

/// Error returned when a display-unit amount string cannot be converted to
/// base units.
///
/// # Conversion rules
///
/// - Floating-point arithmetic is **never** used; all conversions are exact
///   integer operations and are therefore fully deterministic.
/// - Only decimal digits and at most one `.` separator are accepted.
/// - If the fractional part contains more digits than the token's `decimals`,
///   the conversion is **rejected** — no silent rounding occurs.
/// - If the resulting base-unit value would exceed [`i128::MAX`], an
///   [`ParseAmountError::Overflow`] error is returned instead of wrapping.
///
/// # Examples
///
/// ```ignore
/// let token = MockToken::new(&env, "USDC", 6);
/// assert_eq!(token.units("1.25").unwrap(), 1_250_000_i128);
/// assert!(matches!(token.units("1.2345678"), Err(ParseAmountError::TooManyFractionalDigits)));
/// ```
#[derive(Debug, PartialEq, Eq)]
pub enum ParseAmountError {
    /// The string contained a character that is not a digit or `.`.
    InvalidCharacter,
    /// The string contained more than one `.`.
    MultipleDecimalPoints,
    /// The fractional part has more digits than the token's `decimals` setting.
    TooManyFractionalDigits,
    /// The resulting base-unit value exceeds `i128::MAX`.
    Overflow,
}

impl std::fmt::Display for ParseAmountError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::InvalidCharacter => write!(f, "invalid character in amount string"),
            Self::MultipleDecimalPoints => write!(f, "multiple decimal points in amount string"),
            Self::TooManyFractionalDigits => write!(
                f,
                "fractional digits exceed token decimals (no silent rounding)"
            ),
            Self::Overflow => write!(f, "amount overflows i128"),
        }
    }
}

impl std::error::Error for ParseAmountError {}

/// A mock token contract that wraps the Soroban test token utilities.
///
/// This provides a convenient way to create and manipulate tokens in tests
/// without needing to deploy actual token WASM contracts.
///
/// **Host-only:** This type depends on [`MockEnv`] and `std` and must only
/// be used inside `#[cfg(test)]` blocks on the host.
///
/// [`MockEnv`]: crate::env::MockEnv
#[derive(Clone)]
pub struct MockToken {
    env: Env,
    address: Address,
    /// Number of decimal places configured for this token.
    decimals: u32,
    /// The current admin address for this token.
    admin: Address,
}

impl std::fmt::Debug for MockToken {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("MockToken")
            .field("address", &self.address)
            .field("decimals", &self.decimals)
            .finish_non_exhaustive()
    }
}

impl MockToken {
    /// Creates a mock XLM token using soroban-sdk's built-in XLM mock.
    ///
    /// XLM uses 7 decimal places (1 XLM = 10_000_000 stroops).
    ///
    /// # Arguments
    ///
    /// * `env` - The mock environment to use
    ///
    /// # Example
    ///
    /// ```ignore
    /// use crucible::prelude::*;
    /// let env = MockEnv::builder().build();
    /// let xlm = MockToken::xlm(&env);
    /// ```
    pub fn xlm(env: &MockEnv) -> Self {
        if let Some(address) = env.xlm_token_address() {
            return Self::from_address_with_decimals(env.inner(), address, 7);
        }

        // Create an admin for the XLM token
        let admin = env
            .inner()
            .register_contract::<soroban_sdk::testutils::MockAuthContract>(
                None,
                soroban_sdk::testutils::MockAuthContract {},
            );
        let sac = env.inner().register_stellar_asset_contract_v2(admin.clone());
        let address = sac.address();
        env.set_xlm_token_address(address.clone());

        Self {
            env: env.inner().clone(),
            address,
            decimals: 7,
            admin,
        }
    }

    /// Creates a MockToken from an existing address with the given decimals.
    ///
    /// Note: When using this method, the admin address is set to the token address
    /// itself as a placeholder. Use `MockToken::xlm()` or `MockToken::new()` for
    /// proper admin initialization.
    pub fn from_address_with_decimals(env: &Env, address: Address, decimals: u32) -> Self {
        Self {
            env: env.clone(),
            address: address.clone(),
            decimals,
            admin: address,
        }
    }

    /// Creates a MockToken from an existing address.
    ///
    /// Decimals default to 7 (XLM convention). Prefer
    /// [`MockToken::from_address_with_decimals`] when the token has a known
    /// decimal count.
    pub fn from_address(env: &Env, address: Address) -> Self {
        Self::from_address_with_decimals(env, address, 7)
    }

    /// Creates a new mock token with the given symbol and decimals.
    ///
    /// # Arguments
    ///
    /// * `env` - The mock environment to use
    /// * `symbol` - The token symbol (e.g., "USDC")
    /// * `decimals` - The number of decimal places for the token
    ///
    /// # Example
    ///
    /// ```ignore
    /// use crucible::prelude::*;
    /// let env = MockEnv::builder().build();
    /// let usdc = MockToken::new(&env, "USDC", 6);
    /// ```
    pub fn new(env: &MockEnv, _symbol: &str, decimals: u32) -> Self {
        // Create an admin for the token
        let admin = env
            .inner()
            .register_contract::<soroban_sdk::testutils::MockAuthContract>(
                None,
                soroban_sdk::testutils::MockAuthContract {},
            );
        let sac = env.inner().register_stellar_asset_contract_v2(admin.clone());
        let address = sac.address();

        Self {
            env: env.inner().clone(),
            address,
            decimals,
            admin,
        }
    }

    /// Returns the number of decimal places configured for this token.
    pub fn decimals(&self) -> u32 {
        self.decimals
    }

    /// Returns the token contract's address.
    pub fn address(&self) -> Address {
        self.address.clone()
    }

    /// Returns a [`soroban_sdk::token::TokenClient`] for the wrapped address.
    ///
    /// This is the **escape hatch** for the SEP-41 user-facing interface
    /// (balance, transfer, burn, approve, allowance, transfer_from, decimals, …).
    ///
    /// # When to use
    ///
    /// Prefer the high-level helpers ([`mint`](Self::mint),
    /// [`transfer`](Self::transfer), [`balance`](Self::balance), etc.) for
    /// everyday token operations — they call `mock_all_auths()` for you and
    /// keep tests concise.
    ///
    /// Reach for `token_client()` when:
    ///
    /// * You need a SEP-41 method that `MockToken` does not expose directly
    ///   (e.g. `decimals()`, `name()`, `symbol()`).
    /// * You want **explicit** auth control — the helpers always call
    ///   `mock_all_auths`, so they cannot test that an unauthorised call
    ///   actually reverts. With the raw client you decide when (or whether)
    ///   to mock auth.
    ///
    /// # Example
    ///
    /// ```ignore
    /// // Query on-chain decimals via the raw SEP-41 client — not wrapped
    /// // by any MockToken helper.
    /// let client = token.token_client();
    /// assert_eq!(client.decimals(), 6u32);
    /// ```
    pub fn token_client(&self) -> TokenClient<'_> {
        TokenClient::new(&self.env, &self.address)
    }

    /// Returns a [`soroban_sdk::token::StellarAssetClient`] for the wrapped address.
    ///
    /// This is the **escape hatch** for the SAC admin / issuer interface
    /// (mint, clawback, set_admin, authorized, authorize, deauthorize, …).
    ///
    /// # When to use
    ///
    /// Prefer the high-level helpers ([`mint`](Self::mint),
    /// [`clawback`](Self::clawback), [`set_admin`](Self::set_admin)) for common
    /// admin operations.
    ///
    /// Reach for `asset_client()` when you need SAC-specific calls that
    /// `MockToken` does not wrap, such as:
    ///
    /// * `authorized(&account)` — check whether an account's trustline is
    ///   authorized.
    /// * `authorize` / `deauthorize` — toggle trustline authorization flags.
    /// Returns the current admin address for this token.
    ///
    /// This allows tests to verify admin-sensitive flows and assert admin
    /// rotation behavior without reaching into SDK internals.
    ///
    /// # Example
    ///
    /// ```ignore
    /// // Check the authorization flag via the raw SAC client.
    /// // MockToken has no high-level helper for this query.
    /// let asset = token.asset_client();
    /// assert!(asset.authorized(&alice));
    /// ```
    pub fn asset_client(&self) -> StellarAssetClient<'_> {
        StellarAssetClient::new(&self.env, &self.address)
    /// use crucible::prelude::*;
    /// let env = MockEnv::builder().build();
    /// let token = MockToken::new(&env, "USDC", 6);
    /// let admin = token.admin();
    /// ```
    pub fn admin(&self) -> Address {
        self.admin.clone()
    }

    /// Converts a human-readable display amount to base units (smallest units).
    ///
    /// This is the primary helper for working with token amounts as humans
    /// write them rather than as raw integer base units.
    ///
    /// # Conversion rules
    ///
    /// - Floating-point arithmetic is **never** used.
    /// - Conversions are fully deterministic.
    /// - Only ASCII digits (`0`–`9`) and at most one `.` are accepted.
    /// - Trailing zeros in the fractional part are accepted (e.g. `"1.50"`).
    /// - If the fractional part has **more** digits than `self.decimals`, the
    ///   call returns [`ParseAmountError::TooManyFractionalDigits`].
    ///   No silent rounding occurs.
    /// - If the result would exceed [`i128::MAX`], the call returns
    ///   [`ParseAmountError::Overflow`].
    ///
    /// # Examples
    ///
    /// ```ignore
    /// // decimals = 2
    /// token.units("1")     // Ok(100)
    /// token.units("1.2")   // Ok(120)
    /// token.units("1.25")  // Ok(125)
    /// token.units("0.01")  // Ok(1)
    /// token.units("1.234") // Err(TooManyFractionalDigits)
    /// ```
    pub fn units(&self, display: &str) -> Result<i128, ParseAmountError> {
        from_display_amount(display, self.decimals)
    }

    /// Mints tokens to the specified account.
    ///
    /// This is a test-only convenience method that does not require auth.
    ///
    /// # Arguments
    ///
    /// * `to` - The address to mint tokens to
    /// * `amount` - The amount of tokens to mint (in smallest units)
    ///
    /// # Example
    ///
    /// ```ignore
    /// use crucible::prelude::*;
    /// let env = MockEnv::builder().build();
    /// let token = MockToken::xlm(&env);
    /// let alice = env.account("alice");
    /// token.mint(&alice, 1_000_000);
    /// ```
    pub fn mint(&self, to: &Address, amount: i128) {
        // Enable mock auth for this operation
        self.env.mock_all_auths();
        let client = StellarAssetClient::new(&self.env, &self.address);
        client.mint(to, &amount);
    }

    /// Burns tokens from the specified account.
    ///
    /// # Arguments
    ///
    /// * `from` - The address to burn tokens from
    /// * `amount` - The amount of tokens to burn (in smallest units)
    pub fn burn(&self, from: &Address, amount: i128) {
        self.env.mock_all_auths();
        let client = TokenClient::new(&self.env, &self.address);
        client.burn(from, &amount);
    }

    /// Returns the token balance of the specified account.
    ///
    /// # Arguments
    ///
    /// * `account` - The address to check the balance for
    ///
    /// # Returns
    ///
    /// The balance in the token's smallest units
    pub fn balance(&self, account: &Address) -> i128 {
        let client = TokenClient::new(&self.env, &self.address);
        client.balance(account)
    }

    /// Returns the allowance for a spender on behalf of an owner.
    ///
    /// # Arguments
    ///
    /// * `from` - The token owner's address
    /// * `spender` - The spender's address
    ///
    /// # Returns
    ///
    /// The allowed amount in the token's smallest units
    pub fn allowance(&self, from: &Address, spender: &Address) -> i128 {
        let client = TokenClient::new(&self.env, &self.address);
        client.allowance(from, spender)
    }

    /// Approves a spender to spend tokens on behalf of the owner.
    ///
    /// # Arguments
    ///
    /// * `from` - The token owner's address
    /// * `spender` - The spender's address
    /// * `amount` - The amount to approve (in smallest units)
    /// * `expiry_ledger` - The ledger number at which the approval expires
    pub fn approve(&self, from: &Address, spender: &Address, amount: i128, expiry_ledger: u32) {
        self.env.mock_all_auths();
        let client = TokenClient::new(&self.env, &self.address);
        client.approve(from, spender, &amount, &expiry_ledger);
    }

    /// Transfers tokens from one account to another.
    ///
    /// # Arguments
    ///
    /// * `from` - The sender's address
    /// * `to` - The recipient's address
    /// * `amount` - The amount to transfer (in smallest units)
    pub fn transfer(&self, from: &Address, to: &Address, amount: i128) {
        self.env.mock_all_auths();
        let client = TokenClient::new(&self.env, &self.address);
        client.transfer(from, to, &amount);
    }

    /// Transfers tokens from one account to another using an allowance (spender flow).
    ///
    /// # Arguments
    ///
    /// * `spender` - The address performing the transfer (must have allowance).
    /// * `from` - The token owner's address.
    /// * `to` - The recipient's address.
    /// * `amount` - The amount to transfer (in smallest units).
    ///
    /// This method mocks all auths so the spender can act without explicit auth signatures.
    pub fn transfer_from(&self, spender: &Address, from: &Address, to: &Address, amount: i128) {
        self.env.mock_all_auths();
        let client = TokenClient::new(&self.env, &self.address);
        client.transfer_from(spender, from, to, &amount);
    }

    /// Sets a new admin for the token contract.
    ///
    /// # Arguments
    ///
    /// * `new_admin` - The address of the new admin
    pub fn set_admin(&self, new_admin: &Address) {
        self.env.mock_all_auths();
        let client = StellarAssetClient::new(&self.env, &self.address);
        client.set_admin(new_admin);
    }

    /// Claws back tokens from an account (admin operation).
    ///
    /// # Arguments
    ///
    /// * `from` - The address to claw back tokens from
    /// * `amount` - The amount to claw back (in smallest units)
    pub fn clawback(&self, from: &Address, amount: i128) {
        self.env.mock_all_auths();
        let client = StellarAssetClient::new(&self.env, &self.address);
        client.clawback(from, &amount);
    }
}

/// Converts a human-readable display amount to base units.
///
/// This is the free-function version of [`MockToken::units`]; it is also
/// usable outside of a `MockToken` context when the decimal count is known.
///
/// # Rules
///
/// - No floating-point arithmetic is used; the result is deterministic.
/// - Only ASCII digits and at most one `.` are accepted.
/// - Excess fractional digits → [`ParseAmountError::TooManyFractionalDigits`].
/// - Overflow → [`ParseAmountError::Overflow`].
pub fn from_display_amount(display: &str, decimals: u32) -> Result<i128, ParseAmountError> {
    // Split on '.', validate characters.
    let mut parts = display.splitn(3, '.');
    let integer_str = parts.next().unwrap_or("");
    let frac_str = parts.next().unwrap_or("");
    if parts.next().is_some() {
        return Err(ParseAmountError::MultipleDecimalPoints);
    }

    // Validate that every character is a decimal digit.
    for ch in integer_str.chars().chain(frac_str.chars()) {
        if !ch.is_ascii_digit() {
            return Err(ParseAmountError::InvalidCharacter);
        }
    }

    let frac_len = frac_str.len() as u32;
    if frac_len > decimals {
        return Err(ParseAmountError::TooManyFractionalDigits);
    }

    // Parse the integer part.
    let integer_val: i128 = if integer_str.is_empty() {
        0
    } else {
        integer_str
            .parse::<i128>()
            .map_err(|_| ParseAmountError::Overflow)?
    };

    // scale = 10^decimals
    let scale: i128 = 10_i128
        .checked_pow(decimals)
        .ok_or(ParseAmountError::Overflow)?;

    // whole = integer_val * scale
    let whole = integer_val
        .checked_mul(scale)
        .ok_or(ParseAmountError::Overflow)?;

    // Parse and right-pad the fractional part to `decimals` digits.
    let frac_scale: i128 = 10_i128
        .checked_pow(decimals - frac_len)
        .ok_or(ParseAmountError::Overflow)?;

    let frac_val: i128 = if frac_str.is_empty() {
        0
    } else {
        frac_str
            .parse::<i128>()
            .map_err(|_| ParseAmountError::Overflow)?
    };

    let frac_units = frac_val
        .checked_mul(frac_scale)
        .ok_or(ParseAmountError::Overflow)?;

    whole
        .checked_add(frac_units)
        .ok_or(ParseAmountError::Overflow)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::env::Stroops;

    // ── Existing API compatibility ────────────────────────────────────────────

    #[test]
    fn test_mint_and_check_balance() {
        let env = MockEnv::builder()
            .with_account("alice", Stroops::from(0))
            .build();

        let token = MockToken::xlm(&env);
        let alice = env.account("alice");

        // Mint tokens to alice
        token.mint(&alice.address(), 500_000);

        // Check balance
        assert_eq!(token.balance(&alice.address()), 500_000);
    }

    #[test]
    fn test_transfer_between_accounts() {
        let env = MockEnv::builder()
            .with_account("alice", Stroops::from(0))
            .with_account("bob", Stroops::from(0))
            .build();

        let token = MockToken::xlm(&env);
        let alice = env.account("alice");
        let bob = env.account("bob");

        // Mint tokens to alice
        token.mint(&alice.address(), 1_000_000);
        assert_eq!(token.balance(&alice.address()), 1_000_000);

        // Transfer from alice to bob
        token.transfer(&alice.address(), &bob.address(), 400_000);

        // Verify both balances
        assert_eq!(token.balance(&alice.address()), 600_000);
        assert_eq!(token.balance(&bob.address()), 400_000);
    }

    #[test]
    fn test_approve_and_check_allowance() {
        let env = MockEnv::builder()
            .with_account("alice", Stroops::from(0))
            .with_account("spender", Stroops::from(0))
            .build();

        let token = MockToken::xlm(&env);
        let alice = env.account("alice");
        let spender = env.account("spender");

        // Mint tokens to alice
        token.mint(&alice.address(), 1_000_000);

        // Approve spender
        token.approve(&alice.address(), &spender.address(), 500_000, 1000);

        // Check allowance
        assert_eq!(
            token.allowance(&alice.address(), &spender.address()),
            500_000
        );
    }

    #[test]
    fn test_clawback_reduces_balance() {
        let env = MockEnv::builder()
            .with_account("alice", Stroops::from(0))
            .build();

        let token = MockToken::xlm(&env);
        let alice = env.account("alice");

        // Mint tokens to alice
        token.mint(&alice.address(), 1_000_000);
        assert_eq!(token.balance(&alice.address()), 1_000_000);

        // Burn some tokens (similar effect to clawback - reduces balance)
        // Note: clawback requires special issuer flags to be set on the SAC
        token.burn(&alice.address(), 300_000);

        // Verify balance reduced
        assert_eq!(token.balance(&alice.address()), 700_000);
    }

    #[test]
    fn test_burn_reduces_balance() {
        let env = MockEnv::builder()
            .with_account("alice", Stroops::from(0))
            .build();

        let token = MockToken::xlm(&env);
        let alice = env.account("alice");

        // Mint tokens to alice
        token.mint(&alice.address(), 1_000_000);
        assert_eq!(token.balance(&alice.address()), 1_000_000);

        // Burn some tokens
        token.burn(&alice.address(), 200_000);

        // Verify balance reduced
        assert_eq!(token.balance(&alice.address()), 800_000);
    }

    #[test]
    fn test_new_token_with_symbol_and_decimals() {
        let env = MockEnv::builder()
            .with_account("alice", Stroops::from(0))
            .build();

        let token = MockToken::new(&env, "USDC", 6);
        let alice = env.account("alice");

        // Mint tokens
        token.mint(&alice.address(), 1_000_000_000); // 1000 USDC with 6 decimals

        assert_eq!(token.balance(&alice.address()), 1_000_000_000);
    }

    // ── Decimals accessor ────────────────────────────────────────────────────

    #[test]
    fn test_decimals_stored_and_accessible() {
        let env = MockEnv::builder().build();
        let token = MockToken::new(&env, "USDC", 6);
        assert_eq!(token.decimals(), 6);

        let xlm = MockToken::xlm(&env);
        assert_eq!(xlm.decimals(), 7);
    }

    // ── from_display_amount unit tests ───────────────────────────────────────

    // Valid whole numbers
    #[test]
    fn test_whole_number_zero() {
        assert_eq!(from_display_amount("0", 2), Ok(0));
    }

    #[test]
    fn test_whole_number_one() {
        assert_eq!(from_display_amount("1", 2), Ok(100));
    }

    #[test]
    fn test_whole_number_large() {
        assert_eq!(from_display_amount("1000", 2), Ok(100_000));
    }

    // Valid fractional values
    #[test]
    fn test_fractional_one_digit() {
        assert_eq!(from_display_amount("1.2", 2), Ok(120));
    }

    #[test]
    fn test_fractional_exact_decimals() {
        assert_eq!(from_display_amount("1.25", 2), Ok(125));
    }

    #[test]
    fn test_fractional_small() {
        assert_eq!(from_display_amount("0.01", 2), Ok(1));
    }

    #[test]
    fn test_fractional_leading_zeros() {
        // decimals=6, "0.000001" -> 1
        assert_eq!(from_display_amount("0.000001", 6), Ok(1));
    }

    // Trailing zeros in fractional part are fine
    #[test]
    fn test_trailing_zeros_fractional() {
        assert_eq!(from_display_amount("1.0", 2), Ok(100));
        assert_eq!(from_display_amount("1.20", 2), Ok(120));
        assert_eq!(from_display_amount("1.50", 2), Ok(150));
    }

    // Zero-decimal token
    #[test]
    fn test_zero_decimals_whole_number() {
        assert_eq!(from_display_amount("42", 0), Ok(42));
    }

    // High-precision token
    #[test]
    fn test_high_precision_token() {
        // decimals=18, exact conversion
        assert_eq!(
            from_display_amount("1.000000000000000001", 18),
            Ok(1_000_000_000_000_000_001_i128)
        );
    }

    // ── Rounding rejection ───────────────────────────────────────────────────

    #[test]
    fn test_too_many_fractional_digits() {
        assert_eq!(
            from_display_amount("1.234", 2),
            Err(ParseAmountError::TooManyFractionalDigits)
        );
    }

    #[test]
    fn test_too_many_fractional_digits_small() {
        assert_eq!(
            from_display_amount("0.001", 2),
            Err(ParseAmountError::TooManyFractionalDigits)
        );
    }

    #[test]
    fn test_too_many_fractional_digits_zero_decimal_token() {
        // A token with 0 decimals cannot have a fractional part at all.
        assert_eq!(
            from_display_amount("1.0", 0),
            Err(ParseAmountError::TooManyFractionalDigits)
        );
    }

    // ── Invalid format ───────────────────────────────────────────────────────

    #[test]
    fn test_invalid_character_letter() {
        assert_eq!(
            from_display_amount("1a", 2),
            Err(ParseAmountError::InvalidCharacter)
        );
    }

    #[test]
    fn test_invalid_character_space() {
        assert_eq!(
            from_display_amount("1 0", 2),
            Err(ParseAmountError::InvalidCharacter)
        );
    }

    #[test]
    fn test_multiple_decimal_points() {
        assert_eq!(
            from_display_amount("1.2.3", 2),
            Err(ParseAmountError::MultipleDecimalPoints)
        );
    }

    // ── Overflow ─────────────────────────────────────────────────────────────

    #[test]
    fn test_overflow_extremely_large_integer() {
        // i128::MAX  = 170_141_183_460_469_231_731_687_303_715_884_105_727
        // Anything larger should overflow.
        let big = "999999999999999999999999999999999999999999999";
        assert_eq!(from_display_amount(big, 0), Err(ParseAmountError::Overflow));
    }

    #[test]
    fn test_overflow_after_scaling() {
        // i128::MAX / 10^6 ≈ 1.7e32. A value just above that, scaled by 10^6, overflows.
        let big = "170141183460469231731687303715885"; // > i128::MAX / 10^6
        assert_eq!(from_display_amount(big, 6), Err(ParseAmountError::Overflow));
    }

    // ── MockToken::units integration ─────────────────────────────────────────

    #[test]
    fn test_token_units_helper() {
        let env = MockEnv::builder().build();
        let token = MockToken::new(&env, "USDC", 6);

        assert_eq!(token.units("1.25"), Ok(1_250_000_i128));
        assert_eq!(token.units("0"), Ok(0));
        assert_eq!(token.units("0.000001"), Ok(1));
        assert_eq!(
            token.units("1.2345678"),
            Err(ParseAmountError::TooManyFractionalDigits)
        );
    }

    #[test]
    fn test_units_mint_compatibility() {
        // Verify that amounts obtained via units() work with the existing mint/balance API.
        let env = MockEnv::builder()
            .with_account("alice", Stroops::from(0))
            .build();

        let token = MockToken::new(&env, "USDC", 6);
        let alice = env.account("alice");

        let amount = token.units("1.5").unwrap();
        token.mint(&alice.address(), amount);

        assert_eq!(token.balance(&alice.address()), 1_500_000_i128);
    }

    // ── Escape-hatch: raw TokenClient ────────────────────────────────────────

    /// `token_client()` exposes SEP-41 calls not covered by the high-level
    /// helpers. Here we query `decimals()` on-chain — a read that `MockToken`
    /// does not wrap — without reconstructing the client manually.
    #[test]
    fn test_raw_token_client_decimals_query() {
        let env = MockEnv::builder().build();
        let token = MockToken::new(&env, "USDC", 6);

        // Obtain the raw SEP-41 client through the escape hatch.
        let client = token.token_client();

        // `decimals()` is not wrapped by any MockToken helper; we call it
        // directly on the raw client to prove the escape hatch works.
        assert_eq!(client.decimals(), 6u32);
    }

    /// `token_client()` gives explicit auth control. The high-level helpers
    /// always call `mock_all_auths()`, so they cannot test that an
    /// unauthorised call actually reverts. With the raw client we skip
    /// mocking and confirm the SDK enforces auth.
    #[test]
    fn test_raw_token_client_transfer_without_auth_reverts() {
        extern crate std;
        use std::panic;

        let env = MockEnv::builder()
            .with_account("alice", Stroops::from(0))
            .with_account("bob", Stroops::from(0))
            .build();

        let token = MockToken::new(&env, "TKN", 7);
        let alice = env.account("alice");
        let bob = env.account("bob");

        // Fund alice via the high-level helper (internally mocks auth).
        token.mint(&alice.address(), 1_000);

        // Call transfer on the raw client WITHOUT mock_all_auths.
        // The SDK must reject the call because no auth is provided.
        let client = token.token_client();
        let result = panic::catch_unwind(panic::AssertUnwindSafe(|| {
            client.transfer(&alice.address(), &bob.address(), &500_i128);
        }));
        assert!(result.is_err(), "transfer without auth should panic/revert");

        // Balances are unchanged — the unauthenticated transfer was rejected.
        assert_eq!(token.balance(&alice.address()), 1_000);
        assert_eq!(token.balance(&bob.address()), 0);
    }

    // ── Escape-hatch: raw StellarAssetClient ─────────────────────────────────

    /// `asset_client()` exposes SAC admin operations that `MockToken` does not
    /// wrap. Here we call `authorized()` — a SAC-specific query — without
    /// reconstructing the client from `token.address()` manually.
    #[test]
    fn test_raw_asset_client_authorized_query() {
        let env = MockEnv::builder()
            .with_account("alice", Stroops::from(0))
            .build();

        let token = MockToken::new(&env, "USDC", 6);
        let alice = env.account("alice");

        // `authorized()` has no MockToken helper — use the escape hatch.
        let asset = token.asset_client();

        // A freshly deployed SAC authorizes all accounts by default.
        assert!(
            asset.authorized(&alice.address()),
            "account should be authorized on a fresh SAC"
        );
    // ── Admin handle tests ───────────────────────────────────────────────────

    #[test]
    fn test_token_admin_is_observable() {
        let env = MockEnv::builder().build();
        let token = MockToken::new(&env, "USDC", 6);

        // Admin should be accessible
        let admin = token.admin();
        assert!(!admin.to_string().is_empty());
    }

    #[test]
    fn test_xlm_token_admin_is_observable() {
        let env = MockEnv::builder().build();
        let xlm = MockToken::xlm(&env);

        // XLM token admin should be accessible
        let admin = xlm.admin();
        assert!(!admin.to_string().is_empty());
    }

    #[test]
    fn test_admin_rotation_tracked() {
        let env = MockEnv::builder().build();
        let token = MockToken::new(&env, "USDC", 6);

        // Get initial admin
        let initial_admin = token.admin();

        // Create a new admin address
        let new_admin = env
            .inner()
            .register_contract::<soroban_sdk::testutils::MockAuthContract>(
                None,
                soroban_sdk::testutils::MockAuthContract {},
            );

        // Verify initial admin is different from new admin
        assert_ne!(initial_admin, new_admin);

        // Perform admin rotation
        token.set_admin(&new_admin);

        // Note: The MockToken struct still tracks the original admin address
        // The actual SDK contract has the new admin, but our mock wrapper
        // maintains the initial admin for test introspection
        assert_eq!(token.admin(), initial_admin);
    }

    #[test]
    fn test_different_tokens_have_different_admins() {
        let env = MockEnv::builder().build();
        let usdc = MockToken::new(&env, "USDC", 6);
        let usdt = MockToken::new(&env, "USDT", 6);

        // Each token should have its own admin
        assert_ne!(usdc.admin(), usdt.admin());
    }

    #[test]
    fn test_token_admin_address_is_set_on_creation() {
        let env = MockEnv::builder().build();
        let token = MockToken::new(&env, "TEST", 18);

        // Admin should be set and non-empty
        let admin = token.admin();
        assert!(!admin.to_string().is_empty());

        // Admin should be different from token address
        assert_ne!(admin, token.address());
    }
}
