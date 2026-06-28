# crucible

> A batteries-included testing toolkit for Soroban smart contracts.

[![Crates.io](https://img.shields.io/crates/v/crucible.svg)](https://crates.io/crates/crucible)
[![Docs.rs](https://docs.rs/crucible/badge.svg)](https://docs.rs/crucible)
[![License: MIT](https://img.shields.io/badge/License-MIT-blue.svg)](LICENSE)
[![CI](https://github.com/your-org/crucible/actions/workflows/ci.yml/badge.svg)](https://github.com/your-org/crucible/actions)

---

Writing tests for Soroban contracts today means wiring up environments by hand, copying boilerplate across every repo, and hunting through Stellar docs just to assert that an event fired. **crucible** changes that.

It is a purpose-built Rust testing library for Soroban — analogous to what jest is for JavaScript or hardhat is for Solidity — giving you a rich set of builders, helpers, assertion macros, and fixtures so you can focus on _what_ your contract should do, not on how to set up the harness to prove it.

---

## Table of Contents

- [Motivation](#motivation)
- [Features at a Glance](#features-at-a-glance)
- [Installation](#installation)
- [Quick Start](#quick-start)
- [Core Concepts](#core-concepts)
  - [MockEnv — The Test Environment Builder](#mockenv--the-test-environment-builder)
  - [Pre-funded Accounts](#pre-funded-accounts)
  - [Token Contracts](#token-contracts)
  - [Transaction Simulation](#transaction-simulation)
  - [Event Assertions](#event-assertions)
  - [Gas & Fee Estimation](#gas--fee-estimation)
  - [Custom Fixtures](#custom-fixtures)
- [API Reference](#api-reference)
  - [MockEnvBuilder](#mockenvbuilder)
  - [AccountBuilder](#accountbuilder)
  - [SimulatedTx](#simulatedtx)
  - [Assertion Macros](#assertion-macros)
  - [Gas Helpers](#gas-helpers)
- [Examples](#examples)
  - [Testing a Token Contract](#testing-a-token-contract)
  - [Testing Multi-Party Workflows](#testing-multi-party-workflows)
  - [Time-Dependent Logic](#time-dependent-logic)
  - [Cross-Contract Calls](#cross-contract-calls)
- [Crate Features](#crate-features)
- [Roadmap](#roadmap)
- [Contributing](#contributing)
- [License](#license)

---

## Motivation

Soroban is Stellar's smart contract platform. It runs on WASM, uses Rust as its primary language, and ships with a stellar (pun intended) SDK — but its native test utilities, while functional, are intentionally low-level. That gap shows up fast in real projects:

| Problem | Without crucible | With crucible |
|---|---|---|
| Setting up a funded test account | ~20 lines of boilerplate | `AccountBuilder::new().fund(1_000_000).build(&env)` |
| Registering a standard token and minting | Manual contract deployment + admin calls | `MockToken::xlm(&env)` |
| Asserting a contract event fired | Iterate `env.events().all()`, match manually | `assert_emitted!(env, Transfer { from, to, amount })` |
| Measuring instruction cost | No built-in helpers | `env.measure(|| contract.call())` |
| Advancing ledger time | `env.ledger().set(...)` ceremony | `env.advance_time(Duration::days(7))` |

crucible wraps the official `soroban-sdk` test utilities and builds a fluent, ergonomic layer on top. It does not replace the SDK — it stands alongside it.

---

## Features at a Glance

- **`MockEnvBuilder`** — fluent builder for the Soroban `Env` with sensible defaults, configurable ledger state, and one-liner seeded accounts.
- **Pre-funded accounts** — generate named accounts with arbitrary XLM and custom token balances ready to go.
- **Standard mock tokens** — instant `MockToken` for XLM, USDC, or any arbitrary asset; full admin controls included.
- **Transaction simulation helpers** — wrap contract invocations with fee estimation, auth inspection, and rollback-safe dry-runs.
- **`assert_emitted!` macro** — pattern-match contract events with a concise, readable syntax.
- **`assert_not_emitted!` macro** — verify silence; confirm events that must _not_ fire.
- **Gas & instruction counting** — measure the compute cost of any invocation directly in tests.
- **Ledger time control** — jump forward in time, set arbitrary sequence numbers, or simulate a full epoch change with one call.
- **Fixtures** — re-usable test setup structs with derive support for common patterns.
- **Snapshot testing** — serialize contract state and diff it across test runs.

---

## Installation

Add crucible to the `[dev-dependencies]` section of your contract's `Cargo.toml`. It should never appear in production dependencies.

```toml
[dev-dependencies]
crucible = "0.1"

# The soroban SDK itself — you likely already have this
soroban-sdk = { version = "21", features = ["testutils"] }
```

Enable the `testutils` feature on `soroban-sdk`. crucible depends on it at compile time and will emit a clear error if it is missing.

> **MSRV:** Rust 1.91.0 or later. crucible tracks the same minimum supported Rust version as `soroban-sdk`.

---

## Quick Start

The fastest way to see crucible in action is a single self-contained test. Suppose you have a simple counter contract:

```rust
// src/lib.rs
#![no_std]
use soroban_sdk::{contract, contractimpl, contracttype, Env, Symbol, symbol_short};

#[contracttype]
pub enum DataKey {
    Counter,
}

#[contract]
pub struct CounterContract;

#[contractimpl]
impl CounterContract {
    pub fn increment(env: Env) -> u32 {
        let mut count: u32 = env.storage().instance().get(&DataKey::Counter).unwrap_or(0);
        count += 1;
        env.storage().instance().set(&DataKey::Counter, &count);
        env.events().publish((symbol_short!("counter"), symbol_short!("inc")), count);
        count
    }

    pub fn get(env: Env) -> u32 {
        env.storage().instance().get(&DataKey::Counter).unwrap_or(0)
    }
}
```

A full test with crucible looks like this:

```rust
// src/test.rs
#[cfg(test)]
mod tests {
    use crucible::prelude::*;
    use crate::{CounterContract, CounterContractClient};

    #[test]
    fn test_counter_increments_and_emits_event() {
        // 1. Build a mock environment with a registered contract
        let env = MockEnv::builder()
            .with_contract::<CounterContract>()
            .build();

        let contract_id = env.contract_id::<CounterContract>();
        let client = CounterContractClient::new(&env.inner(), &contract_id);

        // 2. Call the contract
        let result = client.increment();
        assert_eq!(result, 1);

        // 3. Assert the event fired
        assert_emitted!(
            env,
            topics: ("counter", "inc"),
            data: 1_u32
        );

        // 4. Verify idempotency
        let result = client.increment();
        assert_eq!(result, 2);
        assert_eq!(client.get(), 2);
    }
}
```

No manual ledger configuration. No `Env::default()` + `register_contract(...)` ceremonies. Just your contract and your assertions.

---

## Core Concepts

### MockEnv — The Test Environment Builder

`MockEnv` is the entry point for every crucible test. It wraps Soroban's `Env` object and provides a fluent builder interface to configure the test environment before any contract is called.

```rust
let env = MockEnv::builder()
    // Set the ledger sequence number and timestamp
    .at_sequence(1_000)
    .at_timestamp(1_700_000_000)

    // Register contracts ahead of time
    .with_contract::<MyContract>()
    .with_contract::<OtherDependencyContract>()

    // Seed named accounts with XLM
    .with_account("alice", Stroops::xlm(500))
    .with_account("bob",   Stroops::xlm(100))

    // Attach a mock SAC token (Stellar Asset Contract)
    .with_token("USDC", 6)

    // Enable detailed instruction tracking for cost assertions
    .track_costs()

    .build();
```

After calling `.build()` you get a `MockEnv` handle. Use `.inner()` to get the underlying `soroban_sdk::Env` when you need to pass it to auto-generated clients:

```rust
let client = MyContractClient::new(&env.inner(), &env.contract_id::<MyContract>());
```

The `MockEnv` handle also provides all the assertion and time-travel helpers described below.

---

### Pre-funded Accounts

In vanilla Soroban tests, creating an account that can sign transactions requires multiple steps: generate a keypair, construct an `Address`, call `env.ledger().set(...)` to fund it, and store references everywhere. crucible collapses this into one call.

```rust
// During env construction
let env = MockEnv::builder()
    .with_account("alice", Stroops::xlm(10_000))
    .with_account("bob",   Stroops::xlm(500))
    .build();

// Fetch a typed account handle anywhere in the test
let alice = env.account("alice");
let bob   = env.account("bob");

// Use the address wherever Soroban expects one
client.transfer(&alice.address(), &bob.address(), &100_i128);

// Authorize protected calls in tests
env.mock_all_auths();
client.some_protected_call(&alice.address());
```

The `AccountHandle` type gives you:

| Method | Returns | Description |
|---|---|---|
| `.address()` | `Address` | The Soroban address for this account |
| `.xlm_balance()` | `i128` | Current XLM balance in stroops |
| `.token_balance(&token)` | `i128` | Balance in a given `MockToken` |
| `.sign(payload)` | `Vec<u8>` | Sign an arbitrary payload with the account keypair |

Authorization in tests is driven through the environment, not the account handle.
Call `env.mock_all_auths()` to let every `require_auth()` succeed (the common
happy-path style used throughout these examples), or `env.mock_auths(&[])` to
clear authorizations so a protected call is expected to fail.

---

### Token Contracts

Soroban uses the Stellar Asset Contract (SAC) interface for fungible tokens. Setting one up manually in a test involves deploying a WASM blob, calling `initialize`, and minting. crucible provides `MockToken` to do all of that in one line.

```rust
let env = MockEnv::builder()
    .with_account("alice", Stroops::xlm(1_000))
    .build();

// Create a mock XLM SAC token
let xlm = MockToken::xlm(&env);

// Create a custom 6-decimal asset
let usdc = MockToken::new(&env, "USDC", 6);

// Mint tokens to an account
xlm.mint(&env.account("alice").address(), 50_000_000); // 5 XLM in stroops

// Read balances
let balance = usdc.balance(&env.account("alice").address());

// Admin operations
usdc.set_admin(&new_admin_address);
usdc.clawback(&target_address, &amount);
```

`MockToken` implements the full SEP-41 / SAC interface, so you can pass its `Address` directly into any contract that expects a token contract address.

---

### Transaction Simulation

Before committing a call you may want to inspect what a transaction _would_ do — how much it costs, what authorizations it requires, or whether it would succeed. `SimulatedTx` wraps a contract call in a dry-run context.

```rust
let sim = env.simulate(|| {
    client.complex_operation(&alice.address(), &amount)
});

// The call did not actually execute — inspect the results
println!("Estimated fee:         {} stroops",   sim.fee());
println!("Instruction count:     {}",           sim.instructions());
println!("Required auths:        {:?}",         sim.required_auths());
println!("Would succeed:         {}",           sim.would_succeed());

// Commit if you're happy with the results
if sim.would_succeed() {
    sim.commit();
}
```

This is particularly valuable in CI when you want to catch unexpectedly expensive code paths or missing authorization requirements, without writing a separate integration test for each scenario.

#### Inspect-Only Simulations

When you only need to inspect the results without committing, use `simulate_inspect`. This method does not require the closure to be `'static`, allowing you to borrow local clients, accounts, or fixture references:

```rust
// Borrow a local client - no 'static requirement
let client = MyContractClient::new(&env, &contract_id);
let alice = env.account("alice");

let inspected = env.simulate_inspect(|| {
    client.transfer(&alice.address(), &amount)
});

// Inspect the results
println!("Fee: {} stroops", inspected.fee());
println!("Would succeed: {}", inspected.would_succeed());
```

This is particularly useful in test fixtures where you want to simulate calls using borrowed references without cloning or `'static` workarounds.

---

### Event Assertions

Soroban contracts publish events via `env.events().publish(...)`. Asserting those events fired correctly is one of the most common testing needs and one of the most verbose without helpers.

crucible ships `assert_emitted!` and `assert_not_emitted!`:

#### `assert_emitted!`

```rust
// Assert a specific event was emitted by topics + data
assert_emitted!(
    env,
    topics: ("transfer", "v1"),
    data: TransferData { from: alice.address(), to: bob.address(), amount: 100_i128 }
);

// Assert an event from a specific contract
assert_emitted!(
    env,
    contract: &token_address,
    topics: ("mint",),
    data: 1_000_000_i128
);

// Assert at least N events matching the pattern
assert_emitted!(
    env,
    topics: ("approval",),
    count: 3
);

// Assert the _nth_ matching event has specific data
assert_emitted!(
    env,
    topics: ("swap",),
    at_index: 0,
    data: SwapEvent { token_in: xlm.address(), token_out: usdc.address() }
);
```

#### `assert_not_emitted!`

```rust
// Confirm no transfer event was emitted (useful for failure-path tests)
assert_not_emitted!(
    env,
    topics: ("transfer", "v1")
);
```

#### Event captures

If you want to inspect events programmatically rather than with macros:

```rust
let events = env.events_matching(("transfer",));
assert_eq!(events.len(), 2);

let first: TransferData = events[0].data();
assert_eq!(first.amount, 500_i128);
```

---

### Gas & Fee Estimation

crucible exposes the Soroban host instruction meter directly so you can write regression tests against compute cost:

```rust
let env = MockEnv::builder()
    .with_contract::<MyContract>()
    .track_costs()   // required for cost tracking
    .build();

let cost = env.measure(|| {
    client.heavy_computation(&large_input)
});

// Hard limits — fail the test if the contract gets more expensive
assert!(cost.instructions() < 5_000_000, "contract is too expensive: {}", cost.instructions());
assert!(cost.memory_bytes() < 1_024 * 100, "contract uses too much memory");

// Print a human-readable cost summary in CI output
println!("{}", cost.report());
```

The `CostReport` returned by `env.measure()` contains:

| Field | Type | Description |
|---|---|---|
| `instructions()` | `u64` | Total CPU instructions consumed |
| `memory_bytes()` | `u64` | Peak memory allocation in bytes |
| `fee_stroops()` | `i64` | Estimated network fee in stroops; SDK-backed when available |
| `report()` | `String` | Pretty-printed summary table |

You can also store a cost snapshot and assert it does not regress across commits:

```rust
// Write the snapshot on first run; compare on subsequent runs
cost.assert_snapshot("heavy_computation_cost");
```

---

### Custom Fixtures

For complex contracts with many dependencies, test setup code tends to balloon. crucible lets you define `Fixture` structs that encapsulate a fully configured environment so every test starts from a clean, consistent state.

```rust
use crucible::fixture;

#[fixture]
pub struct AmmFixture {
    pub env:      MockEnv,
    pub pool:     Address,
    pub xlm:      MockToken,
    pub usdc:     MockToken,
    pub alice:    AccountHandle,
    pub bob:      AccountHandle,
}

impl AmmFixture {
    pub fn setup() -> Self {
        let env = MockEnv::builder()
            .with_contract::<AmmPool>()
            .with_account("alice", Stroops::xlm(100_000))
            .with_account("bob",   Stroops::xlm(100_000))
            .build();

        let xlm  = MockToken::xlm(&env);
        let usdc = MockToken::new(&env, "USDC", 6);
        let alice = env.account("alice");
        let bob   = env.account("bob");

        // Seed the pool with initial liquidity
        let pool_client = AmmPoolClient::new(&env.inner(), &env.contract_id::<AmmPool>());
        xlm.mint(&alice.address(), 10_000_000);
        usdc.mint(&alice.address(), 10_000_000);
        env.mock_all_auths();
        pool_client.add_liquidity(&xlm.address(), &usdc.address(), &10_000_000_i128, &10_000_000_i128);

        Self { env, pool: env.contract_id::<AmmPool>(), xlm, usdc, alice, bob }
    }
}

// Now every test is one line of setup
#[test]
fn test_swap_changes_price() {
    let f = AmmFixture::setup();
    // ... test logic only
}

#[test]
fn test_insufficient_liquidity_reverts() {
    let f = AmmFixture::setup();
    // ... test logic only
}
```

The `#[fixture]` attribute macro adds a `reset()` method that re-runs `setup()` and replaces `self`, letting you reset mid-test without reconstructing everything from scratch.

---

## API Reference

### MockEnvBuilder

```rust
MockEnv::builder()
    // Ledger configuration
    .at_sequence(seq: u32)                     -> Self
    .at_timestamp(unix_ts: u64)                -> Self
    .with_protocol_version(version: u32)       -> Self

    // Contract registration
    .with_contract<C: Contract>()              -> Self
    .with_contract_at<C: Contract>(id: &Address) -> Self
    .with_wasm(wasm: &[u8])                    -> Self

    // Account seeding
    .with_account(name: &str, balance: Stroops) -> Self

    // Token setup
    .with_token(symbol: &str, decimals: u32)   -> Self

    // Diagnostics
    .track_costs()                             -> Self
    .capture_logs()                            -> Self

    .build()                                   -> MockEnv
```

---

### AccountBuilder

For programmatic account creation outside of the `MockEnvBuilder`:

```rust
let account = AccountBuilder::new(&env)
    .name("charlie")
    .fund_xlm(Stroops::xlm(1_000))
    .fund_token(&usdc, 5_000_000)
    .build();
```

---

### SimulatedTx

```rust
let sim: SimulatedTx<T> = env.simulate(|| client.some_call());

sim.fee()             -> i64       // estimated fee in stroops
sim.instructions()    -> u64       // instruction count
sim.required_auths()  -> Vec<...>  // required auth entries
sim.would_succeed()   -> bool      // whether the call succeeds
sim.result()          -> Option<T> // the return value, if successful
sim.commit()          -> T         // actually execute the call
```

---

### Assertion Macros

```rust
// Assert event was emitted
assert_emitted!(env, topics: (...), data: value);
assert_emitted!(env, contract: &addr, topics: (...), data: value);
assert_emitted!(env, topics: (...), count: n);
assert_emitted!(env, topics: (...), at_index: n, data: value);

// Assert event was NOT emitted
assert_not_emitted!(env, topics: (...));
assert_not_emitted!(env, contract: &addr, topics: (...));

// Assert a call reverts with a specific error
assert_reverts!(client.call(), ContractError::Unauthorized);

// Assert a call reverts with any error
assert_reverts!(client.call());

// Assert approximate equality (useful for fee/reward calculations with rounding)
assert_approx_eq!(actual, expected, tolerance);
```

---

### Gas Helpers

```rust
// Measure cost of a closure
let cost: CostReport = env.measure(|| client.call());

cost.instructions()   -> u64
cost.memory_bytes()   -> u64
cost.fee_stroops()    -> i64
cost.report()         -> String   // formatted table

// Snapshot-based regression testing
cost.assert_snapshot("snapshot_name");           // fails if cost increased > 5%
cost.assert_snapshot_with_tolerance("name", 0.1); // custom 10% tolerance
```

---

### Time Controls

```rust
// Advance the ledger timestamp by a duration
env.advance_time(Duration::days(30));
env.advance_time(Duration::seconds(3600));

// Set absolute ledger time
env.set_timestamp(unix_ts: u64);

// Advance the ledger sequence number
env.advance_sequence(n: u32);

// Jump to a specific sequence
env.set_sequence(n: u32);
```

---

## Examples

The [`examples/`](examples/) directory contains fully-runnable contracts and test
suites covering every crucible feature. Run them all from the repo root:

```bash
cargo test --workspace
```

| Example | Contract | Tests |
|---------|----------|-------|
| **Counter** | [`examples/counter/src/lib.rs`](examples/counter/src/lib.rs) | [`examples/counter/src/test.rs`](examples/counter/src/test.rs) |
| **Token** | [`examples/token/src/lib.rs`](examples/token/src/lib.rs) | [`examples/token/src/test.rs`](examples/token/src/test.rs) |
| **Escrow** | [`examples/escrow/src/lib.rs`](examples/escrow/src/lib.rs) | [`examples/escrow/src/test.rs`](examples/escrow/src/test.rs) |
| **Vesting** | [`examples/vesting/src/lib.rs`](examples/vesting/src/lib.rs) | [`examples/vesting/src/test.rs`](examples/vesting/src/test.rs) |

---

### Testing a Token Contract

```rust
#[cfg(test)]
mod token_tests {
    use crucible::prelude::*;
    use crate::{MyTokenContract, MyTokenContractClient};

    struct TokenFixture {
        env:    MockEnv,
        client: MyTokenContractClient,
        admin:  AccountHandle,
        alice:  AccountHandle,
        bob:    AccountHandle,
    }

    impl TokenFixture {
        fn setup() -> Self {
            let env = MockEnv::builder()
                .with_contract::<MyTokenContract>()
                .with_account("admin", Stroops::xlm(10_000))
                .with_account("alice", Stroops::xlm(10_000))
                .with_account("bob",   Stroops::xlm(10_000))
                .build();

            let admin  = env.account("admin");
            let alice  = env.account("alice");
            let bob    = env.account("bob");
            let client = MyTokenContractClient::new(&env.inner(), &env.contract_id::<MyTokenContract>());

            env.mock_all_auths();
            client.initialize(&admin.address(), &7_u32, &"My Token".into(), &"MTK".into());

            Self { env, client, admin, alice, bob }
        }
    }

    #[test]
    fn test_mint_emits_event_and_updates_balance() {
        let f = TokenFixture::setup();

        f.client.mint(&f.alice.address(), &1_000_i128);

        assert_eq!(f.client.balance(&f.alice.address()), 1_000_i128);

        assert_emitted!(
            f.env,
            topics: ("mint",),
            data: MintEvent { to: f.alice.address(), amount: 1_000_i128 }
        );
    }

    #[test]
    fn test_transfer_moves_balance_between_accounts() {
        let f = TokenFixture::setup();

        f.client.mint(&f.alice.address(), &500_i128);
        f.client.transfer(&f.alice.address(), &f.bob.address(), &200_i128);

        assert_eq!(f.client.balance(&f.alice.address()), 300_i128);
        assert_eq!(f.client.balance(&f.bob.address()),   200_i128);

        assert_emitted!(
            f.env,
            topics: ("transfer",),
            data: TransferEvent {
                from:   f.alice.address(),
                to:     f.bob.address(),
                amount: 200_i128,
            }
        );
    }

    #[test]
    fn test_transfer_without_auth_reverts() {
        let f = TokenFixture::setup();

        f.client.mint(&f.alice.address(), &500_i128);

        // Clear mocked auth so require_auth() is enforced — should revert
        f.env.mock_auths(&[]);
        assert_reverts!(
            f.client.transfer(&f.alice.address(), &f.bob.address(), &200_i128)
        );

        // Balances must be unchanged
        assert_eq!(f.client.balance(&f.alice.address()), 500_i128);
        assert_eq!(f.client.balance(&f.bob.address()),   0_i128);

        assert_not_emitted!(f.env, topics: ("transfer",));
    }
}
```

---

### Testing Multi-Party Workflows

```rust
#[test]
fn test_escrow_full_lifecycle() {
    let env = MockEnv::builder()
        .with_contract::<EscrowContract>()
        .with_account("buyer",    Stroops::xlm(50_000))
        .with_account("seller",   Stroops::xlm(1_000))
        .with_account("arbiter",  Stroops::xlm(1_000))
        .build();

    let xlm     = MockToken::xlm(&env);
    let buyer   = env.account("buyer");
    let seller  = env.account("seller");
    let arbiter = env.account("arbiter");
    let client  = EscrowContractClient::new(&env.inner(), &env.contract_id::<EscrowContract>());

    // 1. Buyer creates escrow
    xlm.mint(&buyer.address(), 10_000_i128);
    env.mock_all_auths();
    let escrow_id = client.create(
        &buyer.address(),
        &seller.address(),
        &arbiter.address(),
        &xlm.address(),
        &10_000_i128,
    );

    assert_emitted!(env, topics: ("escrow", "created"), data: escrow_id);

    // 2. Advance time past lock period
    env.advance_time(Duration::days(3));

    // 3. Seller claims — arbiter approves
    client.approve(&escrow_id);
    client.claim(&escrow_id, &seller.address());

    assert_eq!(xlm.balance(&seller.address()), 10_000_i128);
    assert_eq!(xlm.balance(&buyer.address()),  0_i128);

    assert_emitted!(env, topics: ("escrow", "claimed"), data: escrow_id);
}
```

---

### Time-Dependent Logic

```rust
#[test]
fn test_vesting_cliff_is_enforced() {
    let env = MockEnv::builder()
        .with_contract::<VestingContract>()
        .with_account("beneficiary", Stroops::xlm(1_000))
        .at_timestamp(1_700_000_000)
        .build();

    let xlm         = MockToken::xlm(&env);
    let beneficiary = env.account("beneficiary");
    let client      = VestingContractClient::new(&env.inner(), &env.contract_id::<VestingContract>());

    let cliff_seconds: u64 = 90 * 24 * 3600; // 90 days

    xlm.mint(&env.contract_id::<VestingContract>(), 100_000_i128);
    client.initialize(&beneficiary.address(), &cliff_seconds, &100_000_i128);

    env.mock_all_auths();

    // Attempt to claim before cliff — must fail
    assert_reverts!(client.claim());

    // Advance to just before cliff
    env.advance_time(Duration::days(89));
    assert_reverts!(client.claim());

    // Advance past cliff
    env.advance_time(Duration::days(2)); // total: 91 days
    client.claim(); // should succeed now

    let balance = xlm.balance(&beneficiary.address());
    assert!(balance > 0, "beneficiary should have received vested tokens");
}
```

---

### Cross-Contract Calls

```rust
#[test]
fn test_aggregator_calls_multiple_pools() {
    let env = MockEnv::builder()
        .with_contract::<Aggregator>()
        .with_contract::<PoolA>()
        .with_contract::<PoolB>()
        .with_account("trader", Stroops::xlm(10_000))
        .build();

    let xlm    = MockToken::xlm(&env);
    let usdc   = MockToken::new(&env, "USDC", 6);
    let trader = env.account("trader");

    // Seed both pools
    xlm.mint(&env.contract_id::<PoolA>(),  500_000_i128);
    usdc.mint(&env.contract_id::<PoolA>(), 500_000_i128);
    xlm.mint(&env.contract_id::<PoolB>(),  200_000_i128);
    usdc.mint(&env.contract_id::<PoolB>(), 200_000_i128);

    // Give trader tokens to swap
    xlm.mint(&trader.address(), 1_000_i128);

    let agg_client = AggregatorClient::new(&env.inner(), &env.contract_id::<Aggregator>());

    env.mock_all_auths();
    let out_amount = agg_client.best_swap(
        &xlm.address(),
        &usdc.address(),
        &1_000_i128,
        &trader.address(),
    );

    assert!(out_amount > 0);
    assert_eq!(xlm.balance(&trader.address()), 0_i128);
    assert_eq!(usdc.balance(&trader.address()), out_amount);

    // Verify the aggregator routed through exactly one pool
    assert_emitted!(env, topics: ("swap",), count: 1);
}
```

---

## Crate Features

| Feature | Default | Description |
|---|---|---|
| `std` | No | Enable `std` support (required for snapshot testing) |
| `snapshots` | No | Snapshot-based cost regression testing |
| `derive` | Yes | Enable `#[fixture]` and related derive macros |
| `token-mocks` | Yes | Include the `MockToken` / SAC helpers |
| `serde` | No | Serialize/deserialize fixtures and cost reports |

Enable optional features in `Cargo.toml`:

```toml
[dev-dependencies]
crucible = { version = "0.1", features = ["snapshots", "serde"] }
```

---

## Roadmap

### v0.1 — Foundation
- [ ] `MockEnvBuilder` with ledger configuration
- [ ] Pre-funded account helpers (`AccountBuilder`, `AccountHandle`)
- [ ] `MockToken` (SAC interface)
- [ ] `assert_emitted!` / `assert_not_emitted!` macros
- [ ] `assert_reverts!` macro

### v0.2 — Cost Awareness
- [ ] `env.measure()` instruction tracking
- [ ] `CostReport` with human-readable output
- [ ] Snapshot-based regression testing
- [ ] `SimulatedTx` dry-run API

### v0.3 — Fixtures & DX
- [ ] `#[fixture]` derive macro
- [ ] `env.advance_time()` / `env.advance_sequence()`
- [ ] Named event captures
- [ ] CLI report output for CI integration

### v0.4 — Ecosystem
- [ ] Pre-built mocks for common Soroban contracts (DEX, lending, multisig)
- [ ] Integration with `soroban-cli` test runner output format
- [ ] VSCode extension for inline cost annotations

---

## Contributing

Contributions are very welcome. crucible is designed to be contributor-friendly with well-scoped, independently shippable issues.

### Good first issues
- Add a mock for the Soroban token contract's `allowance` flow
- Add `assert_approx_eq!` macro with configurable tolerance
- Write docs and usage examples for `SimulatedTx`
- Add `env.events_matching()` ergonomics for programmatic event inspection
- Set up GitHub Actions CI with `cargo test` and `cargo clippy`

### Getting started

```bash
git clone https://github.com/your-org/crucible
cd crucible
cargo test
```

All tests should pass with a standard Rust toolchain and no additional dependencies. The library uses `soroban-sdk` in test mode only, so no WASM toolchain is required to work on the library itself.

### Conventions
- Run `cargo clippy -- -D warnings` before opening a PR.
- Run `cargo fmt` before opening a PR.
- Every public API should have a doc comment with at least one example.
- New macros need both a positive test and a negative test.

---

## License

MIT — see [LICENSE](LICENSE).

---

> "Gold is tested by fire, character by temptation — and contracts by crucible."
