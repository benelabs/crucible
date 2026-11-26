# Zero-Downtime Database Migration Strategy Guide

This guide establishes the required multi-phase **Expand and Contract** pattern for database schema changes in Crucible to ensure zero API downtime during deployments.

---

## 1. Core Principles

1. **No Breaking DDL**: Direct `DROP COLUMN`, `RENAME COLUMN`, `DROP TABLE`, or adding `NOT NULL` without a `DEFAULT` locks tables and breaks active running API application instances.
2. **Backward Compatibility**: Every migration MUST be backward compatible with the currently deployed API version ($V_N$) and forward compatible with the new version ($V_{N+1}$).
3. **Multi-Phase Deployment**: Schema alterations that remove or rename database structures must be executed in separate deployment phases.

---

## 2. Expand and Contract Pattern

### Scenario: Renaming or Refactoring a Column (`old_name` -> `new_name`)

#### **Phase 1: Expand (Migration 1)**
- Add the new column `new_name` as NULLABLE or with a DEFAULT.
- Create a database trigger or application dual-write layer to synchronize data written to `old_name` into `new_name`.
- Backfill existing historical rows in background batches.
- **Deploy $V_{N+1}$ API**: Code reads from `new_name`, writes to both `old_name` and `new_name`.

#### **Phase 2: Transition**
- Verify backfill completeness.
- **Deploy $V_{N+2}$ API**: Code reads and writes exclusively using `new_name`.

#### **Phase 3: Contract (Migration 2)**
- Remove trigger / dual-write mechanism.
- Execute `ALTER TABLE ... DROP COLUMN old_name;` safely after verifying no application instance references `old_name`.

---

## 3. Automated CI Enforcement

Crucible runs automated CI migration validation via `backend/scripts/check_migrations.sh` on every pull request.
The CI check fails if any migration file contains:
- Unsafe `DROP COLUMN`
- Unsafe `RENAME COLUMN`
- `ALTER TABLE ... NOT NULL` without default values
- `DROP TABLE` without prior deprecation window

---

## 4. Migration Linter and Applied-Schema Tests

`backend/tests/migration_tests.rs` replaces grep-only checking with a statement-level linter plus tests that apply every migration to a real Postgres (`DATABASE_URL`; CI sets `REQUIRE_MIGRATION_DB=1` so they cannot be skipped).

Migrations with a version of `20261123000000` or later are held to the strict rules below. Earlier migrations are already applied in production and are only checked against the legacy rules in section 3.

| Rule | What to do instead |
| --- | --- |
| `CREATE INDEX` on an existing table without `CONCURRENTLY` | Use `CREATE INDEX CONCURRENTLY` |
| `DROP INDEX` without `CONCURRENTLY` | Use `DROP INDEX CONCURRENTLY` |
| `CONCURRENTLY` without a `-- no-transaction` first line | Add `-- no-transaction` as the first line of the file (sqlx runs it outside a transaction) |
| `ALTER COLUMN ... TYPE` | Expand and contract: add a new column, backfill, switch, drop |
| `SET NOT NULL` | Add a `CHECK (col IS NOT NULL) NOT VALID`, validate it, then set NOT NULL |
| `ADD CONSTRAINT ... FOREIGN KEY / CHECK` without `NOT VALID` | Add `NOT VALID`, then `VALIDATE CONSTRAINT` in a later migration |
| `UPDATE` / `DELETE` backfills | Batch the backfill outside the migration, or add `-- lint:allow-backfill` after review |
| `CREATE TABLE IF NOT EXISTS` | Use plain `CREATE TABLE` so unexpected existing objects fail loudly |
| `LOCK TABLE`, `VACUUM FULL`, `CLUSTER` | Do not run these in a migration |

Indexes and constraints on a table created in the same migration are exempt from the index and constraint rules.

Run locally with a Postgres available:

```bash
DATABASE_URL=postgres://postgres:postgres@localhost:5432/postgres   cargo test -p backend --test migration_tests
```
