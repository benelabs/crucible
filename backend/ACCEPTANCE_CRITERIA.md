# Backend Acceptance Criteria

This document records what has been verified for the dashboard feature, what is
implemented but not yet verified by a passing CI run, and what remains
outstanding. It replaces the earlier blanket "production ready" status: a claim
is shown as verified only when it has linked evidence.

## Verification status

| Area | Status | Evidence | Outstanding |
|---|---|---|---|
| Handler implementation | Verified in source | [dashboard handler](src/api/handlers/dashboard.rs), [routes](src/router.rs), [state wiring](src/main.rs) | Runtime behaviour needs a linked passing CI run |
| Endpoints `/api/v1/dashboard/metrics` and `/api/v1/dashboard/contracts/{contract_id}/stats` | Verified in source | [handler](src/api/handlers/dashboard.rs), [integration tests](tests/dashboard_tests.rs) | Response latency targets are not benchmarked in CI |
| Database schema and indexes | Verified in source | [migration](migrations/20260428000000_dashboard.sql) | Migration application must be checked in the deployment environment |
| Redis cache with TTLs | Verified in source | [handler](src/api/handlers/dashboard.rs), [integration tests](tests/dashboard_tests.rs) | Cache hit-rate target has no linked measurement |
| SQLx queries | Verified in source | Runtime-checked `sqlx::query` constructors in [handler](src/api/handlers/dashboard.rs) | Compile-time `sqlx::query!` verification is not used; see [SQLx strategy script](scripts/check-sqlx-strategy.sh) |
| Axum, `AppError`, tracing | Verified in source | [handler](src/api/handlers/dashboard.rs), [error types](src/api/errors.rs) | OpenAPI annotations are not part of this feature |
| Rustdoc and README | Verified in source | [handler docs](src/api/handlers/dashboard.rs), [README](README.md) | No separate `DASHBOARD_IMPLEMENTATION.md` or `DASHBOARD_QUICKSTART.md` exists; the README is the canonical reference |
| Unit and integration tests | Present in source | [unit tests](src/api/handlers/dashboard.rs), [integration tests](tests/dashboard_tests.rs), [benchmark](benches/dashboard_bench.rs) | A passing CI run link is required before calling them verified |
| Production unwraps | Not a backend-wide claim | Dashboard handler's production path has no `unwrap`; [error helpers](src/api/errors.rs) and [JWT helpers](src/api/handlers/auth/jwt.rs) contain unwraps outside the dashboard path | Audit and remove remaining unwraps before making a backend-wide claim |
| No breaking changes | Verified in source | [route registration](src/router.rs) | Existing route behaviour still needs CI evidence |

A "passing CI run" means a run of [backend CI](../.github/workflows/backend-ci.yml)
with a URL from the repository's Actions history. No such URL is recorded here,
so no runtime or production-readiness claim is made.

## Verification commands

The backend CI workflow provisions PostgreSQL and Redis, runs migrations, then
executes these gates:

```bash
./backend/scripts/check-sqlx-strategy.sh
cargo fmt -p backend -- --check
cargo clippy -p backend --all-targets -- -D warnings
SQLX_OFFLINE=true cargo check -p backend --all-targets
cargo check -p backend --all-targets
cargo test -p backend --all-features
sqlx migrate run --source backend/migrations
```

The full backend test suite includes the former `#[ignore]` integration tests;
they now run with the standard `cargo test -p backend --all-features` command.
The empty scheduler test stubs were deleted because they contained no
assertions. The [CI workflow regression tests](tests/ci_workflow_tests.rs) guard
the commands above.

## Production readiness

No backend-wide production-readiness claim is made. Before deployment, work
through the unchecked checklist in [README.md](README.md#production-deployment)
and attach a passing [backend CI](../.github/workflows/backend-ci.yml) run URL
to this document.

## Success metrics (targets only)

These are monitoring targets, not verified results:

- Response time: < 50 ms uncached, < 5 ms cached
- Cache hit rate: > 80%
- Error rate: < 0.1%
- Database query time, Redis memory usage, and API usage patterns: tracked per environment
