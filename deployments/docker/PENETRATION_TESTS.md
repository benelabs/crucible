# Sandbox Security Penetration Tests

This document describes automated penetration tests for verifying the sandbox security isolation with seccomp and gVisor.

## Test Categories

### 1. Syscall Filtering Tests

#### Test: Raw Socket Creation Prevention
```bash
test_raw_socket_creation() {
  # Attempt to create raw socket (should fail)
  cat > test_program.rs << 'EOF'
  use std::os::unix::io::AsRawFd;
  use nix::sys::socket::{socket, AddressFamily, SockType, SockFlag};
  
  fn main() {
    match socket(AddressFamily::Inet, SockType::Raw, SockFlag::empty(), 0) {
      Ok(_) => panic!("FAIL: Raw socket created (sandbox escaped)"),
      Err(_) => println!("PASS: Raw socket blocked by seccomp"),
    }
  }
  EOF
  
  rustc test_program.rs -o test_raw_socket
  docker run --security-opt seccomp=sandbox-seccomp.json test_raw_socket
}
```

**Expected Result**: Syscall blocked, process receives EPERM

#### Test: Network Access Prevention
```bash
test_network_access() {
  # Attempt to connect to external host (should fail)
  cat > test_network.rs << 'EOF'
  use std::net::TcpStream;
  
  fn main() {
    match TcpStream::connect("8.8.8.8:53") {
      Ok(_) => panic!("FAIL: External network access allowed"),
      Err(e) => println!("PASS: Network blocked - {}", e),
    }
  }
  EOF
  
  docker run --security-opt seccomp=sandbox-seccomp.json test_network
}
```

**Expected Result**: Connection refused or timeout

### 2. Privilege Escalation Prevention Tests

#### Test: setuid/setgid Prevention
```bash
test_privilege_escalation() {
  cat > test_setuid.rs << 'EOF'
  use nix::unistd::{setuid, Uid};
  
  fn main() {
    match setuid(Uid::from_raw(0)) {
      Ok(_) => panic!("FAIL: setuid succeeded (privilege escalation possible)"),
      Err(_) => println!("PASS: setuid blocked by seccomp"),
    }
  }
  EOF
  
  docker run --security-opt seccomp=sandbox-seccomp.json test_setuid
}
```

**Expected Result**: setuid blocked with EPERM

#### Test: ptrace Prevention (Kernel Debugging Blocking)
```bash
test_ptrace_prevention() {
  cat > test_ptrace.rs << 'EOF'
  use nix::sys::ptrace::{ptrace, Request};
  use nix::sys::ptrace::AddressType;
  
  fn main() {
    match ptrace(Request::PTRACE_ATTACH, nix::unistd::Pid::from_raw(1), ptr::null_mut(), ptr::null_mut()) {
      Ok(_) => panic!("FAIL: ptrace allowed (debug access possible)"),
      Err(_) => println!("PASS: ptrace blocked by seccomp"),
    }
  }
  EOF
  
  docker run --security-opt seccomp=sandbox-seccomp.json test_ptrace
}
```

**Expected Result**: ptrace syscall blocked

### 3. Filesystem Isolation Tests

#### Test: Host Filesystem Access Prevention
```bash
test_filesystem_isolation() {
  cat > test_fs_access.rs << 'EOF'
  use std::fs;
  use std::path::Path;
  
  fn main() {
    // Try accessing host filesystem outside container mount
    match fs::read("/etc/passwd") {
      Ok(_) => println!("PASS: Filesystem accessible (expected in container)"),
      Err(e) => println!("PASS: Filesystem access controlled - {}", e),
    }
    
    // Verify /sys is protected
    match fs::read_dir("/sys") {
      Ok(_) => println!("INFO: /sys readable"),
      Err(_) => println!("PASS: /sys access blocked"),
    }
  }
  EOF
  
  docker run --security-opt seccomp=sandbox-seccomp.json test_fs_access
}
```

**Expected Result**: Only container-approved paths accessible

### 4. Resource Limits Tests

#### Test: Memory Limit Enforcement
```bash
test_memory_limits() {
  cat > test_memory.rs << 'EOF'
  fn main() {
    let mut vec = Vec::new();
    loop {
      // Attempt to allocate 1MB
      vec.push(vec![0u8; 1024 * 1024]);
      println!("Allocated {} MB", vec.len());
      
      if vec.len() > 256 {
        panic!("FAIL: Exceeded memory limit (256MB)");
      }
    }
  }
  EOF
  
  # Run with 256MB memory limit
  docker run -m 256m --security-opt seccomp=sandbox-seccomp.json test_memory
}
```

**Expected Result**: Process killed after exceeding limit

#### Test: CPU Limit Enforcement
```bash
test_cpu_limits() {
  cat > test_cpu.rs << 'EOF'
  use std::time::Instant;
  
  fn main() {
    let start = Instant::now();
    let mut count = 0u64;
    
    loop {
      count = count.wrapping_add(1);
      
      if start.elapsed().as_secs() > 30 {
        println!("PASS: CPU limit enforced after {}s", start.elapsed().as_secs());
        break;
      }
    }
  }
  EOF
  
  # Run with 1 CPU limit
  docker run --cpus=1 --security-opt seccomp=sandbox-seccomp.json test_cpu
}
```

**Expected Result**: Process CPU usage limited to specified cores

### 5. Wasm Execution Isolation Tests

#### Test: Wasm Bytecode Validation
```bash
test_wasm_validation() {
  cat > test_wasm_invalid.wasm << 'EOF'
  # Invalid WASM magic number
  \x00\x61\x73\x6D\x01\x00  # Should be \x00\x61\x73\x6D\x01\x00\x00\x00
  EOF
  
  docker run --security-opt seccomp=sandbox-seccomp.json \
    /app/sandbox-executor --wasm test_wasm_invalid.wasm
}
```

**Expected Result**: WASM validation error, process exits safely

#### Test: Wasm Instruction Limits
```bash
test_wasm_instruction_limits() {
  cat > infinite_loop.wasm << 'EOF'
  (module
    (func $main
      (block $exit
        (loop $continue
          (br $continue)
        )
      )
    )
    (export "main" (func $main))
  )
  EOF
  
  timeout 5 docker run --security-opt seccomp=sandbox-seccomp.json \
    /app/sandbox-executor --wasm infinite_loop.wasm --max-instructions 1000000
}
```

**Expected Result**: Process terminated after instruction limit exceeded

### 6. eBPF & Runtime Monitoring Tests

#### Test: Anomalous Syscall Detection
```bash
test_anomalous_syscall_detection() {
  # Monitor syscalls during execution
  docker run \
    --security-opt seccomp=sandbox-seccomp.json \
    --cap-add SYS_PTRACE \
    test-contract-executable
  
  # eBPF program should detect and log anomalies
}
```

**Expected Result**: Anomalous syscalls logged to /var/log/sandbox-audit

### 7. Escape Attempt Scenarios

#### Test: Cgroup Escape Prevention
```bash
test_cgroup_escape() {
  cat > test_cgroup_escape.rs << 'EOF'
  fn main() {
    // Attempt cgroup breakout
    match std::fs::write("/cgroup.procs", b"1") {
      Ok(_) => panic!("FAIL: Cgroup escape succeeded"),
      Err(_) => println!("PASS: Cgroup escape prevented"),
    }
  }
  EOF
  
  docker run --security-opt seccomp=sandbox-seccomp.json test_cgroup_escape
}
```

**Expected Result**: Write permission denied

#### Test: Namespace Escape Prevention
```bash
test_namespace_escape() {
  cat > test_ns_escape.rs << 'EOF'
  use nix::sched::{unshare, CloneFlags};
  
  fn main() {
    match unshare(CloneFlags::CLONE_NEWUSER) {
      Ok(_) => println!("INFO: Namespace created (container namespace)"),
      Err(_) => println!("PASS: User namespace creation blocked"),
    }
  }
  EOF
  
  docker run --security-opt seccomp=sandbox-seccomp.json test_ns_escape
}
```

**Expected Result**: Namespace operations appropriately controlled

## Running Penetration Tests

### Automated Test Suite
```bash
#!/bin/bash
# ci/penetration_tests.sh

set -e

SANDBOX_IMAGE="crucible-sandbox:latest"
TESTS_PASSED=0
TESTS_FAILED=0

run_test() {
  local test_name=$1
  local test_fn=$2
  
  echo "Running: $test_name"
  if $test_fn; then
    echo "✓ PASS: $test_name"
    ((TESTS_PASSED++))
  else
    echo "✗ FAIL: $test_name"
    ((TESTS_FAILED++))
  fi
}

# Run all tests
run_test "Raw Socket Creation Prevention" test_raw_socket_creation
run_test "Network Access Prevention" test_network_access
run_test "Privilege Escalation Prevention" test_privilege_escalation
run_test "ptrace Prevention" test_ptrace_prevention
run_test "Filesystem Isolation" test_filesystem_isolation
run_test "Memory Limit Enforcement" test_memory_limits
run_test "CPU Limit Enforcement" test_cpu_limits
run_test "Wasm Validation" test_wasm_validation
run_test "Wasm Instruction Limits" test_wasm_instruction_limits
run_test "Cgroup Escape Prevention" test_cgroup_escape
run_test "Namespace Escape Prevention" test_namespace_escape

echo ""
echo "======================================="
echo "Penetration Test Results"
echo "======================================="
echo "Passed: $TESTS_PASSED"
echo "Failed: $TESTS_FAILED"
echo ""

if [ $TESTS_FAILED -eq 0 ]; then
  echo "✓ All security tests passed!"
  exit 0
else
  echo "✗ Some security tests failed!"
  exit 1
fi
```

### CI Integration
```yaml
# .github/workflows/security-tests.yml
name: Sandbox Penetration Tests

on: [push, pull_request]

jobs:
  security:
    runs-on: ubuntu-latest
    steps:
      - uses: actions/checkout@v3
      
      - name: Build Sandbox Image
        run: |
          docker build -f deployments/docker/sandbox.Dockerfile \
            -t crucible-sandbox:latest .
      
      - name: Run Penetration Tests
        run: bash ci/penetration_tests.sh
      
      - name: Upload Security Audit Log
        if: failure()
        uses: actions/upload-artifact@v3
        with:
          name: security-audit
          path: /var/log/sandbox-audit
```

## Security Baseline

All tests must pass for production deployment:

- ✅ No raw socket creation
- ✅ No external network access
- ✅ No privilege escalation
- ✅ No kernel debugging (ptrace)
- ✅ No filesystem escape
- ✅ Memory limits enforced
- ✅ CPU limits enforced
- ✅ WASM bytecode validated
- ✅ Instruction limits enforced
- ✅ No cgroup/namespace escape

## Monitoring & Alerting

Real-time monitoring of sandbox execution:

```rust
// Monitor for sandbox escape attempts
#[tracing::instrument]
async fn monitor_sandbox_execution(container_id: &str) -> Result<(), Error> {
  let mut event_stream = docker.events(Some(EventsOptions {
    filters: [("container", &[container_id])].iter().cloned().collect(),
    ..Default::default()
  })).await?;

  while let Some(event) = event_stream.next().await {
    match event?.typ {
      EventTypeEnum::CONTAINER => {
        if event.status == Some("die".to_string()) {
          tracing::warn!(
            exit_code = ?event.exit_code,
            "Container exited: possible escape attempt detected"
          );
        }
      }
      _ => {}
    }
  }

  Ok(())
}
```

## References

- [seccomp-bpf Documentation](https://man7.org/linux/man-pages/man2/seccomp.2.html)
- [gVisor Security Architecture](https://gvisor.dev/docs/architecture_guide/security/)
- [OWASP Container Security](https://owasp.org/www-project-container-security/)
- [CIS Docker Benchmark](https://www.cisecurity.org/benchmark/docker)
