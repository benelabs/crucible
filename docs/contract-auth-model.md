# Contract Initialization and Authorization Model

This document describes who may initialize and administer the `proxy` and
`oracle` contracts.

## Proxy (`contracts/proxy`)

- `initialize(admin, implementation)` requires `admin.require_auth()` and can
  succeed exactly once. A second call fails with `ProxyError::AlreadyInitialized`.
  Because the named admin must authorize the call, a front-running account cannot
  install an address it does not control as admin.
- `upgrade`, `set_implementation` and `set_admin` load the stored admin and require
  its authorization. Before initialization they fail with `ProxyError::NotInitialized`.
- Deployment: deploy and call `initialize` in the same transaction, or immediately
  after, signed by the intended admin. An uninitialized proxy has no admin; the
  first valid initialization wins, and only the admin named in it can authorize it.
- Upgrade trust model: the proxy admin can replace the contract WASM. Use a
  multisig or governance account as the admin for production deployments.

## Oracle (`contracts/oracle`)

- `initialize(admin)` requires `admin.require_auth()` and is one-time
  (`OracleError::AlreadyInitialized` on repeat).
- `register_source`, `activate_source` and `deactivate_source` require the stored
  admin's authorization. Before initialization they fail with
  `OracleError::NotInitialized`.
- `submit_price` requires the submitting source's own authorization, and the
  source must be whitelisted and active. Prices must be positive and may not
  deviate more than 15% from the previous observation.
- Price reads reject observations older than one hour (`OracleError::PriceStale`).

## Typed errors

| Contract | Errors |
| --- | --- |
| Proxy | `AlreadyInitialized`, `NotInitialized`, `NotAdmin`, `InvalidWasmHash` |
| Oracle | `AlreadyInitialized`, `NotInitialized`, `NotAdmin`, `PriceNotFound`, `PriceStale`, `SourceNotWhitelisted`, `SourceNotFound`, `SourceDeactivated`, `InvalidPrice`, `PriceDeviationTooHigh`, `InvalidNumSources` |

`NotAdmin` is reserved: admin checks rely on `require_auth`, which rejects an
unauthorized caller at the host level.
