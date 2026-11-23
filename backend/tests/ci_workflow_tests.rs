//! Regression tests for the backend CI verification contract.
//!
//! These guard against a workflow that silently stops verifying the backend
//! (for example by dropping tests, clippy, formatting or the SQLx checks).

use std::path::PathBuf;

fn repo_root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .expect("backend crate must live in the workspace root")
        .to_path_buf()
}

fn workflow() -> String {
    std::fs::read_to_string(repo_root().join(".github/workflows/backend-ci.yml"))
        .expect("backend-ci.yml must exist")
}

#[test]
fn backend_ci_runs_all_quality_gates() {
    let workflow = workflow();

    for command in [
        "./backend/scripts/check-sqlx-strategy.sh",
        "cargo fmt -p backend -- --check",
        "cargo clippy -p backend --all-targets -- -D warnings",
        "SQLX_OFFLINE: 'true'",
        "cargo check -p backend --all-targets",
        "cargo test -p backend --all-features",
        "sqlx migrate run --source backend/migrations",
    ] {
        assert!(
            workflow.contains(command),
            "backend CI must run `{command}`"
        );
    }
}

#[test]
fn backend_ci_provisions_postgres_and_redis() {
    let workflow = workflow();

    assert!(workflow.contains("image: postgres:16-alpine"));
    assert!(workflow.contains("image: redis:7-alpine"));
    assert!(workflow.contains("DATABASE_URL: postgres://postgres:postgres@localhost:5432"));
    assert!(workflow.contains("REDIS_URL: redis://127.0.0.1:6379"));
    assert!(workflow.contains("'backend/**'"));
}

#[test]
fn stale_backend_diagnostics_are_removed() {
    assert!(
        !repo_root().join("backend/compile_errors.txt").exists(),
        "stale SQLx compile-error log must not be committed"
    );
    assert!(repo_root()
        .join("backend/scripts/check-sqlx-strategy.sh")
        .exists());
}
