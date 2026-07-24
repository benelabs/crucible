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
