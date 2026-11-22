use std::path::PathBuf;
use std::process::Command;

#[test]
fn test_zero_downtime_migration_checker_script() {
    let repo_root = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .expect("backend crate has a parent directory")
        .to_path_buf();

    let output = Command::new("bash")
        .current_dir(&repo_root)
        .arg("backend/scripts/check_migrations.sh")
        .arg("backend/migrations")
        .output()
        .expect("Failed to execute check_migrations.sh");

    assert!(
        output.status.success(),
        "Migration checker failed: {}",
        String::from_utf8_lossy(&output.stdout)
    );
}
