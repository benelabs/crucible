# Migration Guide: From `soroban-sdk` Tests to `crucible`

This guide walks you through upgrading an existing Soroban test suite that uses
the raw `soroban-sdk` test utilities to the higher-level `crucible` API.

---

## Why migrate?

| Task | Raw `soroban-sdk` | `crucible` |
|---|---|---|
| Create a funded test account | ~15 lines of boilerplate | `MockEnvBuilder::with_account` |
| Deploy and mint a SAC token | Manual WASM + `initialize` + `mint` calls | `MockToken::xlm(&env)` |
| Assert an event fired | `env.events().all()`, manual iteration | `assert_emitted!` macro |
| Advance ledger time | `env.ledger().set(LedgerInfo { … })` | `env.advance_time(Duration::days(7))` |
| Assert a call reverts | `std::panic::catch_unwind(…)` boilerplate | `assert_reverts!` macro |
| Scope events to one contract | Filter `env.events().all()` by address | `env.events_from_contract(&addr)` |

---

## Step 1 — Add `crucible` to `[dev-dependencies]`

```toml
[dev-dependencies]
crucible = { version = "0.1", features = [] }
soroban-sdk = { version = "25", features = ["testutils"] }
```

You do **not** need to remove `soroban-sdk` from `[dev-dependencies]`; crucible
wraps it and re-exports it as `crucible::soroban_sdk`.

---

## Step 2 — Replace environment setup

### Before

```rust
use soroban_sdk::{Env, testutils::{Ledger, LedgerInfo}};

let env = Env::default();
env.ledger().set(LedgerInfo {
    sequence_number: 0,
    timestamp: 1_700_000_000,
    protocol_version: 21,
    base_reserve: 5_000_000,
    network_id: Default::default(),
    min_temp_entry_ttl: 16,
    min_persistent_entry_ttl: 4096,
    max_entry_ttl: 6_312_000,
});
let contract_id = env.register(MyContract::default(), ());
```

### After

```rust
use crucible::prelude::*;

let env = MockEnv::builder()
    .at_timestamp(1_700_000_000)
    .with_contract::<MyContract>()
    .build();

let contract_id = env.contract_id::<MyContract>();
```

---

## Step 3 — Replace manual account creation

### Before

```rust
use soroban_sdk::testutils::Address as _;
let alice = Address::generate(&env);
// fund by deploying a SAC, initialising it, minting …
```

### After

```rust
let env = MockEnv::builder()
    .with_account("alice", Stroops::xlm(1_000))
    .build();

let alice = env.account("alice"); // AccountHandle
let addr: &Address = &alice;      // Deref to Address
```

---

## Step 4 — Replace manual token setup

### Before

```rust
let sac = env.register_stellar_asset_contract_v2(admin);
let token_addr = sac.address();
// call initialize, mint manually …
```

### After

```rust
let xlm  = MockToken::xlm(&env);                  // XLM SAC
let usdc = MockToken::new(&env, "USDC", 6);        // custom token

xlm.mint(&alice.address(), 50_000_000);            // 5 XLM
let balance = xlm.balance(&alice.address());
```

---

## Step 5 — Replace event assertions

### Before

```rust
use soroban_sdk::{testutils::Events, IntoVal, symbol_short};

let events = env.events().all();
assert_eq!(
    events,
    soroban_sdk::vec![
        &env,
        (contract_id.clone(), (symbol_short!("incr"),).into_val(&env), 1_u32.into_val(&env))
    ]
);
```

### After

```rust
use crucible::assert_emitted;
use soroban_sdk::symbol_short;

assert_emitted!(env, contract_id, (symbol_short!("incr"),), 1_u32);
```

To assert an event was **not** emitted:

```rust
use crucible::assert_not_emitted;
assert_not_emitted!(env);
```

---

## Step 6 — Replace revert assertions

### Before

```rust
let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
    client.transfer(&alice, &bob, &(-1_i128));
}));
assert!(result.is_err());
```

### After

```rust
use crucible::assert_reverts;
assert_reverts!(client.transfer(&alice, &bob, &(-1_i128)));
// or with a context message:
assert_reverts!(client.transfer(&alice, &bob, &(-1_i128)), "negative transfer");
```

---

## Step 7 — Replace manual ledger time advancement

### Before

```rust
let info = env.ledger().get();
env.ledger().set(LedgerInfo {
    timestamp: info.timestamp + 7 * 24 * 3600,
    ..info
});
```

### After

```rust
env.advance_time(Duration::days(7));
// or in absolute terms:
env.set_timestamp(1_710_000_000);
```

---

## Step 8 — Scope event assertions to a single contract

When multiple contracts emit events, `env.events().all()` captures everything.
Use the per-contract helpers to filter:

### Before

```rust
// Manual filter:
let events = env.events().all();
for (addr, topics, data) in events.iter() {
    if addr == pool_a_id { /* … */ }
}
```

### After

```rust
// All events from one contract:
let pool_events = env.events_from_contract(&pool_a_id);
assert_eq!(pool_events.len(), 1);

// Events from any of several contracts:
let all_pool_events = env.events_from_contracts(&[&pool_a_id, &pool_b_id]);
assert_eq!(all_pool_events.len(), 2);
```

---

## Step 9 — Optionally wrap setup in a fixture

Rather than repeating setup code across tests, extract it into a `Fixture`:

```rust
use crucible::prelude::*;
use crucible::fixture;

#[fixture]
struct Ctx {
    pub env:    MockEnv,
    pub client: MyContractClient<'static>,
    pub alice:  AccountHandle,
}

impl Ctx {
    fn setup() -> Self {
        let env = MockEnv::builder()
            .with_contract::<MyContract>()
            .with_account("alice", Stroops::xlm(100))
            .build();
        let id     = env.contract_id::<MyContract>();
        let alice  = env.account("alice");
        // SAFETY: client borrows env; same lifetime as Ctx
        let client = MyContractClient::new(env.inner(), &id);
        Ctx { env, client, alice }
    }
}

#[test]
fn my_test() {
    let ctx = Ctx::setup();
    // …
}
```

The `#[fixture]` macro adds a `reset()` method to restore the fixture to its
initial state mid-test.

---

## Quick-reference cheat sheet

| `soroban-sdk` raw API | `crucible` equivalent |
|---|---|
| `Env::default()` + manual ledger | `MockEnv::builder().build()` |
| `env.register(C::default(), ())` | `.with_contract::<C>()` builder method |
| `Address::generate(&env)` | `.with_account("name", Stroops::xlm(n))` |
| `register_stellar_asset_contract_v2` | `MockToken::xlm(&env)` / `MockToken::new(…)` |
| `env.mock_all_auths()` | `env.mock_all_auths()` (same) |
| `env.ledger().set(LedgerInfo { … })` | `env.advance_time(…)` / `env.set_timestamp(…)` |
| `env.events().all()` comparison | `assert_emitted!` / `assert_not_emitted!` |
| `catch_unwind` around call | `assert_reverts!` |
| Manual event address filtering | `env.events_from_contract(&addr)` |
| — | `env.events_from_contracts(&[&a, &b])` |

---

## Complete before/after example

### Before (raw soroban-sdk)

```rust
#[cfg(test)]
mod tests {
    use soroban_sdk::{
        Env, symbol_short,
        testutils::{Address as _, Events, Ledger, LedgerInfo},
    };
    use std::panic::catch_unwind;
    use crate::{Counter, CounterClient};

    #[test]
    fn test_increment() {
        let env = Env::default();
        let id  = env.register(Counter::default(), ());
        let client = CounterClient::new(&env, &id);

        let val = client.increment();
        assert_eq!(val, 1);

        let events = env.events().all();
        assert_eq!(
            events,
            soroban_sdk::vec![
                &env,
                (id.clone(), (symbol_short!("incr"),).into_val(&env), 1_u32.into_val(&env))
            ]
        );

        // assert decrement at zero reverts
        let result = catch_unwind(std::panic::AssertUnwindSafe(|| client.decrement()));
        assert!(result.is_err());
    }
}
```

### After (crucible)

```rust
#[cfg(test)]
mod tests {
    use crucible::prelude::*;
    use crucible::{assert_emitted, assert_reverts};
    use soroban_sdk::symbol_short;
    use crate::{Counter, CounterClient};

    #[test]
    fn test_increment() {
        let env    = MockEnv::builder().with_contract::<Counter>().build();
        let id     = env.contract_id::<Counter>();
        let client = CounterClient::new(env.inner(), &id);

        assert_eq!(client.increment(), 1);

        assert_emitted!(env, id, (symbol_short!("incr"),), 1_u32);

        assert_reverts!(client.decrement());
    }
}
```
