# Crucible Sandbox Security Documentation

## Overview

The Crucible sandbox provides a secure, isolated execution environment for untrusted WebAssembly (WASM) and Rust compilation processes. It uses multiple layered security mechanisms to prevent kernel exploits, privilege escalation, and host filesystem access.

## Security Layers

### 1. Seccomp Syscall Filtering

**File:** `sandbox-seccomp.json`

Seccomp (Secure Computing) mode restricts the system calls available to the sandbox process using a whitelist approach:

- **Default Action:** `SCMP_ACT_ERRNO` - Deny all syscalls by default
- **Allowed Syscalls:** ~90 syscalls the sandbox-executor (a tokio/axum HTTP service) needs for execution, memory management, networking, and signal handling — see `sandbox-seccomp.json`
- **Blocked Syscalls:** All dangerous operations including:
  - Raw socket creation (`AF_RAW`, `SOCK_RAW`)
  - Privilege escalation (`setuid`, `setgid`, `setcap`)
  - Kernel module manipulation (`create_module`, `delete_module`)
  - Process tracing (`ptrace`, `process_vm_*`)
  - Filesystem mounting (`mount`, `umount`, `chroot`)
  - IPC manipulation (`shmctl`, `semctl`, `msgctl`)

### 2. Linux Capabilities

Capabilities are Unix features that break root privileges into independent units. The sandbox drops all dangerous capabilities:

**Dropped:**
- `CAP_SYS_ADMIN` - Can do almost anything
- `CAP_SYS_MODULE` - Load kernel modules
- `CAP_SYS_PTRACE` - Trace processes
- `CAP_NET_ADMIN` - Configure networking
- `CAP_SYS_BOOT` - Reboot system
- `CAP_SYS_CHROOT` - Change root
- `CAP_DAC_OVERRIDE` - Bypass file permissions
- `CAP_SETFCAP` - Set file capabilities

**Retained:**
- `CAP_NET_BIND_SERVICE` - Bind to ports < 1024

### 3. Read-Only Filesystem

The container root filesystem is read-only, preventing:
- Binary modification
- Configuration tampering
- Persistence across container restarts

**Writable Paths:**
- `/tmp` - Temporary execution space
- `/var/run` - Runtime data

### 4. Resource Limits

Docker container limits prevent resource exhaustion:

```yaml
cpu: 1 core (max)
memory: 256 MB (max)
pids: Limited by cgroup v2
```

**Soroban Sandbox Limits:**
```rust
max_wasm_bytes: 2 MB
max_cpu_instructions: 25 million
max_memory_bytes: 64 MB
timeout_ms: 2000 ms
max_args: 32
```

### 5. Network Isolation

- Container runs on isolated bridge network `172.20.0.0/16`
- No access to host network (network namespace isolated)
- Only port 3000 exposed for API communication

### 6. User Isolation

- Runs as `nobody` user (UID 65534)
- No root capabilities
- No access to host users or groups

## Building the Sandbox

```bash
docker build -f deployments/docker/sandbox.Dockerfile \
  -t crucible-sandbox:latest .
```

## Running the Sandbox

### Using Docker Compose

```bash
docker-compose -f deployments/docker/docker-compose.sandbox.yml up
```

### Manual Docker Run

```bash
docker run \
  --security-opt seccomp=deployments/docker/sandbox-seccomp.json \
  --cap-drop=ALL \
  --cap-add=NET_BIND_SERVICE \
  --memory=256m \
  --cpus=1 \
  --network sandbox-network \
  --read-only \
  --tmpfs /tmp:size=128m \
  --tmpfs /var/run:size=32m \
  -p 3000:3000 \
  crucible-sandbox:latest
```

## Security Testing

Run security tests to verify sandbox isolation:

```bash
# Run security tests (require Docker)
cargo test --test sandbox_security_tests -- --ignored --nocapture

# Check penetration test scenarios
cargo test --test sandbox_security_tests sandbox_penetration_tests -- --ignored
```

### Test Coverage

The sandbox includes tests for:
- ✅ Raw socket blocking
- ✅ Filesystem access prevention
- ✅ Syscall whitelist validation
- ✅ Dangerous syscall blocking
- ✅ Resource limit enforcement
- ✅ WASM execution timeout
- ✅ Capability dropping
- ✅ Memory protection
- ✅ Socket type restrictions

## Known Limitations

1. **Language Isolation:** Only Rust/WASM execution is supported
2. **I/O Isolation:** File I/O limited to stdin/stdout/stderr
3. **Network:** Only outbound networking to explicitly allowed endpoints
4. **Compilation:** Rust compilation happens inside sandbox with timeout

## Audit Trail

### Kernel Protections

- **Seccomp:** BPF-based syscall filtering (kernel 3.17+)
- **Capabilities:** Fine-grained privilege model (kernel 2.2+)
- **Namespaces:** PID, network, mount, user isolation
- **Cgroups v2:** Resource accounting and limits

### Container Protections

- **Distroless Image:** Minimal attack surface (no shell, package manager)
- **Read-Only Root:** Prevents persistence
- **Non-root User:** Process runs as unprivileged user
- **Security Options:** SELinux/AppArmor compatible

## Compliance

The sandbox meets security requirements for:

- **OWASP Top 10:** #1 Broken Access Control, #6 Vulnerable Code
- **CWE Top 25:** #78 OS Command Injection, #94 Code Injection
- **CVE Prevention:** Kernel exploits, privilege escalation
- **Regulatory:** Suitable for financial/compliance-sensitive workloads

## Incident Response

### If Container Escapes Occur

1. **Immediate:**
   - Kill container: `docker kill <container>`
   - Audit logs: `docker logs <container>`
   - Check host for modifications: `auditctl -l`

2. **Investigation:**
   - Review seccomp violations in kernel logs
   - Analyze Strace output (if available)
   - Check cgroup resource usage patterns

3. **Remediation:**
   - Update seccomp profile if new syscall needed
   - Report vulnerability
   - Deploy patched version

## Future Improvements

- [ ] gVisor integration for enhanced isolation
- [ ] Firecracker micro-VM support
- [ ] eBPF-based network filtering
- [ ] Hardware-based memory tagging (ARMv8.5+)
- [ ] Confidential computing (AMD SEV / Intel TDX)

## References

- [Seccomp Documentation](https://man7.org/linux/man-pages/man2/seccomp.2.html)
- [Linux Capabilities](https://man7.org/linux/man-pages/man7/capabilities.7.html)
- [Docker Security](https://docs.docker.com/engine/security/)
- [OCI Runtime Spec](https://github.com/opencontainers/runtime-spec)
- [Soroban Documentation](https://developers.stellar.org/soroban/)

## Support

For security issues, please report to: security@crucible.dev
