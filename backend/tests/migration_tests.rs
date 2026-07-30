use std::process::Command;

#[test]
fn test_zero_downtime_migration_checker_script() {
    let output = Command::new("bash")
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
