# Contributing to Crucible

## Prerequisites

- **Rust** — Install via [rustup](https://rustup.rs). The workspace uses the stable toolchain.
- **Stellar / Soroban** — The crucible crate depends on `soroban-sdk`. Ensure the `wasm32-unknown-unknown` target is installed: `rustup target add wasm32-unknown-unknown`
- **Backend** — The `backend` crate uses `sqlx` with PostgreSQL. Set `DATABASE_URL` in `backend/.env` before running migrations.
- **Node.js (optional)** — The frontend uses `npm` for dependency management and tests.

## Getting Started
git clone https://github.com/benelabs/crucible.git
cd crucible
cargo build --workspace
cargo test --workspace --all-features

text

## Quality Gates

Run these locally before pushing. CI runs them on every PR:
cargo fmt --all --check
cargo clippy --workspace -- -D warnings
cargo test --workspace --all-features
cargo build --package crucible --target wasm32-unknown-unknown

text

## Workspace Structure

| Crate | Description |
|-------|-------------|
| crucible | Core Soroban test framework with cost tracking, fixtures, and snapshots |
| crucible-macros | Derive macros for fixture attribute |
| backend | Axum-based API server with PostgreSQL |
| frontend | Vite + React dashboard |
| examples | Reference Soroban contracts using the framework |

## Common Commands
cargo test -p crucible --all-features
cargo test -p crucible-macros
cargo test -p backend
cd frontend && npm test
cargo doc --workspace --no-deps --all-features --open

text

## Cost Snapshots

Cost snapshots capture instruction and memory metrics for contract invocations and live in `test_snapshots/cost/` next to the crate under test.

**Normal test runs** — if a snapshot file is missing, the test fails immediately with a message pointing to the missing path. No files are written. This keeps CI and contributor working trees clean.

**Creating or updating snapshots** — set `CRUCIBLE_UPDATE_SNAPSHOTS=1` before running tests:

```sh
CRUCIBLE_UPDATE_SNAPSHOTS=1 cargo test -p crucible --features snapshots
```

This writes (or overwrites) every snapshot that is exercised during the run. Review the diff with `git diff` and commit the updated files alongside your code change.

**Accepting a regression** — if an intentional cost increase exceeds the 5 % default tolerance, re-run with `CRUCIBLE_UPDATE_SNAPSHOTS=1` to record the new baseline, then commit both the code and the updated snapshot.

## Pull Request Process

1. Fork and create a feature branch
2. Make changes, add tests
3. Run quality gates locally
4. Update docs if needed
5. Open PR referencing the issue

## Need Help?

Open an issue on the repository.
