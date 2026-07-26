# Crucible Architecture Overview

This document describes the high-level architecture of the Crucible project, clarifying crate ownership, module boundaries, and dependency directions to help new contributors understand where functionality belongs.

## Project Overview

Crucible is a **batteries-included testing toolkit for Soroban smart contracts**, analogous to Jest (JavaScript) or Hardhat (Solidity). It combines:

- A **core Soroban testing toolkit** with builders, helpers, assertion macros, and fixtures
- A **proc-macro crate** providing derive support
- **Example contracts** demonstrating testing patterns
- **Application contracts** (treasury, governance, etc.)
- **A backend platform** providing performance profiling, analytics, and deployment infrastructure

## Repository Structure

```
crucible/
├── contracts/
│   ├── crucible/              [CORE TOOLKIT - Testing Framework]
│   ├── treasury/              [Application Contract - Treasury]
│   ├── governance/            [Application Contract - Governance]
│   ├── insurance/             [Application Contract - Insurance]
│   ├── oracle/                [Application Contract - Oracle]
│   └── supply_chain/          [Application Contract - Supply Chain]
├── crucible-macros/           [Proc-Macro Crate - Derives for toolkit]
├── examples/                  [Example Contracts - Demonstrate testing patterns]
│   ├── counter/
│   ├── token/
│   ├── vesting/
│   ├── staking/
│   ├── cross-contract/
│   ├── lending/
│   └── [other examples...]
├── backend/                   [API Platform - Analytics & Profiling]
│   ├── src/
│   │   ├── main.rs           [Axum HTTP server entry point]
│   │   ├── api/              [HTTP handlers & routes]
│   │   ├── db/               [Database access layer]
│   │   ├── services/         [Business logic]
│   │   ├── workers/          [Background jobs]
│   │   ├── config/           [Configuration management]
│   │   └── utils/            [Utilities & helpers]
│   ├── migrations/           [SQL schema migrations]
│   ├── benches/              [Performance benchmarks]
│   └── tests/                [Integration & E2E tests]
├── src/                       [Workspace root (unused)]
├── Cargo.toml                [Workspace manifest]
├── CONTRIBUTING.md           [Contribution guidelines]
├── ARCHITECTURE.md           [This file]
└── README.md                 [Project overview]
```

## Core Crates & Their Roles

### 1. **crucible** (Core Testing Toolkit)
**Location:** `contracts/crucible/`  
**Purpose:** A Rust testing library for Soroban smart contracts  
**Key Concepts:**
- `MockEnv` — Fluent builder for the Soroban `Env` test object
- `MockToken` — Pre-built mock Stellar Asset Contract (SAC) tokens
- `AccountBuilder` — Helpers for creating pre-funded test accounts
- `assert_emitted!` / `assert_not_emitted!` — Event assertion macros
- **Fixtures** — Re-usable test setup components
- **Snapshot testing** — State serialization and diffing

**Dependencies:**
- `soroban-sdk` (with `testutils` feature) — Official Soroban SDK
- `crucible-macros` — Proc-macro support (via optional `derive` feature)
- `serde` / `serde_json` — Serialization (for snapshots feature)

**Public API Entry Point:**
```rust
pub use crucible::prelude::*;
```

**Guidelines:**
- ✅ Should remain **independent** of the backend
- ✅ Can depend on `soroban-sdk` and community crates
- ❌ Should **NOT** import from `backend` or `examples`
- ✅ Focuses on test utilities and builders
- ✅ All public APIs exported via `prelude` module

**Example Usage:**
```rust
#[test]
fn test_contract() {
    let env = MockEnv::builder()
        .with_contract::<MyContract>()
        .with_account("alice", Stroops::xlm(1_000))
        .build();
    
    let token = MockToken::usdc(&env);
    // ... test code
}
```

---

### 2. **crucible-macros** (Proc-Macro Crate)
**Location:** `crucible-macros/`  
**Purpose:** Provides derive macros and code generation for the toolkit  
**Key Features:**
- Derive support for fixtures and test helpers
- Code generation for common testing patterns
- Compile-time validation of test structures

**Dependencies:**
- `proc-macro2`, `quote`, `syn` — Standard macro/AST libraries

**Guidelines:**
- ✅ **Only** used by crucible and tests
- ✅ Implements code generation for the testing framework
- ❌ Should **NOT** contain runtime logic
- ❌ Should **NOT** depend on the backend

**Example Usage:**
```rust
#[derive(Fixture)]
struct MyTestSetup {
    env: MockEnv,
    contract_id: ContractId,
}
```

---

### 3. **backend** (API Platform)
**Location:** `backend/`  
**Purpose:** High-performance Rust API server providing:
- Performance profiling and analytics dashboards
- Build error analytics
- Audit logging and security monitoring
- Contract deployment tracking
- Background job processing via Redis

**Architecture:**
```
Clients (HTTP)
    ↓
Axum HTTP Server (async/await)
    ├→ Middleware: CORS, Tracing, Compression, Request ID
    ├→ API Routes: /api/v1/*, /health/*, /metrics
    ├→ Error Handling: Structured AppError responses
    └→ Database & Cache Layers
        ├→ PostgreSQL (via SQLx, compile-time checked)
        ├→ Redis (via redis-rs)
        └→ Worker Queue (via Apalis)
```

**Key Modules:**
- `api/handlers/` — HTTP endpoint handlers
- `db/` — Database queries and models
- `services/` — Business logic (alerts, error analytics, audit logs)
- `workers/` — Background job handlers
- `config/` — Configuration and environment management
- `telemetry/` — Tracing and observability

**API Endpoints:**
- Health checks: `GET /health/live`, `GET /health/ready`
- Metrics: `GET /metrics` (Prometheus format)
- Alerts: `GET|POST /api/alerts/rules`, `POST /api/alerts/ingest`
- Audit: `GET|POST /api/v1/audit/*`
- Errors: `GET /api/v1/errors/dashboard/*`

**Dependencies:**
- `axum` — Web framework
- `tokio` — Async runtime
- `sqlx` — PostgreSQL driver (compile-time checked queries)
- `redis` — Caching and job queue
- `soroban-sdk` — For contract type definitions and serialization
- `tracing` + `opentelemetry` — Observability

**Guidelines:**
- ✅ **Independent** service with its own database and API
- ✅ Can use `soroban-sdk` for contract types (not testing)
- ❌ Should **NOT** depend on crucible or examples
- ✅ Focuses on operational metrics and analytics
- ✅ Can be deployed and scaled independently

**Example Handler:**
```rust
#[get("/api/v1/errors/dashboard/build-errors")]
async fn get_build_error_analytics(
    State(db): State<PgPool>,
    State(redis): State<RedisPool>,
) -> Result<Json<BuildErrorMetrics>> {
    // Fetch from cache or database
}
```

---

### 4. **Examples** (Contract Demonstrations)
**Location:** `examples/*/`  
**Purpose:** Reference implementations of Soroban contracts with comprehensive tests  
**Key Examples:**
- `counter/` — Simple counter contract
- `token/` — Token contract with transfer logic
- `vesting/` — Time-locked fund release
- `staking/` — Staking and rewards mechanisms
- `cross-contract/` — Inter-contract communication patterns
- `lending/` — DeFi lending protocol
- `multisig/` — Multi-signature wallet
- `nft-marketplace/` — NFT trading
- `prediction-market/` — Prediction market
- `dex/` — Decentralized exchange

**Test Pattern:**
```rust
#[cfg(test)]
mod tests {
    use crucible::prelude::*;
    use crate::{MyContract, MyContractClient};

    #[test]
    fn test_example() {
        let env = MockEnv::builder()
            .with_contract::<MyContract>()
            .build();
        
        let client = MyContractClient::new(&env.inner(), &env.contract_id::<MyContract>());
        // ... test assertions
    }
}
```

**Guidelines:**
- ✅ **Only** depend on crucible (for testing)
- ✅ Demonstrate testing best practices
- ✅ Serve as templates for new contracts
- ❌ Should **NOT** depend on backend
- ✅ Each example is self-contained

**Dependencies:**
- `soroban-sdk` — Soroban programming framework
- `crucible` (dev-dependency) — Testing framework

---

### 5. **Application Contracts** (Business Logic)
**Location:** `contracts/[treasury|governance|insurance|oracle|supply_chain]/`  
**Purpose:** Production smart contracts using the Soroban SDK  
**Examples:**
- `treasury/` — Treasury management and fund allocation
- `governance/` — Governance and voting mechanisms
- `insurance/` — Insurance fund and claim processing
- `oracle/` — Price feeds and data oracle
- `supply_chain/` — Supply chain tracking

**Testing Pattern:**
```rust
#[cfg(test)]
mod tests {
    use crucible::prelude::*;
    use crate::{TreasuryContract, TreasuryContractClient};

    #[test]
    fn test_allocation() {
        let env = MockEnv::builder()
            .with_contract::<TreasuryContract>()
            .with_token("USDC", 6)
            .build();
        // ... test
    }
}
```

**Guidelines:**
- ✅ Use crucible for comprehensive testing
- ✅ Follow testing patterns from examples
- ✅ Can have integration with other contracts
- ❌ Should **NOT** depend on backend (backend may query them)

---

## Dependency Graph

```
┌─────────────────────────────────────────────────────────────────┐
│                      soroban-sdk (base)                         │
│                   (official Stellar library)                    │
└─────────────────────────────────────────────────────────────────┘
                    ↓                           ↓
        ┌──────────────────────┐    ┌──────────────────────┐
        │   crucible-macros    │    │    Examples (Rust)   │
        │  (proc macros only)  │    │  Example Contracts   │
        └──────────────────────┘    └──────────────────────┘
                    ↓                           ↓
        ┌──────────────────────┐    ┌──────────────────────┐
        │  crucible (toolkit)  │←───│  Example Tests ✓     │
        │  ├─ MockEnv          │    │  Use MockEnv, etc.   │
        │  ├─ MockToken        │    └──────────────────────┘
        │  ├─ Fixtures         │
        │  └─ Assertions       │
        └──────────────────────┘
                    ↓
        ┌──────────────────────┐
        │ App Contracts (Rust) │    ┌──────────────────────┐
        │ ├─ Treasury          │    │   backend (Axum)     │
        │ ├─ Governance        │    │  ├─ PostgreSQL       │
        │ ├─ Insurance         │    │  ├─ Redis            │
        │ ├─ Oracle            │    │  ├─ API Handlers     │
        │ └─ Supply Chain      │    │  ├─ Workers          │
        │        ↓             │    │  └─ Telemetry        │
        │  Contract Tests ✓    │    └──────────────────────┘
        │  Use crucible        │
        └──────────────────────┘
```

### Dependency Rules

| From | To | Allowed? | Reason |
|------|----|---------:|--------|
| `examples` | `crucible` | ✅ Yes | Examples demonstrate toolkit usage |
| `crucible` | `examples` | ❌ No | Would create circular dependency |
| `app-contracts` | `crucible` | ✅ Yes (test only) | Dev-dependency for testing |
| `crucible` | `backend` | ❌ No | Toolkit must be independent |
| `backend` | `crucible` | ❌ No | Backend is separate service |
| `examples` | `backend` | ❌ No | Examples are standalone |
| `crucible-macros` | `crucible` | ✅ Yes | Macros provide derives for toolkit |
| `crucible` | `crucible-macros` | ✅ Yes (optional) | Optional feature for derives |
| `backend` | `soroban-sdk` | ✅ Yes | For contract types (not testing) |

---

## How Tests Use Crucible Components

### Test Flow Diagram

```
┌──────────────────────────────────────────────────────────┐
│                  Test Execution Flow                     │
├──────────────────────────────────────────────────────────┤
│                                                          │
│  1. Setup Phase                                         │
│     ├─ MockEnv::builder()                              │
│     │   └─ Creates test Soroban Env with defaults      │
│     ├─ .with_contract::<MyContract>()                  │
│     │   └─ Registers contract in test environment      │
│     ├─ .with_account("alice", Stroops::xlm(1_000))    │
│     │   └─ Creates funded test account                 │
│     ├─ .with_token("USDC", 6)                          │
│     │   └─ Deploys MockToken SAC                       │
│     └─ .build()                                         │
│        └─ Returns MockEnv instance ready for tests     │
│                                                          │
│  2. Fixture Assembly (Optional)                         │
│     ├─ Define reusable test setup struct:              │
│     │   struct ContractFixture {                       │
│     │       env: MockEnv,                              │
│     │       contract_id: ContractId,                   │
│     │       alice_account: Address,                    │
│     │   }                                               │
│     └─ Instantiate with builder output                 │
│                                                          │
│  3. Execution Phase                                     │
│     ├─ Create contract client:                         │
│     │   let client = MyContractClient::new(            │
│     │       &env.inner(), &contract_id                 │
│     │   )                                               │
│     ├─ Invoke contract methods:                         │
│     │   let result = client.transfer(                  │
│     │       &from, &to, &amount                        │
│     │   )                                               │
│     └─ Contract executes in test env                   │
│                                                          │
│  4. Assertion Phase                                     │
│     ├─ assert_emitted!(env,                            │
│     │   topics: ("transfer"),                          │
│     │   data: (from, to, amount)                       │
│     │ ) — Pattern-match emitted events                 │
│     ├─ assert_not_emitted!(env, ...)                   │
│     │   — Verify events that must NOT occur            │
│     ├─ assert_eq!(result, expected)                    │
│     │   — Standard Rust assertions                     │
│     └─ Assertions fail fast on mismatch                │
│                                                          │
│  5. Cleanup & State Restoration                         │
│     └─ Test isolation: each test gets fresh MockEnv    │
│        No shared state between tests                    │
│                                                          │
└──────────────────────────────────────────────────────────┘
```

### MockEnv Builder Pattern

```rust
let env = MockEnv::builder()              // 1. Create builder
    .at_sequence(1_000)                   // 2. Set ledger height
    .at_timestamp(1_700_000_000)          // 3. Set ledger time
    .with_contract::<MyContract>()        // 4. Register contract
    .with_account("alice", stroops)       // 5. Create account
    .with_token("USDC", 6)                // 6. Deploy SAC token
    .track_costs()                        // 7. Enable cost tracking
    .build();                             // 8. Finalize → MockEnv

// Then use:
env.contract_id::<MyContract>()           // Get registered contract ID
env.account("alice")                      // Get account by name
env.advance_time(Duration::days(7))       // Jump forward in time
env.measure(|| { /* invocation */ })      // Measure instruction cost
assert_emitted!(env, ...)                 // Assert events
```

### MockToken Usage Pattern

```rust
// XLM token (native)
let xlm = MockToken::xlm(&env);

// Standard issued assets
let usdc = MockToken::issued_asset(&env, "USDC", 6);
let eurc = MockToken::issued_asset(&env, "EURC", 6);

// Token operations
xlm.mint(&alice, 1_000_000);              // Mint tokens to account
xlm.transfer(&alice, &bob, 50_000);       // Transfer between accounts
xlm.approve(&alice, &contract, 100_000);  // Approve contract spend
let balance = xlm.balance(&alice);        // Check account balance
```

### Fixture Reuse Pattern

```rust
// Define once
#[derive(Fixture)]
struct DexFixture {
    env: MockEnv,
    pool_id: ContractId,
    token_a: MockToken,
    token_b: MockToken,
    alice: Address,
    bob: Address,
}

// Use in multiple tests
#[test]
fn test_swap() {
    let fix = DexFixture::setup();
    // execute swap
}

#[test]
fn test_liquidity_provision() {
    let fix = DexFixture::setup();
    // add liquidity
}

#[test]
fn test_fee_accrual() {
    let fix = DexFixture::setup();
    // verify fees
}
```

---

## Where New Functionality Belongs

### Deciding Ownership: Decision Tree

```
Is this code about testing Soroban contracts?
├─ YES → Does it help write cleaner tests?
│  ├─ YES → Is it a builder, assertion macro, or fixture?
│  │  └─ YES → Add to `crucible/src/lib.rs` 📍
│  │  └─ NO → Add to `examples/` as pattern 📍
│  └─ NO → This is a contract example
│     └─ Add to `examples/name/src/lib.rs` 📍
└─ NO → Is this business logic for a Soroban contract?
   ├─ YES → Add to `contracts/[app]/src/lib.rs` 📍
   └─ NO → Is this backend API/analytics functionality?
      └─ YES → Add to `backend/src/` 📍
```

### Concrete Examples

| Scenario | Location | Reason |
|----------|----------|--------|
| New assertion macro for contract events | `crucible/src/assertions.rs` | Toolkit feature; used by all tests |
| Example: Voting contract | `examples/voting/src/lib.rs` | Demonstrates testing patterns |
| Tests for voting example | `examples/voting/src/tests.rs` | Use crucible's MockEnv, MockToken, etc. |
| Treasury contract tests | `contracts/treasury/src/tests.rs` | Use crucible (dev-dependency) |
| API endpoint for build stats | `backend/src/api/handlers/metrics.rs` | Operational analytics |
| New derive macro for fixtures | `crucible-macros/src/lib.rs` | Proc-macro support for toolkit |
| Ledger time manipulation helper | `crucible/src/time_helpers.rs` | Test utility for all contracts |

---

## Module Boundaries & Visibility

### crucible (Testing Toolkit)

**Public Modules:**
```rust
pub mod prelude;           // Re-exports all common types/macros
pub mod builders;          // MockEnvBuilder, AccountBuilder
pub mod tokens;            // MockToken, TokenOperations
pub mod fixtures;          // Fixture trait and derives
pub mod assertions;        // assert_emitted!, assert_not_emitted!
pub mod time;              // Time manipulation helpers
pub mod cost;              // Instruction/gas measurement
pub mod snapshots;         // Snapshot testing (feature-gated)
```

**Private Modules:**
```rust
mod utils;                 // Internal helpers (not re-exported)
mod errors;                // Internal error handling
```

**Prelude (Primary Entry Point):**
```rust
pub use crate::builders::{MockEnv, MockEnvBuilder, AccountBuilder};
pub use crate::tokens::MockToken;
pub use crate::fixtures::Fixture;
pub use crate::assertions::{assert_emitted, assert_not_emitted};
pub use crate::time::*;
pub use crate::cost::*;
```

### backend

**Public API Modules:**
```rust
pub mod api;               // HTTP handlers
pub mod db;                // Database layer
pub mod config;            // Configuration
pub mod telemetry;         // Tracing setup
```

**Implementation Modules:**
```rust
mod services;              // Business logic
mod workers;               // Job handlers
mod error;                 // Error types
mod utils;                 // Helpers
```

---

## Reducing Accidental Coupling

### ✅ Best Practices

1. **Import from `prelude` only** in test files
   ```rust
   use crucible::prelude::*;  // ✅ Stable imports
   ```

2. **Don't import internal modules** in production code
   ```rust
   use crucible::utils::internal_helper;  // ❌ Avoid this
   ```

3. **Use feature flags** for optional dependencies
   ```toml
   [features]
   snapshots = ["serde", "serde_json"]
   ```

4. **Backend should not know about test utilities**
   ```rust
   // backend/src/lib.rs
   use soroban_sdk;                // ✅ Types only
   use crucible;                    // ❌ Never do this
   ```

5. **Examples are templates, not library code**
   ```rust
   // examples/ can depend on crucible (for testing)
   // but examples should NOT be imported by other crates
   ```

6. **Contract tests use dev-dependencies**
   ```toml
   [dev-dependencies]
   crucible = { path = "../../contracts/crucible" }
   ```

### ❌ Anti-Patterns to Avoid

| Anti-Pattern | Problem | Solution |
|--------------|---------|----------|
| `use crucible::utils::*` in app code | Relies on internals; breaks on refactor | Use `prelude` only |
| `backend` depends on examples | Tight coupling; examples change frequently | Backend should be independent |
| `crucible` depends on `backend` | Circular; toolkit can't be used standalone | One-way dependency only |
| Examples import from other examples | Code duplication; hard to maintain | Share via crucible toolkit |
| Contract tests use `backend` utilities | Tests depend on infrastructure | Use only crucible for testing |

---

## Architecture Checklist for Contributors

When adding new code, verify:

- [ ] **Correct location?** Used decision tree in "Where New Functionality Belongs"
- [ ] **No accidental coupling?** Checked dependency rules in "Dependency Graph"
- [ ] **Using public APIs?** Imported from `prelude`, not internal modules
- [ ] **Test coverage?** Added tests in appropriate location
- [ ] **Documentation?** Added module-level docs explaining the feature
- [ ] **Backwards compatible?** Won't break existing code
- [ ] **Feature-gated?** Optional dependencies use Cargo features
- [ ] **No circular imports?** Verified with `cargo check`

---

## Quick Reference: What Goes Where

| Code Type | Location | Example |
|-----------|----------|---------|
| Test framework helpers | `crucible/src/` | `builders.rs`, `tokens.rs` |
| Assertion macros | `crucible/src/assertions.rs` | `assert_emitted!` |
| Derive macros | `crucible-macros/src/` | `#[derive(Fixture)]` |
| Example contract | `examples/name/src/lib.rs` | Counter, Token |
| Example tests | `examples/name/src/tests.rs` | Uses `MockEnv` from crucible |
| App contract | `contracts/app/src/lib.rs` | Treasury, Governance |
| App contract tests | `contracts/app/src/tests.rs` | Uses `MockEnv` from crucible |
| API endpoint | `backend/src/api/handlers/` | `/api/v1/metrics`, `/health` |
| Database queries | `backend/src/db/` | Query builders, migrations |
| Background jobs | `backend/src/workers/` | Async task handlers |
| Configuration | `backend/src/config/` | Environment, settings |

---

## Development Workflow

### To Run Tests Locally

```bash
# Run all toolkit tests
cargo test -p crucible

# Run specific example tests
cargo test -p crucible-example-counter

# Run app contract tests
cargo test -p treasury

# Run backend tests
cargo test -p backend
```

### To Add a New Feature

1. **Determine ownership** using decision tree above
2. **Add implementation** in correct location
3. **Add tests** (unit or integration as appropriate)
4. **Update docs** (module comments, examples)
5. **Verify dependencies** with `cargo check`
6. **Test in examples** to ensure no coupling

### To Review a PR

Verify:
- Code is in the right location
- No accidental cross-crate dependencies
- Tests are comprehensive
- Public API additions are in `prelude`
- No breaking changes to existing interfaces

---

## Glossary

| Term | Definition |
|------|-----------|
| **MockEnv** | Test environment builder that wraps `soroban_sdk::Env` |
| **MockToken** | Pre-built Stellar Asset Contract for testing |
| **Fixture** | Reusable test setup; can be derived with `#[derive(Fixture)]` |
| **assert_emitted!** | Macro to verify contract events were emitted |
| **SAC** | Stellar Asset Contract; standardized contract for managing assets |
| **Proc-macro** | Compile-time code generation (in `crucible-macros`) |
| **prelude** | Re-export module with commonly used types and macros |
| **Stroops** | Smallest unit of XLM (1 XLM = 10,000,000 stroops) |
| **Ledger** | Soroban's persistent storage; includes time, sequence number, state |

---

## Further Reading

- [Architecture Decision Records](docs/adr/) — Index of all ADRs
- [Soroban Documentation](https://soroban.stellar.org/)
- [Cargo Workspaces](https://doc.rust-lang.org/cargo/reference/workspaces.html)
- [Proc Macros](https://doc.rust-lang.org/reference/procedural-macros.html)
- [Axum Web Framework](https://github.com/tokio-rs/axum)

---

## Questions & Feedback

For questions about this architecture:
1. Check this document and the decision trees
2. Ask in project discussions
3. Review existing examples (`examples/*/`)
4. Refer to the Soroban SDK documentation

Last Updated: June 2026
