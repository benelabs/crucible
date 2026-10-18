//! Automated checks on the sandbox's security posture.
//!
//! These tests verify structural properties of the deploy-time artifacts
//! (the seccomp profile and the resource limits baked into the sandbox
//! service) that CI can check on every change. They are NOT a substitute for
//! a live container-escape drill: verifying that a *running* gVisor/seccomp
//! container actually blocks a raw-socket or ptrace attempt requires a
//! provisioned container runtime, which this test suite does not have
//! access to. That gap is documented, not silently skipped — see
//! deployments/docker/SANDBOX_SECURITY.md for the manual/CI-with-Docker
//! verification procedure.

use serde_json::Value;
use std::fs;
use std::path::PathBuf;

fn seccomp_profile() -> Value {
    let path = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../deployments/docker/sandbox-seccomp.json");
    let raw = fs::read_to_string(&path)
        .unwrap_or_else(|e| panic!("failed to read {}: {e}", path.display()));
    serde_json::from_str(&raw).unwrap_or_else(|e| panic!("invalid seccomp JSON: {e}"))
}

fn allowed_syscalls(profile: &Value) -> Vec<String> {
    profile["syscalls"]
        .as_array()
        .expect("syscalls must be an array")
        .iter()
        .filter(|rule| rule["action"] == "SCMP_ACT_ALLOW")
        .flat_map(|rule| {
            rule["names"]
                .as_array()
                .expect("names must be an array")
                .iter()
                .map(|n| n.as_str().unwrap().to_string())
        })
        .collect()
}

#[test]
fn seccomp_profile_is_deny_by_default() {
    let profile = seccomp_profile();
    assert_eq!(
        profile["defaultAction"], "SCMP_ACT_ERRNO",
        "the profile must deny any syscall not explicitly allow-listed"
    );
}

#[test]
fn seccomp_profile_blocks_privilege_escalation_and_kernel_tampering() {
    let profile = seccomp_profile();
    let allowed = allowed_syscalls(&profile);

    let must_not_be_allowed = [
        "ptrace",
        "process_vm_readv",
        "process_vm_writev",
        "mount",
        "umount2",
        "pivot_root",
        "chroot",
        "setns",
        "unshare",
        "setuid",
        "setgid",
        "setresuid",
        "setresgid",
        "capset",
        "init_module",
        "finit_module",
        "delete_module",
        "kexec_load",
        "kexec_file_load",
        "reboot",
        "swapon",
        "swapoff",
        "acct",
        "iopl",
        "ioperm",
        "bpf",
        "add_key",
        "request_key",
        "keyctl",
        "execve",
        "execveat",
    ];

    for syscall in must_not_be_allowed {
        assert!(
            !allowed.contains(&syscall.to_string()),
            "{syscall} must not be in the seccomp allow-list — it is a privilege \
            escalation, kernel-tampering, or process-spawning primitive"
        );
    }
}

#[test]
fn seccomp_profile_has_no_contradictory_rules() {
    let profile = seccomp_profile();
    let rules = profile["syscalls"].as_array().unwrap();

    let mut seen = std::collections::HashMap::new();
    for rule in rules {
        let action = rule["action"].as_str().unwrap().to_string();
        for name in rule["names"].as_array().unwrap() {
            let name = name.as_str().unwrap().to_string();
            if let Some(prev_action) = seen.insert(name.clone(), action.clone()) {
                assert_eq!(
                    prev_action, action,
                    "{name} appears with conflicting actions ({prev_action} vs {action}) — \
                    seccomp rule ordering is not guaranteed, so a syscall must only ever \
                    appear once"
                );
            }
        }
    }
}

#[test]
fn seccomp_profile_allows_the_syscalls_a_tokio_http_service_needs() {
    let profile = seccomp_profile();
    let allowed = allowed_syscalls(&profile);

    // A minimal but real set the sandbox-executor (tokio + axum) needs to
    // start, accept connections, and run the WASM interpreter. This is
    // deliberately broader than the issue's illustrative "read, write, exit,
    // futex" — that literal set cannot host an HTTP service at all.
    let required = [
        "read", "write", "exit", "exit_group", "futex", "mmap", "munmap", "mprotect", "brk",
        "clone", "clone3", "socket", "bind", "listen", "accept4", "epoll_wait", "epoll_ctl",
        "close", "openat",
    ];

    for syscall in required {
        assert!(
            allowed.contains(&syscall.to_string()),
            "{syscall} is required for the sandbox-executor to run but is missing \
            from the seccomp allow-list"
        );
    }
}

#[test]
fn sandbox_resource_limits_match_the_dockerfile_env_defaults() {
    // Keeps backend/src/services/sandbox.rs::SandboxLimits::default() honest
    // against the ENV defaults baked into deployments/docker/sandbox.Dockerfile.
    // If either changes without the other, a WASM binary that the service
    // accepts could still be rejected by the container's own env-configured
    // limits (or vice versa) — a silent policy drift, not a crash, so it
    // needs an explicit test rather than relying on a build failure.
    let dockerfile_path = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../deployments/docker/sandbox.Dockerfile");
    let dockerfile = fs::read_to_string(&dockerfile_path)
        .unwrap_or_else(|e| panic!("failed to read {}: {e}", dockerfile_path.display()));

    let expectations = [
        ("SANDBOX_LIMITS_MAX_WASM_BYTES", "2097152"),
        ("SANDBOX_LIMITS_MAX_ARGS", "32"),
        ("SANDBOX_LIMITS_MAX_ARG_XDR_BYTES", "65536"),
        ("SANDBOX_LIMITS_MAX_CPU_INSTRUCTIONS", "25000000"),
        ("SANDBOX_LIMITS_MAX_MEMORY_BYTES", "268435456"),
        ("SANDBOX_LIMITS_TIMEOUT_MS", "2000"),
    ];

    for (key, value) in expectations {
        let needle = format!("{key}={value}");
        assert!(
            dockerfile.contains(&needle),
            "expected {needle} in sandbox.Dockerfile to match \
            SandboxLimits::default() in backend/src/services/sandbox.rs"
        );
    }
}
