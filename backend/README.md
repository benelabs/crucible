# Crucible Backend

High-performance Rust backend for the Crucible smart contract testing platform, providing performance profiling, mock service layers, specialized serialization utilities, and robust background monitoring.

## 🚀 Tech Stack
- **Web Framework**: Axum (async Rust)
- **Runtime**: Tokio
- **Database**: PostgreSQL (via SQLx 0.8)
- **Caching & Jobs**: Redis (via Apalis)
- **Observability**: OpenTelemetry + Tracing
- **API Documentation**: Utoipa (Swagger UI)
## Technical Stack
- **Axum**: High-performance web framework.
- **SQLx**: Async PostgreSQL driver with compile-time checked queries.
- **Redis**: Caching and threshold tracking.
- **Upload Validation**: Safe file upload validation with size, name, and MIME checks.
- **Tracing**: Observability and structured logging.
- **Error Handling**: Structured `AppError` responses for HTTP clients.

## API Endpoints

### Health & Observability
- `GET /health/live` - Liveness probe for process health.
- `GET /health/ready` - Readiness probe for PostgreSQL + Redis connectivity.
- `GET /metrics` - Prometheus metrics exposition endpoint.

### Rules Management
- `GET /api/alerts/rules` - List all alerting rules.
- `POST /api/alerts/rules` - Create a new alerting rule.
- `GET /api/alerts/rules/:id` - Get details of a specific rule.

### Log Ingestion
- `POST /api/alerts/ingest` - Ingest a log entry for pattern matching.

### Security Audit Report Viewer
- `GET /api/v1/audit/reports` — List recent audit events. Supports optional `event_type` and `limit` query parameters.
- `GET /api/v1/audit/reports/:id` — Fetch details for a single audit event.
- `POST /api/v1/audit/log` — Log a security audit event for later review.

### Build Error Analytics Dashboard API
- `GET /api/v1/errors/dashboard/build-errors` — Returns build error analytics (total errors, error types, recent errors)
    - **Response:**
      ```json
      {
        "total_errors": 42,
        "error_types": [["TypeA", 20], ["TypeB", 22]],
        "recent_errors": [
          {"id": 1, "error_type": "TypeA", "message": "...", "occurred_at": "2024-05-28T12:34:56"}
        ]
      }
      ```
    - **Description:**
      Returns analytics for build errors, including total count, breakdown by type, and recent errors. Uses Redis for caching and SQLx for DB access. See `src/api/handlers/errors.rs` for details.

## Database Schema
```sql
CREATE TABLE log_alert_rules (
    id UUID PRIMARY KEY DEFAULT gen_random_uuid(),
    name TEXT NOT NULL,
    pattern TEXT NOT NULL,
    threshold INT NOT NULL DEFAULT 1,
    interval_seconds INT NOT NULL DEFAULT 60,
    is_enabled BOOLEAN NOT NULL DEFAULT true
);

CREATE TABLE log_alerts (
    id UUID PRIMARY KEY DEFAULT gen_random_uuid(),
    rule_id UUID NOT NULL REFERENCES log_alert_rules(id),
    message TEXT NOT NULL,
    triggered_at TIMESTAMPTZ NOT NULL DEFAULT NOW()
);
```
> Production-ready API server for the Crucible smart contract testing platform, built with Rust, Axum, PostgreSQL, and Redis.

---

## 🏗️ Architecture

```text
┌──────────────┐     ┌──────────────────────┐     ┌────────────────┐
│   Clients    │────▶│   Axum HTTP Server   │────▶│  PostgreSQL 16 │
│  (port 8080) │     │                      │     │  (port 5432)   │
└──────────────┘     │  Middleware Stack:   │     └────────────────┘
                     │  ├─ CORS             │
                     │  ├─ Tracing          │     ┌────────────────┐
                     │  ├─ Compression      │────▶│   Redis 7      │
                     │  └─ Request ID       │     │  (port 6379)   │
                     └──────────────────────┘     └────────────────┘
```

---

## ⚡ Quick Start

### Prerequisites
- [Docker](https://docs.docker.com/get-docker/) ≥ 24.0
- [Docker Compose](https://docs.docker.com/compose/install/) ≥ 2.20
- [Rust](https://rustup.rs/) ≥ 1.78 (for local development)

### Starting Services
```bash
cd backend
cp .env.example .env

# Start all core services (app, postgres, redis)
docker compose up -d

# Check service health
curl http://localhost:8080/health
docker compose ps

# Test the health endpoints
curl http://localhost:8080/health/live
curl http://localhost:8080/health/ready

# Expected readiness response:
# {"status":"healthy","database":"healthy","cache":"healthy","version":"0.1.0"}

# Test the metrics endpoint
curl http://localhost:8080/metrics

# Test the API status endpoint
curl http://localhost:8080/api/v1/status
```

### Applying Database Migrations
SQLx migrations live in `backend/migrations/`. Apply them after the database is running:
```bash
export DATABASE_URL=postgres://crucible:crucible_secret@localhost:5432/crucible_db
sqlx migrate run --source backend/migrations
# or from the repo root:
make db-migrate
```

Verify migration state at any time with:
```bash
sqlx migrate info --source backend/migrations
```

### Local Development (without Docker)
Run Postgres and Redis in Docker, but the Rust app natively:
```bash
docker compose up -d postgres redis
export DATABASE_URL=postgres://crucible:crucible_secret@localhost:5432/crucible_db
export REDIS_URL=redis://:crucible_redis_secret@localhost:6379/0
# Apply migrations before starting the app
sqlx migrate run --source backend/migrations
cargo run
```

---

## ⚙️ Configuration

This application uses a layered configuration system. Base values and environment-specific tunings are compiled directly into the binary, ensuring safe fallbacks. Infrastructure secrets and dynamic overrides are provided securely at runtime via environment variables.

### Environment Variables

| Variable | Default | Required in Prod? | Description |
## Project Structure

```
backend/
├── docker-compose.yml      # Docker Compose service orchestration
├── Dockerfile              # Multi-stage build for the Rust binary
├── Cargo.toml              # Rust dependencies and build configuration
├── .env.example            # Environment variable template
├── .dockerignore           # Files excluded from Docker build context
├── README.md               # This file
├── migrations/             # SQLx database migrations
│   └── .keep
├── scripts/
│   └── init-db.sql         # Database initialization (schema + seeds)
└── src/
    ├── main.rs             # Application entry point, router, health checks
    └── error.rs            # Custom error types with HTTP status mapping
```

## Environment Variables

| Variable                  | Default                     | Description                              |
|---------------------------|-----------------------------|------------------------------------------|
| `APP_ENV`                 | `development`               | Environment (`development`/`production`) |
| `APP_PORT`                | `8080`                      | HTTP server port                         |
| `RUST_LOG`                | `crucible_backend=debug`    | Log level filter                         |
| `DATABASE_URL`            | *(composed from parts)*     | Full PostgreSQL connection string        |
| `POSTGRES_USER`           | `crucible`                  | PostgreSQL username                      |
| `POSTGRES_PASSWORD`       | `crucible_secret`           | PostgreSQL password                      |
| `POSTGRES_DB`             | `crucible_db`               | PostgreSQL database name                 |
| `DATABASE_MAX_CONNECTIONS`| `10`                        | Max pool connections                     |
| `DATABASE_MIN_CONNECTIONS`| `2`                         | Min pool connections                     |
| `REDIS_URL`               | *(composed from parts)*     | Full Redis connection string             |
| `REDIS_PASSWORD`          | `crucible_redis_secret`     | Redis authentication password            |
| `REDIS_POOL_SIZE`         | `5`                         | Redis connection pool size               |
| `JWT_SECRET`              | *(dev default)*             | JWT signing secret                       |
| `APP_CORS__ALLOWED_ORIGINS__0` | *(none)*               | Allowed CORS origin (use `__0`, `__1`, etc. for multiple) |

### CORS Behavior per Environment

CORS configuration is driven by `cors.allowed_origins` in the config system. The behavior changes with the environment:

| Environment   | Default `allowed_origins` | Wildcard `*` allowed? | Behavior                                      |
|---------------|---------------------------|------------------------|-----------------------------------------------|
| Development   | `["*"]`                   | Yes                    | Permissive CORS — any origin, method, header  |
| Staging       | `[]`                      | **No** (rejected)      | Must set explicit origins or startup fails    |
| Production    | `[]`                      | **No** (rejected)      | Must set explicit origins or startup fails    |

**Setting origins via environment variables:**

Use indexed keys with the `APP__` prefix. The `config` crate maps `APP_CORS__ALLOWED_ORIGINS__N` to `cors.allowed_origins[N]`:

```
APP_CORS__ALLOWED_ORIGINS__0=https://app.example.com
APP_CORS__ALLOWED_ORIGINS__1=https://admin.example.com
```

**Startup validation:**
In Production or Staging, the application refuses to start if `allowed_origins` is empty or contains the wildcard `"*"`. The error message directs you to set explicit origins.

**SEP-1 exception:** The `/.well-known/stellar.toml` endpoint always returns `Access-Control-Allow-Origin: *` — this is required by the Stellar protocol for wallet discovery and is independent of the application CORS configuration.

## Docker Compose Features

### Health Checks

All services include Docker health checks:

- **PostgreSQL**: `pg_isready` command verifying database connectivity
- **Redis**: `redis-cli PING` command verifying cache availability
- **App**: HTTP `GET /health` checking both downstream dependencies

The `app` service uses `depends_on` with `condition: service_healthy` to ensure infrastructure is ready before starting.

### Resource Limits

Each service has memory and CPU limits configured via `deploy.resources`:

| Service    | Memory Limit | CPU Limit | Memory Reserve | CPU Reserve |
|------------|-------------|-----------|----------------|-------------|
| `app`      | 512 MB      | 2.0       | 128 MB         | 0.5         |
| `postgres` | 512 MB      | 1.0       | 128 MB         | 0.25        |
| `redis`    | 256 MB      | 0.5       | 64 MB          | 0.1         |

### Persistent Volumes

Named volumes ensure data survives container restarts:

- `crucible-postgres-data` — PostgreSQL data directory
- `crucible-redis-data` — Redis append-only file and snapshots
- `crucible-pgadmin-data` — pgAdmin configuration

### Networking

All services communicate over the `crucible-network` bridge network with a dedicated subnet (`172.28.0.0/16`), isolating traffic from other Docker workloads.

### Logging

JSON file logging with rotation:
- App: 50 MB max file, 5 files retained
- Infrastructure: 10 MB max file, 3 files retained

## Database Schema

The init script (`scripts/init-db.sql`) creates:

| Table         | Purpose                                      |
|---------------|----------------------------------------------|
| `contracts`   | Deployed smart contract metadata             |
| `test_runs`   | Test execution results per contract          |
| `test_cases`  | Individual test results within a run         |
| `jobs`        | Background job queue tracking                |

Extensions enabled: `uuid-ossp`, `pgcrypto`, `citext`

## API Endpoints

| Method   | Path                                              | Description                          |
|----------|---------------------------------------------------|--------------------------------------|
| `GET`    | `/health`                                         | Health check (DB + Redis)            |
| `GET`    | `/api/v1/status`                                  | API status and version info          |
| `POST`   | `/api/v1/deployments`                             | Register a new deployment            |
| `GET`    | `/api/v1/deployments/:id`                         | Get a deployment by UUID             |
| `GET`    | `/api/v1/deployments/contract/:contract_id`       | List deployments for a contract      |
| `PATCH`  | `/api/v1/deployments/:id/status`                  | Update deployment health status      |

### Deploy Health

The deploy-health API tracks the lifecycle and health of contract deployments.

#### Register a deployment

```http
POST /api/v1/deployments
Content-Type: application/json

{
  "contract_id": "CAABC123...",
  "version": "1.2.0",
  "metadata": { "network": "testnet" }
}
```

Response `201 Created`:

```json
{
  "id": "550e8400-e29b-41d4-a716-446655440000",
  "contract_id": "CAABC123...",
  "version": "1.2.0",
  "status": "pending",
  "deployed_at": "2026-05-27T22:00:00Z",
  "last_checked_at": null,
  "error_message": null,
  "metadata": { "network": "testnet" },
  "created_at": "2026-05-27T22:00:00Z",
  "updated_at": "2026-05-27T22:00:00Z"
}
```

#### Update deployment status

```http
PATCH /api/v1/deployments/:id/status
Content-Type: application/json

{
  "status": "healthy"
}
```

Valid status values: `pending` | `healthy` | `degraded` | `failed`.

Pass `"error_message"` alongside `"status": "failed"` or `"status": "degraded"` to record a reason.

#### Caching

Single-deployment and contract-list responses are cached in Redis for 30 seconds.
A `PATCH` to update status invalidates the per-deployment cache entry immediately.

## Production Deployment

For production, update the following:

1. **Change all passwords** in `.env` — never use defaults
2. **Set `APP_ENV=production`**
3. **Set `RUST_LOG=crucible_backend=info,tower_http=info`** — reduce log verbosity
4. **Set a strong `JWT_SECRET`** — at least 64 characters
5. **Set explicit `APP_CORS__ALLOWED_ORIGINS__0`** — to your frontend domain(s). Wildcard `*` is rejected in production.
6. **Consider external managed databases** — for PostgreSQL and Redis at scale
7. **Add TLS termination** — via a reverse proxy (nginx, Caddy, or cloud LB)

## License

MIT — see [LICENSE](../LICENSE) for details.
This is the backend component of the Crucible project, providing a JSON schema generator utility.

## Features

- JSON Schema generation using `schemars`
- Async HTTP server with Axum
- PostgreSQL database integration with SQLx
- Redis caching and job queues
- Comprehensive error handling and tracing

## Usage

Add to your `Cargo.toml`:

```toml
[dependencies]
crucible-backend = { path = "../backend" }
```

Then use the JSON schema generator:

```rust
use crucible_backend::utils::json_schema::generate_json_schema;
use serde::{Deserialize, Serialize};
use schemars::JsonSchema;

#[derive(Serialize, Deserialize, JsonSchema)]
struct MyStruct {
    field: String,
}

let schema = generate_json_schema::<MyStruct>();
```

## Running Tests

```bash
cargo test --manifest-path backend/Cargo.toml
```

## Building

```bash
cargo build --manifest-path backend/Cargo.toml
```
This is the backend service layer for the Crucible toolkit, providing performance profiling, mock service layers, specialized serialization utilities, and robust background monitoring.

## Features

### 🚀 Performance Profiling API
High-performance endpoints for monitoring application health and system metrics.
- `/api/v1/profiling/metrics`: Real-time system metrics.
- `/api/v1/profiling/health`: System health status.
- `/api/status`: Unified health, metrics, and active recovery tasks.

### 🔭 OpenTelemetry Tracing (Issue #365)
Production-grade distributed tracing with OTLP exporter and Jaeger integration.
- **Full instrumentation**: HTTP handlers, database queries, Redis operations, background jobs
- **Semantic conventions**: W3C trace context, OpenTelemetry semantic conventions
- **Sampling strategies**: Environment-based sampling (100% dev, 10% staging, 1% prod)
- **Zero overhead**: < 1% p95 latency impact with optimized span creation
- **Jaeger UI**: Visual trace exploration at `http://localhost:16686`

### 🧪 Mock Service Layer
A robust mock layer for testing services in isolation, supporting both database and cache operations.

### 🔢 Custom Serialization
Specialized Serde serializers for high-precision types and Stellar-specific formats.

### 🛠️ Background Services
The backend runs several background workers for system health and data consistency.

## Tech Stack
- **Web Framework**: Axum (async Rust)
- **Runtime**: Tokio
- **Database**: PostgreSQL (via SQLx 0.8)
- **Caching & Jobs**: Redis (via Apalis)
- **Serialization**: Serde
- **Observability**: Tracing + OpenTelemetry (OTLP)
- **API Documentation**: Utoipa (Swagger UI)

## Structure
- `src/api/` — API handlers and routing
- `src/services/` — Business logic and external integrations
- `src/models/` — Data structures and database schemas
- `tests/` — Integration and API tests
- `src/api/` – API handlers and routing
- `src/bin/` – Standalone service binaries
- `src/config/` – Environment configuration and hot-reload
- `src/db/` – Database utilities and seed data
- `src/jobs/` – Background job definitions (Apalis)
- `src/services/` – Business logic and external integrations
- `src/telemetry/` – Observability and logging setup

## Build Request Deduplication

The backend includes a build request deduplication service in `src/services/dedup.rs`.
It fingerprints build submissions with SHA-256, uses Redis `SET NX EX` for fast
in-flight duplicate suppression, persists accepted fingerprints in PostgreSQL,
and enqueues accepted requests into a Redis list for build workers.

The service exposes an Axum router for `POST /build-requests` when mounted by the
application. Duplicate submissions return `duplicate` or `in_progress` decisions
instead of enqueueing repeated build work.

Schema is managed by `migrations/20260528010000_build_request_dedup.sql`. The
service also exposes `BuildDedupService::ensure_schema` for isolated integration
tests that need to provision the table directly.

## Test Fixture Factory

`src/test_utils/factories.rs` provides `FixtureFactory` and `TestFixture` for
creating related users, products, orders, and sessions in tests. Existing
domain-specific factories remain available.
- `src/utils/` – Serialization, validation, XDR helpers
- `src/test_utils/` – Mock traits for unit testing

### API Handlers (`src/api/handlers/`)

| Module | Description |
|---|---|
| `profiling` | System status, metrics, health, and profiling trigger endpoints |
| `dashboard` | Aggregated dashboard data endpoint with Redis caching |
| `stellar` | Stellar SEP-1 `.well-known/stellar.toml` endpoint |

### Services (`src/services/`)

| Module | Description |
|---|---|
| `sys_metrics` | Build system metrics exporter with PostgreSQL persistence and Redis caching (compilation times, dependency counts, cache hit rates) |
| `cache_metrics` | Cache operation analytics with PostgreSQL event storage and Redis-cached summaries |
| `error_recovery` | Tracks retry state for failing tasks with configurable max retries |
| `log_aggregator` | Async MPSC-based log pipeline; persists entries via a background worker |
| `log_alerts` | Threshold-based alerting over the log pipeline with sliding-window evaluation |
| `logger` | Structured logging bootstrap; pretty output in development and JSON logs in staging/production |
| `feature_flags` | Feature flag management backed by PostgreSQL with Redis caching |
| `alerts` | Critical-error notification dispatcher — deduplication, in-memory queue, Redis pub/sub |
| `tracing` | OpenTelemetry tracing initialisation — wires `tracing` spans to an OTLP HTTP exporter |

### Database (`src/db/`)

| Module | Description |
|---|---|
| `seeds` | Idempotent seed data for development and test environments |
| `test_coverage` | Code coverage tracking and caching for CI integration |
| `tracing` | OpenTelemetry tracing service with OTLP exporter |

### Middleware

| Name | Description |
|---|---|
| `logging` | Captures request/response metadata, latency, and status codes; integrated with `tracing` and `log_aggregator` |
| `permissions` | Role-based access control (RBAC) with PostgreSQL storage and Redis caching for permission checks |

### Binaries (`src/bin/`)

| Binary | Description |
|--------|-------------|
| `backup` | Database backup and restore HTTP service + job enqueuer |

### Database (`src/db/`)

| Module | Description |
|---|---|
| `seeds` | Idempotent seed data for development and test environments |

## Tech Stack
- **Web Framework**: Axum (async Rust)
- **Runtime**: Tokio
- **Database**: PostgreSQL (via SQLx 0.8)
- **Caching & Jobs**: Redis (via Apalis)
- **Serialization**: Serde
- **Observability**: OpenTelemetry + Tracing
- **API Documentation**: Utoipa (Swagger UI)

## Workers Module

The backend includes a dedicated workers module for background processing and system monitoring:

### 🌟 Cache Warming System
Pre-loads frequently accessed data into Redis cache to improve performance.
- Automatically warms dashboard metrics and popular build data
- Configurable warm intervals and TTL settings
- Integrated with PostgreSQL database queries

### 🚀 Response Caching Middleware
HTTP response caching middleware that stores API responses in Redis.
- Automatic cache key generation based on request method, URI, and query parameters
- Configurable TTL for cached responses
- Integration with Axum middleware stack

### 📊 Job Progress Tracking
Monitors and reports progress for long-running background jobs.
- Real-time progress tracking with percentage calculation
- Redis-based storage for job state
- Support for completion steps and total steps tracking

### 🩺 Worker Health Monitoring
Tracks and reports health status of background workers.
- Heartbeat monitoring with configurable thresholds
- Automatic health status calculation
- Redis-based health state storage

All workers are designed to be production-ready with comprehensive error handling, tracing integration, and proper resource management.

## API Endpoints

| Method | Path | Description |
|---|---|---|
| `GET` | `/api/status` | System health, metrics, and active recovery tasks |
| `POST` | `/api/profile` | Trigger a profiling collection run |
| `GET` | `/health` | Backup service liveness probe |
| `POST` | `/backups` | Enqueue a new backup job |
| `GET` | `/backups` | List all backup records |
| `GET` | `/backups/:id` | Get a single backup record |
| `POST` | `/backups/:id/restore` | Enqueue a restore job for a backup |
| `POST` | `/api/coverage` | Submit a new code coverage report |
| `GET` | `/api/coverage/:project` | Get latest coverage report for a specific project |
| `GET` | `/` | Base API greeting |
| `GET` | `/.well-known/stellar.toml` | Stellar network metadata (SEP-1) |
| `GET` | `/api/v1/profiling/metrics` | Detailed performance metrics (OpenAPI) |
| `GET` | `/api/v1/profiling/health` | Service health check (OpenAPI) |
| `GET` | `/api/v1/dashboard/metrics` | Dashboard aggregated metrics with Redis caching |
| `GET` | `/api/v1/dashboard/contracts/:contract_id/stats` | Contract-specific statistics |
| `POST` | `/cache-metrics` | Record a cache operation metric |
| `GET` | `/cache-metrics` | List recent cache operation metrics |
| `GET` | `/cache-metrics/summary` | Cache hit/miss, latency, payload, and operation analytics |
| `GET` | `/api/v1/profiling/prometheus` | Prometheus-compatible metrics |
| `GET` | `/api/status` | System health summary and recovery status |
| `POST` | `/api/profile` | Trigger a manual profiling collection run |
| `GET` | `/api/dashboard` | Aggregated dashboard data: metrics, recovery tasks, and active alerts (Redis-cached, 30 s TTL) |
| `GET` | `/swagger-ui` | Interactive API documentation |
| `GET` | `/api/v1/errors/dashboard/build-errors` | Returns build error analytics (total errors, error types, recent errors) |

## Running
## OpenTelemetry Tracing

### Quick Start

1. **Start Jaeger** (includes OTLP collector):
   ```bash
   docker-compose -f docker-compose-jaeger.yml up -d
   ```

2. **Run the backend** with tracing enabled:
   ```bash
   export APP_OBSERVABILITY__TRACING_ENDPOINT=http://localhost:4318/v1/traces
   export APP_ENV=development
   cargo run -p backend
   ```

3. **View traces** in Jaeger UI:
   ```
   http://localhost:16686
   ```

### Architecture

The tracing system instruments the entire request lifecycle:

```
HTTP Request → Axum Handler → Service Method → Database/Redis → Response
     ↓              ↓               ↓                ↓
  http.request  service.method  db.query      db.redis.command
```

### Instrumented Components

#### HTTP Handlers (100% coverage)
- ✅ `GET /api/v1/profiling/metrics` - Metrics collection
- ✅ `GET /api/v1/profiling/health` - Health checks with DB ping
- ✅ `GET /api/v1/profiling/prometheus` - Prometheus metrics export
- ✅ `GET /api/status` - System status aggregation
- ✅ `POST /api/profile` - Profile collection trigger
- ✅ `GET /.well-known/stellar.toml` - Stellar TOML endpoint

#### Service Methods (100% coverage)
- ✅ `MetricsExporter::get_metrics()` - Metrics retrieval
- ✅ `MetricsExporter::update_metrics()` - Metrics update
- ✅ `ErrorManager::get_active_tasks()` - Recovery task listing
- ✅ `ErrorManager::handle_error()` - Error recovery with retry logic
- ✅ `FeatureFlagService::is_enabled()` - Feature flag check (Redis + DB)
- ✅ `FeatureFlagService::set()` - Feature flag update (DB + cache invalidation)
- ✅ `FeatureFlagService::get()` - Feature flag retrieval
- ✅ `FeatureFlagService::list()` - Feature flag listing
- ✅ `FeatureFlagService::delete()` - Feature flag deletion
- ✅ `FeatureFlagService::flush_cache()` - Cache flush

#### Background Jobs (100% coverage)
- ✅ `monitor_transaction()` - Stellar transaction monitoring (Apalis job)

### Semantic Conventions

The tracing implementation follows OpenTelemetry semantic conventions:

#### HTTP Spans
```rust
http.method = "GET"
http.route = "/api/v1/profiling/metrics"
http.status_code = 200
http.flavor = "1.1"
http.scheme = "https"
user.id = "user123"
otel.kind = "server"
```

#### Database Spans (PostgreSQL)
```rust
db.system = "postgres"
db.statement = "SELECT * FROM users WHERE id = $1"  // truncated to 256 chars
db.operation = "SELECT"
db.rows_affected = 1
otel.kind = "client"
```

#### Redis Spans
```rust
db.system = "redis"
db.redis.command = "GET"
db.redis.key = "flag:new_dashboard"
otel.kind = "client"
```

#### Service Spans
```rust
service.name = "FeatureFlagService"
service.method = "is_enabled"
otel.kind = "internal"
```

#### Job Spans
```rust
job.name = "monitor_transaction"
job.id = "550e8400-e29b-41d4-a716-446655440000"
otel.kind = "internal"
```

### Configuration

#### Environment Variables

| Variable | Default | Description |
|---|---|---|
| `APP_OBSERVABILITY__TRACING_ENDPOINT` | `http://localhost:4318/v1/traces` | OTLP HTTP traces endpoint |
| `APP_ENV` | `development` | Environment (development, staging, production) |
| `RUST_LOG` | `info,crucible=debug` | Log level filter |

#### Sampling Strategies

Sampling is automatically configured based on environment:

| Environment | Sampling Rate | Strategy |
|---|---|---|
| `dev` | 100% | AlwaysOn |
| `staging` | 10% | TraceIdRatioBased |
| `production` | 1% | ParentBased + TraceIdRatioBased |

#### Span Limits

To prevent memory issues, spans have the following limits:

- **Max attributes per span**: 128
- **Max events per span**: 128
- **Max links per span**: 128
- **Query truncation**: 256 characters (first line only for multiline queries)

### Jaeger UI Guide

#### Searching Traces

1. **By Service**: Select `crucible-backend` from the service dropdown
2. **By Operation**: Filter by operation name (e.g., `http.request`, `db.query`)
3. **By Tags**: Search by custom tags (e.g., `http.method=GET`, `user.id=user123`)
4. **By Duration**: Find slow requests with min/max duration filters

#### Understanding Traces

A typical trace hierarchy:

```
http.request (GET /api/v1/profiling/health)
├── service.method (MetricsExporter::get_metrics)
├── db.query (SELECT 1)  ← Database health check
└── service.method (ErrorManager::get_active_tasks)
```

#### Key Metrics

- **Trace Duration**: Total request time (p50, p95, p99)
- **Span Count**: Number of operations per request
- **Error Rate**: Percentage of traces with errors
- **Service Dependencies**: Visual service map

### Performance Impact

Benchmarked on a 4-core system with 8GB RAM:

| Metric | Without Tracing | With Tracing | Overhead |
|---|---|---|---|
| `APP_ENV` | `development` | Yes | `development`, `staging`, or `production` |
| `APP_SERVER__PORT` | (from TOML) | No | HTTP server listen port. |
| `APP_SERVER__TLS__CERT_PATH` | None | Yes | Path to the TLS certificate chain. |
| `APP_SERVER__TLS__KEY_PATH` | None | Yes | Path to the TLS private key. **(SENSITIVE)** |
| `APP_DATABASE__URL` | None | Yes | PostgreSQL connection string. **(SENSITIVE)** |
| `APP_REDIS__URL` | None | Yes | Redis connection string. **(SENSITIVE)** |

### Configuration Hot-Reload

The backend supports atomic configuration hot-reloading without restarting the server via `ArcSwap`.

```bash
# Trigger a reload from the HTTP endpoint
curl -X POST http://localhost:8080/api/config/reload
```

---

## 📡 API Endpoints

Crucible uses a typed contract system for all API endpoints to ensure consistency.

| Category | Method | Path | Description |
|---|---|---|---|
| **Health** | `GET` | `/health` | Health check (DB + Redis) |
| **Health** | `GET` | `/api/status` | System health summary and recovery status |
| **Dashboard**| `GET` | `/api/v1/dashboard/metrics`| Dashboard aggregated metrics with Redis caching |
| **Errors** | `GET` | `/api/v1/errors/dashboard/build-errors`| Returns build error analytics |
| **Config** | `GET` | `/api/config` | Retrieve current configuration (sanitized) |
| **Config** | `POST` | `/api/config/reload` | Trigger configuration hot-reload |
| **Docs** | `GET` | `/swagger-ui` | Interactive API documentation |

---

## 🔭 Observability & Tracing

Production-grade distributed tracing with OTLP exporter and Jaeger integration.

1. **Start Jaeger**: `docker-compose -f docker-compose-jaeger.yml up -d`
2. **Run Backend**: `export APP_OBSERVABILITY__TRACING_ENDPOINT=http://localhost:4318/v1/traces; cargo run -p backend`
3. **View Traces**: Open `http://localhost:16686`

Spans from every `#[tracing::instrument]`-annotated function are exported with **< 1% latency overhead**.

---

## 🛠️ Background Services & Features

### 1. Build System Metrics Exporter
Tracks compilation times, dependency counts, cache hit rates, and resource usage. Includes durable storage in PostgreSQL and high-performance caching in Redis.

### 2. Critical Error Alerting
Dispatches notifications when a critical condition fires (e.g., `db_down`). Deduplicates within a configurable cooldown window and publishes to Redis pub/sub.

### 3. Feature Flags
Stored in PostgreSQL and cached in Redis with a 5-minute TTL.

---

## ⏱️ Cron Job Scheduler

The distributed cron job scheduler guarantees single execution per tick across multiple instances, provides exact timeouts with safe cancellation, tracks full execution history in PostgreSQL, and offers graceful shutdown.

### Usage Example

```rust
use backend::services::sys_metrics::{BuildMetricsService, BuildMetric, BuildStatus};
use sqlx::PgPool;
use redis::Client;

let service = BuildMetricsService::new(pool, redis);

// Record a build metric
let metric = BuildMetric {
    id: None,
    project_name: "crucible".to_string(),
    build_id: "build-123".to_string(),
    build_status: BuildStatus::Success,
    compilation_time_ms: 5000,
    dependency_count: 42,
    cache_hit_rate: Some(85.5),
    cpu_usage: Some(75.2),
    memory_usage_mb: Some(1024),
    build_timestamp: Utc::now(),
};
service.record_build(metric).await?;

// Get project metrics with caching
let metrics = service.get_project_metrics("crucible", 10).await?;

// Get aggregated summary
let summary = service.get_project_summary("crucible").await?;
println!("Success rate: {}%", summary.success_rate);
```

### API Reference

#### BuildMetricsService

- `new(db, redis)` - Create a new metrics service
- `record_build(metric)` - Record a build metric (invalidates cache)
- `get_project_metrics(project_name, limit)` - Get metrics for a project (with caching)
- `get_project_summary(project_name)` - Get aggregated statistics
- `get_recent_metrics(limit)` - Get recent builds across all projects
- `delete_project_metrics(project_name)` - Delete all metrics for a project

## Cache Metrics and Analytics

The `cache_metrics` module records cache operations as durable PostgreSQL events and caches aggregate summaries in Redis with a short TTL.

- `CacheMetricsService::record(input)` - Persist a cache operation and invalidate affected summaries
- `CacheMetricsService::recent(query)` - Read bounded recent cache events
- `CacheMetricsService::summary(query)` - Return hit/miss counts, hit rate, latency, payload totals, and operation breakdowns
- `CacheMetricsService::ensure_schema()` - Create the table and indexes for tests or embedded deployments
- `router(service)` - Build Axum routes for recording and reading cache analytics

## Backup Service Configuration

All configuration is via environment variables.

| Variable | Required | Default | Description |
|----------|----------|---------|-------------|
| `DATABASE_URL` | Yes | — | PostgreSQL connection string |
| `REDIS_URL` | No | `redis://127.0.0.1/` | Redis connection string |
| `BACKUP_QUEUE` | No | `backup_jobs` | Redis list key for backup jobs |
| `RESTORE_QUEUE` | No | `restore_jobs` | Redis list key for restore jobs |
| `BIND_ADDR` | No | `0.0.0.0:8080` | HTTP server bind address |
| `BACKUP_DIR` | No | `/var/backups/crucible` | Directory for `pg_dump` output files |
## Configuration Hot-Reload

`ConfigWatcher` holds the live `AppConfig` behind an `Arc<RwLock<_>>`. Any part of the application that holds a `ConfigHandle` sees new values immediately after a reload — no restart required.

```rust
use std::sync::Arc;

let mut scheduler = Scheduler::new(pool.clone(), redis_client.clone());

// Register a custom job
scheduler.register(JobDefinition {
    name: "health_check".to_string(),
    cron_expr: "0 * * * * * *".to_string(), // Every minute
    handler: Arc::new(HealthCheckJob),
    timeout_secs: 10,
    max_retries: 0,
})?;

let scheduler_handle = scheduler.start()?;

// On application exit:
// scheduler_handle.shutdown().await?;
```

### Implementing a New Job

Implement the `JobHandler` trait for any struct.

```rust
use async_trait::async_trait;
use crucible_backend::workers::scheduler::{JobHandler, JobContext};
use crucible_backend::workers::error::JobError;

pub struct MyJob;

#[async_trait]
impl JobHandler for MyJob {
    async fn run(&self, ctx: JobContext) -> Result<(), JobError> {
        // Business logic here
        // The ctx contains the db pool, redis client, and run metadata
        Ok(())
    }
}
```

### Built-in Jobs

| Job | Default Cron | Purpose |
|---|---|---|
| `HealthCheckJob` | `0 * * * * * *` (Every minute) | Verifies Database and Redis connectivity. |
| `CleanupJob` | `0 0 0 * * * *` (Daily at midnight) | Prunes `job_runs` history older than the configured retention period. |

### Distributed Locking Behavior
Before running a job, the scheduler issues `SET {job_name}:lock "1" NX PX {timeout_secs * 1000 + 5000}`. If the lock is already held by another running instance, the current instance skips the tick. The TTL ensures that if the node crashes hard without cleaning up the lock, it expires automatically shortly after the job would have timed out anyway.

### Environment Variables
- `SCHEDULER_ENABLED`: (bool) Set to `false` to prevent the scheduler from starting on this node.
- `JOB_HISTORY_RETAIN_DAYS`: (integer) Days to retain job history before `CleanupJob` prunes it.

---

## 🧪 Testing

This project utilizes highly isolated, in-process integration testing leveraging Axum's `oneshot` capability.

```bash
# Unit tests
cargo test -p backend --lib

# Integration tests (requires PostgreSQL and Redis)
cargo test -p backend --test integration_tests
```

### Test Database Isolation Strategy
We utilize **Isolated Schemas**. For every `#[tokio::test]`, the `TestContext` dynamically creates a completely isolated PostgreSQL schema (e.g. `test_a1b2c3d4...`) and maps the SQLx connection pool strictly to that `search_path`.
- **Parallelization**: Tests run fully concurrently.
- **Safety**: The schema is dropped entirely on `Drop`.

### Integration Test Framework Example
You can utilize the `ApiTestClient` located in `src/test_utils/client.rs`.

```rust
use crate::test_utils::{setup, client::ApiTestClient};
use tower::ServiceExt;
use hyper::{Request, StatusCode};

#[tokio::test]
async fn test_create_resource() {
    let ctx = setup().await;
    let client = ApiTestClient::new(ctx.app);
    
    let response = client.post("/api/resources")
        .bearer("mock-token")
        .json(&serde_json::json!({ "name": "Crucible" }))
        .send()
        .await;

    response.assert_status(StatusCode::CREATED);
}
```

---

## 📄 License
MIT — see [LICENSE](../LICENSE) for details.
## Structure
- `src/api/` – API handlers and routing
- `src/config/` – Environment configuration
- `src/db/` – Database utilities and seed data
- `src/jobs/` – Background job definitions (Apalis)
- `src/services/` – Business logic and external integrations
- `src/telemetry/` – Observability and logging setup

## Configuration

This application uses a layered configuration system. Base values and environment-specific tunings are compiled directly into the binary, ensuring safe fallbacks. Infrastructure secrets and dynamic overrides are provided securely at runtime via environment variables.

### Setting the Environment
The application environment is controlled by the `APP_ENV` environment variable.
Valid values: `development` (or `dev`), `staging` (or `stg`), `production` (or `prod`).
If `APP_ENV` is omitted, the application safely defaults to `development` and emits a warning log.

### Environment Variables

| Variable | Type | Default | Required in Prod? | Description |
|---|---|---|---|---|
| `APP_ENV` | String | `development` | Yes | Defines the execution environment. |
| `APP_SERVER__PORT` | Integer | (from TOML) | No | HTTP server listen port. |
| `APP_SERVER__TLS__CERT_PATH` | String | None | Yes | Path to the TLS certificate chain. |
| `APP_SERVER__TLS__KEY_PATH` | String | None | Yes | Path to the TLS private key. **(SENSITIVE)** |
| `APP_DATABASE__URL` | String | None | Yes | PostgreSQL connection string. **(SENSITIVE)** |
| `APP_DATABASE__MAX_CONNECTIONS`| Integer | (from TOML) | No | Maximum size of the database connection pool. |
| `APP_REDIS__URL` | String | None | Yes | Redis connection string. **(SENSITIVE)** |
| `APP_REDIS__JOB_QUEUE_URL` | String | Falls back to URL | No | Separate Redis instance for the job queue. **(SENSITIVE)** |
| `APP_OBSERVABILITY__LOG_LEVEL` | String | (from TOML) | No | Logging verbosity (e.g., `info`, `warn`, `debug`). |
| `APP_OBSERVABILITY__TRACING_ENDPOINT` | String | `http://localhost:4318/v1/traces` | No | OTLP HTTP traces endpoint for Jaeger or an OpenTelemetry Collector. |

### How to Add a New Configuration Field

1. Add the field to the relevant struct in `backend/src/config/` (e.g., `ServerConfig` in `server.rs`).
2. Add a sensible default value for it in `backend/src/config/defaults/default.toml`.
3. If the value needs to differ by environment, override it in `development.toml`, `staging.toml`, or `production.toml`.
4. If the field is an infrastructure secret, *do not* put it in the TOML files. Pass it at runtime via the corresponding environment variable (e.g., `APP_NEW_MODULE__SECRET_KEY`).
5. (Optional) Add validation logic to `AppConfig::validate` in `backend/src/config/mod.rs` if the field has hard constraints.

### 🔒 Security Note
Certain configuration fields are sensitive and **must never be logged or committed to version control**. 
These include:
- `DatabaseConfig::url`
- `RedisConfig::url`
- `RedisConfig::job_queue_url`
- `TlsConfig::key_path`

These fields implement a custom `Debug` trait that automatically replaces their contents with `"[REDACTED]"` to prevent accidental leakage in application logs or panic traces. Additionally, they are marked to skip serialization.

## Testing

This project utilizes highly isolated, in-process integration testing leveraging Axum's `oneshot` capability.

### Running Tests
Execute the test suite natively using Cargo:
```bash
cargo test
```

### Environment Requirements
Integration tests require a running PostgreSQL instance and Redis instance.
- `TEST_DATABASE_URL`: (Default: `postgres://postgres:password@localhost/crucible_test`)
- `TEST_REDIS_URL`: (Default: `redis://127.0.0.1/1`)

### Test Database Isolation Strategy
We utilize **Isolated Schemas** rather than database truncation or transactions. For every `#[tokio::test]` executed, the `TestContext` dynamically creates a completely isolated PostgreSQL schema (e.g. `test_a1b2c3d4...`) and maps the SQLx connection pool strictly to that `search_path`.
- **Parallelization**: Tests never share state. They run fully concurrently.
- **Safety**: No cross-test pollution. The schema is dropped entirely on `Drop`.

### Writing a New Integration Test
You can utilize the `ApiTestClient` located in `src/test_utils/client.rs`.

```rust
use crate::test_utils::{setup, client::ApiTestClient};

#[tokio::test]
async fn test_create_resource() {
    // 1. Spin up isolated context (DB, Redis, Router)
    let ctx = setup().await;
    let client = ApiTestClient::new(ctx.app);

    // 2. Perform fluent API request
    let payload = serde_json::json!({ "name": "Crucible" });
    
    let response = client.post("/api/resources")
        .bearer("mock-token")
        .json(&payload)
        .send()
        .await;

    // 3. Chain assertions and extract typed body
    response
        .assert_status(StatusCode::CREATED)
        .assert_json_field("name", "Crucible");
}
```

### Adding New Fixture Helpers
To add reusable database states, write standard async functions in `src/test_utils/fixtures.rs` that accept `&PgPool`. Because schemas are isolated, fixtures never need to worry about cleaning up after themselves.

```rust
impl Validate for ProfileTriggerRequest {
    fn validate(&self) -> Result<(), String> {
        if self.duration_secs == 0 {
            return Err("duration_secs must be > 0".to_string());
        }
        Ok(())
    }
}
```
## Structure
- `src/api/` – API handlers and routing
- `src/config/` – Environment configuration
- `src/db/` – Database utilities and seed data
- `src/jobs/` – Background job definitions (Apalis)
- `src/services/` – Business logic and external integrations
- `src/telemetry/` – Observability and logging setup
- `src/workers/` – Background worker modules, including the exponential backoff retry policy

## Exponential Backoff Retry Policy

Crucible includes a production-ready retry module (`src/workers/retry.rs`) to safely retry fallible asynchronous operations (such as SQLx database transactions, Redis calls, or external Stellar Horizon requests).

### Features
- **Exponential Backoff**: Dynamically increases backoff duration using the formula `base_delay * multiplier^attempt`.
- **Full Jitter**: Implements the AWS recommended full jitter strategy to prevent thundering herd problems.
- **Conditional Retries**: Abort retries early based on the specific error encountered using the `ShouldRetry` predicate.
- **Observability**: Built-in structured tracing (`tracing` spans and logs) to track retry attempts, backoff durations, and exhausted errors.
- **Serializable Configuration**: `RetryConfig` supports direct Serialization/Deserialization for config-driven retries.

### Usage Example
```rust
use backend::workers::retry::{RetryPolicy, RetryError};

async fn perform_stellar_tx() -> Result<(), MyError> {
    let policy = RetryPolicy::default()
        .with_base_delay(std::time::Duration::from_millis(100))
        .with_max_delay(std::time::Duration::from_secs(10));

    policy.retry(|| async {
        // Fallible operation
        send_transaction().await
    }).await.map_err(|err| err.into_inner())
}
```



## Permissions & RBAC

The backend implements role-based access control (RBAC) with three built-in roles and fine-grained permission management.

### Roles

| Role | Description |
|---|---|
| `admin` | Full system access, can manage all resources and permissions |
| `user` | Standard user with limited permissions |
| `guest` | Read-only access to public resources |

### Permission System

Permissions are defined as `(resource, action)` pairs stored in PostgreSQL and cached in Redis:

use backend::api::middleware::permissions::{Permission, PermissionChecker};

let checker = PermissionChecker::new(db, redis);
let permission = Permission::new("contracts", "read");

if checker.has_permission(user_id, &permission).await? {
    // User has permission
}

### Default Permissions

| Resource | Actions | Description |
|---|---|---|
| `contracts` | `read`, `write`, `delete` | Smart contract management |
| `test_runs` | `read`, `write`, `delete` | Test execution management |
| `users` | `read`, `write`, `delete` | User management |
| `permissions` | `manage` | Permission assignment |

### Using Permission Middleware

use axum::{routing::get, Router};
use backend::api::middleware::permissions::require_permission;

let app = Router::new()
    .route("/contracts", get(list_contracts))
    .layer(axum::middleware::from_fn_with_state(
        state.clone(),
        require_permission("contracts", "read")
    ));

### Caching Strategy

Permission checks are cached in Redis with a 5-minute TTL:

- Cache key format: `perm:{user_id}:{resource}:{action}`
- Role cache: `role:{user_id}`
- Automatic invalidation on permission changes via `invalidate_cache(user_id)`

### Managing Permissions

```sql
-- Grant permission to user
INSERT INTO user_permissions (user_id, permission_id)
SELECT 123, id FROM permissions 
WHERE resource = 'contracts' AND action = 'write';

-- Revoke permission
DELETE FROM user_permissions 
WHERE user_id = 123 
  AND permission_id = (SELECT id FROM permissions WHERE resource = 'contracts' AND action = 'write');

-- List user permissions
SELECT p.resource, p.action, p.description
FROM user_permissions up
JOIN permissions p ON up.permission_id = p.id
WHERE up.user_id = 123;

### Database Schema

Run migration `20260430000000_permissions.sql` to set up:

- `user_role` enum type (`admin`, `user`, `guest`)
- `permissions` table with resource/action pairs
- `user_permissions` junction table
- Indexes for performance optimization
