//! Migration safety and applied-schema tests.
//!
//! Two layers:
//!
//! 1. A statement-level SQL linter ([`lint_migration`]) that flags lock-heavy
//!    or unsafe DDL. Strict rules apply to migrations whose version is at or
//!    after [`STRICT_FROM_VERSION`]; earlier migrations are already applied in
//!    production and are only held to the legacy zero-downtime rules.
//! 2. Integration tests that apply every migration to a real Postgres
//!    (`DATABASE_URL`) in a throwaway database and assert on the resulting
//!    schema, representative queries and the main contract lookup plan.
//!
//! The database tests are skipped when `DATABASE_URL` is unset unless
//! `REQUIRE_MIGRATION_DB=1`, which CI sets so they can never be skipped there.

use std::path::{Path, PathBuf};
use std::process::Command;
use std::str::FromStr;

use sqlx::postgres::{PgConnectOptions, PgPoolOptions};
use sqlx::{Connection, PgConnection, PgPool};

/// Migrations with a version >= this are held to the strict rules.
const STRICT_FROM_VERSION: &str = "20261123000000";

#[derive(Debug, Clone, PartialEq, Eq)]
struct Violation {
    rule: &'static str,
    statement: String,
}

fn migrations_dir() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("migrations")
}

// ---------------------------------------------------------------------------
// Linter
// ---------------------------------------------------------------------------

/// Remove `--` and `/* */` comments, and replace dollar-quoted bodies with an
/// empty `$$ $$` so function bodies are not mistaken for top-level DDL.
fn sanitize(sql: &str) -> String {
    let chars: Vec<char> = sql.chars().collect();
    let mut out = String::with_capacity(sql.len());
    let mut i = 0;
    while i < chars.len() {
        let c = chars[i];
        let next = chars.get(i + 1).copied();
        if c == '-' && next == Some('-') {
            while i < chars.len() && chars[i] != '\n' {
                i += 1;
            }
        } else if c == '/' && next == Some('*') {
            i += 2;
            while i + 1 < chars.len() && !(chars[i] == '*' && chars[i + 1] == '/') {
                i += 1;
            }
            i += 2;
        } else if c == '\'' {
            // Keep string literals, but neutralise their contents.
            out.push_str("''");
            i += 1;
            while i < chars.len() {
                if chars[i] == '\'' {
                    if chars.get(i + 1) == Some(&'\'') {
                        i += 2;
                        continue;
                    }
                    break;
                }
                i += 1;
            }
            i += 1;
        } else if c == '$' {
            // Dollar-quote tag: $$ or $tag$
            let mut j = i + 1;
            while j < chars.len() && (chars[j].is_alphanumeric() || chars[j] == '_') {
                j += 1;
            }
            if j < chars.len() && chars[j] == '$' {
                let tag: String = chars[i..=j].iter().collect();
                let rest: String = chars[j + 1..].iter().collect();
                if let Some(end) = rest.find(&tag) {
                    out.push_str("$$ $$");
                    i = j + 1 + rest[..end].chars().count() + tag.chars().count();
                    continue;
                }
            }
            out.push(c);
            i += 1;
        } else {
            out.push(c);
            i += 1;
        }
    }
    out
}

fn statements(sql: &str) -> Vec<String> {
    sanitize(sql)
        .split(';')
        .map(|s| {
            s.split_whitespace()
                .collect::<Vec<_>>()
                .join(" ")
                .to_uppercase()
        })
        .filter(|s| !s.is_empty())
        .collect()
}

/// Table names created within the same migration; indexes and constraints on
/// them cannot block concurrent traffic.
fn created_tables(stmts: &[String]) -> Vec<String> {
    stmts
        .iter()
        .filter_map(|s| {
            let rest = s.strip_prefix("CREATE TABLE ")?;
            let rest = rest.strip_prefix("IF NOT EXISTS ").unwrap_or(rest);
            rest.split([' ', '('])
                .next()
                .map(|t| t.trim_matches('"').to_string())
        })
        .collect()
}

fn index_target_table(stmt: &str) -> Option<String> {
    let idx = stmt.find(" ON ")?;
    let rest = stmt[idx + 4..].trim_start();
    let rest = rest.strip_prefix("ONLY ").unwrap_or(rest);
    rest.split([' ', '('])
        .next()
        .map(|t| t.trim_matches('"').to_string())
}

fn has_directive(raw: &str, directive: &str) -> bool {
    raw.lines()
        .any(|l| l.trim().eq_ignore_ascii_case(&format!("-- {directive}")))
}

/// Lint one migration. `strict` enables the production-safety rules.
fn lint_migration(raw: &str, strict: bool) -> Vec<Violation> {
    let stmts = statements(raw);
    let new_tables = created_tables(&stmts);
    let no_tx = has_directive(raw, "no-transaction");
    let allow_backfill = has_directive(raw, "lint:allow-backfill");
    let mut out = Vec::new();
    let mut flag = |rule: &'static str, s: &str| {
        out.push(Violation {
            rule,
            statement: s.chars().take(120).collect(),
        });
    };

    for s in &stmts {
        // --- legacy zero-downtime rules (all migrations) ---
        if s.contains("DROP COLUMN") && !raw.to_uppercase().contains("CONTRACT") {
            flag("drop-column-without-contract-annotation", s);
        }
        if s.contains("RENAME COLUMN") {
            flag("rename-column", s);
        }
        if s.contains("ADD COLUMN") && s.contains("NOT NULL") && !s.contains("DEFAULT") {
            flag("add-not-null-column-without-default", s);
        }

        if !strict {
            continue;
        }

        // --- strict rules (new migrations only) ---
        let is_create_index =
            s.starts_with("CREATE INDEX ") || s.starts_with("CREATE UNIQUE INDEX ");
        if is_create_index {
            let concurrent = s.contains(" CONCURRENTLY ");
            let on_new_table = index_target_table(s)
                .map(|t| new_tables.contains(&t))
                .unwrap_or(false);
            if !concurrent && !on_new_table {
                flag("create-index-not-concurrent", s);
            }
        }
        if s.starts_with("DROP INDEX ") && !s.contains(" CONCURRENTLY ") {
            flag("drop-index-not-concurrent", s);
        }
        if s.contains(" CONCURRENTLY ") && !no_tx {
            flag("concurrently-requires-no-transaction-directive", s);
        }
        if s.starts_with("CREATE TABLE IF NOT EXISTS ") {
            flag("create-table-if-not-exists-masks-existing-object", s);
        }
        if s.starts_with("ALTER TABLE ")
            && (s.contains(" ALTER COLUMN ")
                && (s.contains(" TYPE ") || s.contains(" SET DATA TYPE ")))
        {
            flag("alter-column-type-rewrites-table", s);
        }
        if s.starts_with("ALTER TABLE ") && s.contains(" SET NOT NULL") {
            flag("set-not-null-scans-table", s);
        }
        if s.starts_with("ALTER TABLE ")
            && s.contains(" ADD CONSTRAINT ")
            && (s.contains(" FOREIGN KEY ") || s.contains(" CHECK ") || s.contains(" CHECK("))
            && !s.contains(" NOT VALID")
        {
            let on_new_table = index_target_table(&format!("X ON {}", &s["ALTER TABLE ".len()..]))
                .map(|t| new_tables.contains(&t))
                .unwrap_or(false);
            if !on_new_table {
                flag("add-constraint-without-not-valid", s);
            }
        }
        if (s.starts_with("UPDATE ") || s.starts_with("DELETE FROM ")) && !allow_backfill {
            flag("data-backfill-needs-explicit-allow", s);
        }
        if s.starts_with("LOCK TABLE ") || s.starts_with("VACUUM FULL") || s.starts_with("CLUSTER ")
        {
            flag("lock-heavy-statement", s);
        }
    }
    out
}

fn lint_dir(dir: &Path) -> Vec<(String, Violation)> {
    let mut all = Vec::new();
    let mut files: Vec<_> = std::fs::read_dir(dir)
        .expect("read migrations dir")
        .filter_map(|e| e.ok())
        .map(|e| e.path())
        .filter(|p| p.extension().map(|e| e == "sql").unwrap_or(false))
        .collect();
    files.sort();
    for path in files {
        let name = path.file_name().unwrap().to_string_lossy().to_string();
        let version = name.split('_').next().unwrap_or("");
        let strict = version >= STRICT_FROM_VERSION;
        let raw = std::fs::read_to_string(&path).expect("read migration");
        for v in lint_migration(&raw, strict) {
            all.push((name.clone(), v));
        }
    }
    all
}

#[test]
fn repository_migrations_pass_lint() {
    let violations = lint_dir(&migrations_dir());
    assert!(
        violations.is_empty(),
        "unsafe migration patterns found:\n{}",
        violations
            .iter()
            .map(|(f, v)| format!("  {f}: [{}] {}", v.rule, v.statement))
            .collect::<Vec<_>>()
            .join("\n")
    );
}

fn rules(sql: &str) -> Vec<&'static str> {
    lint_migration(sql, true)
        .into_iter()
        .map(|v| v.rule)
        .collect()
}

#[test]
fn lint_detects_non_concurrent_index_on_existing_table() {
    assert_eq!(
        rules("CREATE INDEX idx_a ON contracts(name);"),
        vec!["create-index-not-concurrent"]
    );
    assert_eq!(
        rules("CREATE UNIQUE INDEX idx_a ON contracts (name);"),
        vec!["create-index-not-concurrent"]
    );
}

#[test]
fn lint_allows_index_on_table_created_in_same_migration() {
    let sql = "CREATE TABLE widgets (id BIGINT PRIMARY KEY, name TEXT);\n\
               CREATE INDEX idx_widgets_name ON widgets(name);";
    assert!(rules(sql).is_empty(), "{:?}", rules(sql));
}

#[test]
fn lint_requires_no_transaction_directive_for_concurrently() {
    let without = "CREATE INDEX CONCURRENTLY idx_a ON contracts(name);";
    assert_eq!(
        rules(without),
        vec!["concurrently-requires-no-transaction-directive"]
    );
    let with = "-- no-transaction\nCREATE INDEX CONCURRENTLY idx_a ON contracts(name);";
    assert!(rules(with).is_empty());
}

#[test]
fn lint_detects_drop_index_not_concurrent() {
    assert_eq!(
        rules("DROP INDEX IF EXISTS idx_a;"),
        vec!["drop-index-not-concurrent"]
    );
    assert!(rules("-- no-transaction\nDROP INDEX CONCURRENTLY IF EXISTS idx_a;").is_empty());
}

#[test]
fn lint_detects_type_changes_and_set_not_null() {
    assert_eq!(
        rules("ALTER TABLE contracts ALTER COLUMN name TYPE TEXT;"),
        vec!["alter-column-type-rewrites-table"]
    );
    assert_eq!(
        rules("ALTER TABLE contracts ALTER COLUMN name SET DATA TYPE TEXT;"),
        vec!["alter-column-type-rewrites-table"]
    );
    assert_eq!(
        rules("ALTER TABLE contracts ALTER COLUMN name SET NOT NULL;"),
        vec!["set-not-null-scans-table"]
    );
}

#[test]
fn lint_detects_unvalidated_constraints_only_when_not_marked_not_valid() {
    assert_eq!(
        rules("ALTER TABLE transactions ADD CONSTRAINT fk FOREIGN KEY (a) REFERENCES b(id);"),
        vec!["add-constraint-without-not-valid"]
    );
    assert!(rules(
        "ALTER TABLE transactions ADD CONSTRAINT fk FOREIGN KEY (a) REFERENCES b(id) NOT VALID;"
    )
    .is_empty());
}

#[test]
fn lint_detects_backfills_unless_explicitly_allowed() {
    assert_eq!(
        rules("UPDATE contracts SET name = 'x' WHERE name IS NULL;"),
        vec!["data-backfill-needs-explicit-allow"]
    );
    assert_eq!(
        rules("DELETE FROM contracts WHERE name IS NULL;"),
        vec!["data-backfill-needs-explicit-allow"]
    );
    assert!(rules("-- lint:allow-backfill\nUPDATE contracts SET name = 'x';").is_empty());
}

#[test]
fn lint_ignores_dml_inside_function_bodies_and_comments() {
    let sql = "-- UPDATE contracts SET a = 1;\n\
               CREATE OR REPLACE FUNCTION f() RETURNS TRIGGER AS $$\n\
               BEGIN UPDATE contracts SET a = 1; RETURN NEW; END;\n\
               $$ LANGUAGE plpgsql;";
    assert!(rules(sql).is_empty(), "{:?}", rules(sql));
}

#[test]
fn lint_detects_create_table_if_not_exists_and_lock_heavy_statements() {
    assert_eq!(
        rules("CREATE TABLE IF NOT EXISTS t (id INT);"),
        vec!["create-table-if-not-exists-masks-existing-object"]
    );
    assert_eq!(
        rules("LOCK TABLE contracts IN ACCESS EXCLUSIVE MODE;"),
        vec!["lock-heavy-statement"]
    );
}

#[test]
fn lint_applies_legacy_rules_to_all_migrations() {
    let legacy = |sql: &str| -> Vec<&'static str> {
        lint_migration(sql, false)
            .into_iter()
            .map(|v| v.rule)
            .collect()
    };
    assert_eq!(
        legacy("ALTER TABLE t RENAME COLUMN a TO b;"),
        vec!["rename-column"]
    );
    assert_eq!(
        legacy("ALTER TABLE t ADD COLUMN a INT NOT NULL;"),
        vec!["add-not-null-column-without-default"]
    );
    // Strict-only rules do not fire for legacy migrations.
    assert!(legacy("CREATE INDEX idx_a ON contracts(name);").is_empty());
}

#[test]
fn legacy_shell_checker_script_still_passes() {
    let repo_root = Path::new(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .expect("backend crate has a parent directory")
        .to_path_buf();

    let output = match Command::new("bash")
        .current_dir(&repo_root)
        .arg("backend/scripts/check_migrations.sh")
        .arg("backend/migrations")
        .output()
    {
        Ok(o) => o,
        Err(e) => {
            eprintln!("skipping: bash unavailable: {e}");
            return;
        }
    };

    assert!(
        output.status.success(),
        "Migration checker failed: {}",
        String::from_utf8_lossy(&output.stdout)
    );
}

// ---------------------------------------------------------------------------
// Applied-schema tests (real Postgres)
// ---------------------------------------------------------------------------

/// A throwaway database with every migration applied.
struct MigratedDb {
    pool: PgPool,
    admin_url: String,
    name: String,
}

impl MigratedDb {
    /// Returns `None` (skip) when no database is configured, unless
    /// `REQUIRE_MIGRATION_DB=1`, in which case it panics.
    async fn create() -> Option<Self> {
        let url = match std::env::var("DATABASE_URL").ok().filter(|u| !u.is_empty()) {
            Some(u) => u,
            None => {
                if std::env::var("REQUIRE_MIGRATION_DB").as_deref() == Ok("1") {
                    panic!("REQUIRE_MIGRATION_DB=1 but DATABASE_URL is not set");
                }
                eprintln!("skipping: DATABASE_URL not set");
                return None;
            }
        };

        let name = format!("crucible_migtest_{}", uuid::Uuid::new_v4().simple());
        let mut admin = PgConnection::connect(&url)
            .await
            .expect("connect to DATABASE_URL");
        sqlx::query(&format!("CREATE DATABASE \"{name}\""))
            .execute(&mut admin)
            .await
            .expect("create throwaway database");

        let opts = PgConnectOptions::from_str(&url)
            .expect("parse DATABASE_URL")
            .database(&name);
        let pool = PgPoolOptions::new()
            .max_connections(2)
            .connect_with(opts)
            .await
            .expect("connect to throwaway database");

        sqlx::migrate!("./migrations")
            .run(&pool)
            .await
            .expect("all migrations apply to an empty database");

        Some(Self {
            pool,
            admin_url: url,
            name,
        })
    }

    async fn cleanup(self) {
        self.pool.close().await;
        if let Ok(mut admin) = PgConnection::connect(&self.admin_url).await {
            let _ = sqlx::query(&format!(
                "DROP DATABASE IF EXISTS \"{}\" WITH (FORCE)",
                self.name
            ))
            .execute(&mut admin)
            .await;
        }
    }
}

#[tokio::test]
async fn all_migrations_apply_and_are_recorded() {
    let Some(db) = MigratedDb::create().await else {
        return;
    };

    let expected = sqlx::migrate!("./migrations").migrations.len() as i64;
    let applied: i64 =
        sqlx::query_scalar("SELECT COUNT(*) FROM _sqlx_migrations WHERE success = TRUE")
            .fetch_one(&db.pool)
            .await
            .unwrap();
    assert_eq!(
        applied, expected,
        "every migration must be recorded as applied"
    );

    // Re-running is a no-op: the migrator is idempotent against its own history.
    sqlx::migrate!("./migrations").run(&db.pool).await.unwrap();

    db.cleanup().await;
}

#[tokio::test]
async fn contracts_lookup_has_single_unique_index_and_no_redundant_index() {
    let Some(db) = MigratedDb::create().await else {
        return;
    };

    // Every index on contracts(contract_id) as (name, is_unique).
    let indexes: Vec<(String, bool)> = sqlx::query_as(
        "SELECT i.relname::text, x.indisunique \
         FROM pg_index x \
         JOIN pg_class i ON i.oid = x.indexrelid \
         JOIN pg_class t ON t.oid = x.indrelid \
         JOIN pg_namespace n ON n.oid = t.relnamespace \
         JOIN pg_attribute a ON a.attrelid = t.oid AND a.attnum = x.indkey[0] \
         WHERE n.nspname = 'public' AND t.relname = 'contracts' \
           AND x.indnatts = 1 AND a.attname = 'contract_id'",
    )
    .fetch_all(&db.pool)
    .await
    .unwrap();
    assert_eq!(
        indexes.len(),
        1,
        "contracts(contract_id) must have exactly one index, found {indexes:?}"
    );
    assert!(indexes[0].1, "the remaining index must be the UNIQUE one");

    // General redundancy detector: a plain index whose key is identical to a
    // unique/primary index on the same table. Asserted for `contracts`;
    // reported for other tables so drift is visible without failing here.
    let redundant: Vec<(String, String, String)> = sqlx::query_as(
        "SELECT c.relname::text, a.indexrelid::regclass::text, b.indexrelid::regclass::text \
         FROM pg_index a \
         JOIN pg_index b ON a.indrelid = b.indrelid \
              AND a.indexrelid <> b.indexrelid AND a.indkey = b.indkey \
         JOIN pg_class c ON c.oid = a.indrelid \
         JOIN pg_namespace n ON n.oid = c.relnamespace \
         WHERE n.nspname = 'public' \
           AND a.indpred IS NULL AND b.indpred IS NULL \
           AND a.indexprs IS NULL AND b.indexprs IS NULL \
           AND NOT a.indisunique AND (b.indisunique OR b.indisprimary)",
    )
    .fetch_all(&db.pool)
    .await
    .unwrap();
    for (table, idx, covering) in &redundant {
        eprintln!("note: index {idx} on {table} is covered by {covering}");
    }
    assert!(
        redundant.iter().all(|(table, _, _)| table != "contracts"),
        "redundant indexes on contracts: {redundant:?}"
    );

    db.cleanup().await;
}

#[tokio::test]
async fn main_contract_lookup_uses_the_unique_index() {
    let Some(db) = MigratedDb::create().await else {
        return;
    };

    let mut conn = db.pool.acquire().await.unwrap();
    // Force the planner to prefer an index on the (empty) table so the plan
    // reflects which index is available, not table statistics.
    sqlx::query("SET enable_seqscan = off")
        .execute(&mut *conn)
        .await
        .unwrap();
    let plan: Vec<String> = sqlx::query_scalar(
        "EXPLAIN SELECT id, contract_id FROM contracts WHERE contract_id = 'C123'",
    )
    .fetch_all(&mut *conn)
    .await
    .unwrap();
    let plan = plan.join("\n");
    assert!(
        plan.contains("contracts_contract_id_key"),
        "contract lookup should use the UNIQUE constraint index, plan was:\n{plan}"
    );
    assert!(!plan.contains("Seq Scan"), "unexpected seq scan:\n{plan}");

    drop(conn);
    db.cleanup().await;
}

#[tokio::test]
async fn schema_enforces_expected_constraints() {
    let Some(db) = MigratedDb::create().await else {
        return;
    };
    let pool = &db.pool;

    sqlx::query("INSERT INTO contracts (contract_id, name) VALUES ('C1', 'one')")
        .execute(pool)
        .await
        .unwrap();

    // UNIQUE(contract_id)
    let dup = sqlx::query("INSERT INTO contracts (contract_id, name) VALUES ('C1', 'dup')")
        .execute(pool)
        .await;
    assert!(dup.is_err(), "duplicate contract_id must be rejected");

    // CHECK on transactions.status
    let bad_status = sqlx::query(
        "INSERT INTO transactions (transaction_hash, contract_id, status) VALUES ('h1', 'C1', 'bogus')",
    )
    .execute(pool)
    .await;
    assert!(bad_status.is_err(), "invalid status must be rejected");

    // FK transactions.contract_id -> contracts.contract_id
    let orphan = sqlx::query(
        "INSERT INTO transactions (transaction_hash, contract_id, status) VALUES ('h2', 'missing', 'success')",
    )
    .execute(pool)
    .await;
    assert!(orphan.is_err(), "orphan transaction must be rejected");

    // Valid insert, then ON DELETE CASCADE.
    sqlx::query(
        "INSERT INTO transactions (transaction_hash, contract_id, status) VALUES ('h3', 'C1', 'success')",
    )
    .execute(pool)
    .await
    .unwrap();
    sqlx::query("DELETE FROM contracts WHERE contract_id = 'C1'")
        .execute(pool)
        .await
        .unwrap();
    let remaining: i64 =
        sqlx::query_scalar("SELECT COUNT(*) FROM transactions WHERE contract_id = 'C1'")
            .fetch_one(pool)
            .await
            .unwrap();
    assert_eq!(remaining, 0, "deleting a contract cascades to transactions");

    db.cleanup().await;
}
