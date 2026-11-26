# Crucible Backend

Axum API, PostgreSQL persistence, Redis caching, and background workers for the
Crucible testing platform. This README is the single canonical reference for the
backend; production-readiness evidence is tracked separately in
[ACCEPTANCE_CRITERIA.md](ACCEPTANCE_CRITERIA.md).

[CI workflow](https://github.com/benelabs/crucible/actions/workflows/backend-ci.yml)

## Status

The backend is under active verification. No production-readiness claim is made
until a passing [backend CI run](https://github.com/benelabs/crucible/actions/workflows/backend-ci.yml)
is linked in [ACCEPTANCE_CRITERIA.md](ACCEPTANCE_CRITERIA.md). Claims in this
README describe implemented code, not measured production behaviour.

## Quick start

Prerequisites: Rust 1.91+, Docker with Compose, and `sqlx-cli` for migrations.

```bash
docker compose -f docker-compose.yml up -d
sqlx migrate run --source migrations
cargo run -p backend
```

Health checks:

```bash
curl http://localhost:8080/health
curl http://localhost:8080/health/ready
curl http://localhost:8080/api/status
```

`GET /health/ready` reports database and cache health plus the build version.

Local development without Docker requires PostgreSQL and Redis on the URLs
configured below. The application does not provision services itself.

## Configuration

All settings come from environment variables. `.env` is supported for local
development; no secrets are committed.

| Variable | Purpose | Default |
|---|---|---|
| `DATABASE_URL` | PostgreSQL connection string | required |
| `TEST_DATABASE_URL` | Database used by integration tests | `DATABASE_URL` |
| `REDIS_URL` | Redis connection string | required |
| `TEST_REDIS_URL` | Redis database used by integration tests | `REDIS_URL/1` |
| `BIND_ADDR` | HTTP listen address | `0.0.0.0:8080` |
| `RUST_LOG` | Tracing filter | `info` |
| `OTEL_EXPORTER_OTLP_ENDPOINT` | OTLP collector endpoint | unset |
| `CORS_ALLOWED_ORIGINS` | Comma-separated browser origins | none |
| `RATE_LIMIT_PER_MINUTE` | Request limit per client | unset |
| `JWT_SECRET` | Signing secret for auth tokens | required in production |
| `BACKUP_SCHEDULE` | Cron expression for database backups | unset |
| `BACKUP_DESTINATION` | Backup output location | unset |
| `APP_SANDBOX__MAX_WASM_BYTES` | Maximum decoded WASM size accepted by `/api/v1/sandbox/execute` | `2097152` |
| `APP_SANDBOX__MAX_ARGS` | Maximum number of XDR arguments per invocation | `32` |
| `APP_SANDBOX__MAX_ARG_XDR_BYTES` | Maximum decoded size of one XDR argument | `65536` |
| `APP_SANDBOX__MAX_CPU_INSTRUCTIONS` | CPU instruction/fuel ceiling; also bounds per-request overrides | `25000000` |
| `APP_SANDBOX__MAX_MEMORY_BYTES` | Memory budget; also bounds initial WASM memory declarations | `67108864` |
| `APP_SANDBOX__TIMEOUT_MS` | Wall-clock timeout; also bounds per-request overrides | `2000` |
| `APP_SANDBOX__MAX_CONCURRENT_EXECUTIONS` | Concurrent sandbox execution permits | `32` |
| `APP_SANDBOX__MAX_WASM_IMPORTS` | Maximum imports a WASM module may declare (rejects import storms) | `128` |
| `APP_SANDBOX__MAX_WASM_TABLE_ENTRIES` | Maximum initial entries in a single WASM table | `100000` |

CORS is restrictive in production and development: only explicitly configured
origins are allowed. Do not use wildcard origins outside local tests.

Sandbox limits are validated during `AppConfig::load`; zero or inverted values
fail startup. Every submitted WASM module is fully validated (including function
bodies) and its imports, tables, and initial memory are checked against these
limits before a runtime worker is spawned. The enforced limits and per-request
budget fields are published as `SandboxLimits` and `SandboxBudget` in the OpenAPI
docs at `/swagger-ui`.

## API endpoints

| Method | Path | Purpose |
|---|---|---|
| `GET` | `/health` | Liveness probe |
| `GET` | `/health/ready` | Database, cache, and version readiness |
| `GET` | `/api/status` | Service status and feature flags |
| `GET` | `/api/v1/rules` | List rules |
| `POST` | `/api/v1/rules` | Create a rule |
| `POST` | `/api/v1/logs/ingest` | Ingest logs with idempotency protection |
| `GET` | `/api/v1/metrics` | Aggregated metrics |
| `GET` | `/api/v1/audit/reports` | Security audit reports |
| `GET` | `/api/v1/dashboard/metrics` | Dashboard build and contract metrics |
| `GET` | `/api/v1/dashboard/contracts/{contract_id}/stats` | Per-contract dashboard stats |
| `GET` | `/api/v1/analytics/build-errors` | Build error analytics |
| `GET` | `/api/v1/analytics/build-metrics` | Build system metrics |
| `GET` | `/api/v1/cache/analytics` | Cache hit/miss analytics |

Request handlers use `AppError` for HTTP status mapping. Query parameters and
JSON bodies are validated before reaching services. Dashboard queries use
runtime-checked SQLx constructors; the SQLx strategy is enforced by
[`scripts/check-sqlx-strategy.sh`](scripts/check-sqlx-strategy.sh).

## Architecture

```
src/
  api/          handlers, middleware, errors, JWT and revocation
  config/       typed configuration
  db/           connection and transaction helpers
  services/     business logic, cache, dedup, metrics, backups
  workers/      scheduler, job history, distributed locking
  telemetry.rs  tracing and metrics setup
  router.rs     route registration
  main.rs       process entry point
migrations/     SQLx migrations
tests/          integration test binaries
benches/        Criterion benchmarks
```

Services depend on `PgPool` and a Redis `ConnectionManager` supplied through
application state. Route modules declare their own state extractors and return
`AppError`-compatible results.

## Features

### Caching and idempotency

Redis provides short-lived caches for dashboard metrics and API responses.
Idempotent log ingestion uses a Redis-backed key with a TTL so replays do not
create duplicate rows. Request deduplication is implemented with PostgreSQL and
Redis coordination in `src/services/dedup.rs`.

### Observability

Tracing uses `#[instrument]` on handlers and services, with structured fields
for request IDs, contract IDs, and error contexts. Set `RUST_LOG` for local
filtering. When `OTEL_EXPORTER_OTLP_ENDPOINT` is configured, spans are exported
over OTLP and can be viewed in Jaeger or any OTLP-compatible backend.

Configuration hot reload is available through the admin endpoint; it refreshes
feature flags and log filters without restarting the process.

### Background work

The scheduler uses a Tokio interval loop with cron expressions. Jobs may declare
a timeout, retry policy, and Redis-backed distributed lock. Job outcomes are
written to `job_runs`; panics are recorded as `JobRunStatus::Failed` rather than
terminating the worker. `SchedulerHandle::shutdown` waits for in-progress jobs.

Built-in jobs include cache warming, backup execution, build-metric export,
critical error alerting, and cache analytics refresh.

New jobs implement `JobHandler` and register a `JobDefinition` with the
scheduler. Use a unique job name so the distributed lock is stable.

### Retry policy

Transient failures use exponential backoff with jitter. The policy caps delay
and attempts, logs each retry, and preserves the original error after the final
attempt.

### Permissions and RBAC

Roles map to permission sets checked by middleware. Default roles are reader,
writer, and admin; admin grants all permissions. Permission lookups are cached
in Redis and invalidated when roles change. Database tables and migration
history are the authority for role assignments.

### Backup service

`BACKUP_SCHEDULE` and `BACKUP_DESTINATION` configure scheduled logical backups.
The worker records each run's outcome and exposes it through job history.
Restoration is an operator procedure and is not automated.

### Build metrics and cache analytics

`BuildMetricsService` records compilation time, dependency count, cache hit
rate, CPU and memory usage. `CacheMetricsService` records hit/miss counts and
estimated savings. Both expose aggregate queries used by the dashboard and
analytics endpoints.

## Testing

Start PostgreSQL and Redis, apply migrations, then run:

```bash
cargo fmt -p backend -- --check
cargo clippy -p backend --all-targets -- -D warnings
SQLX_OFFLINE=true cargo check -p backend --all-targets
cargo check -p backend --all-targets
cargo test -p backend --all-features
```

Integration tests use `TEST_DATABASE_URL` and `TEST_REDIS_URL` and create the
tables they need. They previously used `#[ignore]`; those attributes were
removed so the standard test command exercises Redis and PostgreSQL paths.
Build-metric tests share one table and serialize through a process-local lock.
The CI workflow regression tests live in `tests/ci_workflow_tests.rs`.

## Production deployment

Before deploying, complete and attach evidence for each item:

1. Link a passing [backend CI run](https://github.com/benelabs/crucible/actions/workflows/backend-ci.yml).
2. Apply all migrations to the target database.
3. Configure Redis, `JWT_SECRET`, CORS origins, and OTLP export.
4. Run the full test suite and establish benchmark baselines.
5. Configure monitoring, alerting, and log aggregation.
6. Review security settings and run the SAST workflow.
7. Load test dashboard endpoints.
8. Document the operator runbook and restoration procedure.

## License

MIT. See the [repository root](../README.md) for details.
