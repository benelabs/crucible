This PR resolves issue #788 and issue #786 for `crucible`.

### Summary of Changes

- **Issue #788 ([Enhancement] Add cargo feature flag 'full')**:
  - Added `full = ["snapshots", "derive"]` meta-feature flag in `contracts/crucible/Cargo.toml`.
  - Added `test-full` CI job matrix entry in `.github/workflows/ci.yml` running `cargo test --workspace --exclude backend --features full`.
  - Updated `README.md` Crate Features table and example to include the `full` feature flag.

- **Issue #786 ([Enhancement] Add env.token(symbol) accessor)**:
  - Added `token_configs` to `MockEnvBuilder` and implemented `.with_token(symbol, decimals)`.
  - Stored registered tokens in `MockEnv` token registry HashMap during `.build()`.
  - Added `env.token(symbol)` and `env.token_opt(symbol)` accessors to `MockEnv` with clear panic formatting when a token is not found.
  - Preserved standalone `MockToken::new()` constructor usage.
  - Added comprehensive unit tests in `contracts/crucible/src/env.rs`.

Closes #788
Closes #786
