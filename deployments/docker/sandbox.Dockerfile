# Sandbox Dockerfile with Seccomp Security Isolation
#
# Runs the backend API — which owns the /api/v1/sandbox/execute endpoint used
# to execute untrusted contract WASM, see backend/src/services/sandbox.rs —
# inside a locked-down container: non-root, read-only root filesystem, all
# Linux capabilities dropped except NET_BIND_SERVICE (so CAP_NET_RAW is
# unavailable, which is what the kernel actually checks when a raw socket is
# requested), and a deny-by-default seccomp profile (sandbox-seccomp.json).
#
# See docker-compose.sandbox.yml for the full runtime hardening (cap_drop,
# read_only, resource limits) that pairs with this image, and
# deployments/docker/SANDBOX_SECURITY.md for the threat model and its
# documented limitations.
#
# Usage:
#   docker compose -f deployments/docker/docker-compose.sandbox.yml up --build

# Stage 1: Build the backend binary
FROM rust:1.91-slim AS builder

WORKDIR /app

RUN apt-get update && apt-get install -y --no-install-recommends \
    ca-certificates \
    pkg-config \
    libssl-dev \
    && rm -rf /var/lib/apt/lists/*

# The backend crate is a workspace member; its dependency graph
# (crucible-macros, contracts/*) is resolved via the shared Cargo.lock, so a
# partial copy of just backend/ would not produce a reproducible build.
COPY . .

RUN cargo build --release --package backend --bin backend

# Stage 2: Minimal, non-root runtime image
FROM gcr.io/distroless/cc-debian12:nonroot AS runtime

WORKDIR /app

COPY --from=builder /app/target/release/backend /app/backend
COPY --from=builder /app/deployments/docker/sandbox-seccomp.json /app/seccomp.json

# Resource limits enforced by ContractSandboxService (backend/src/services/sandbox.rs)
ENV SANDBOX_LIMITS_MAX_WASM_BYTES=2097152 \
    SANDBOX_LIMITS_MAX_ARGS=32 \
    SANDBOX_LIMITS_MAX_ARG_XDR_BYTES=65536 \
    SANDBOX_LIMITS_MAX_CPU_INSTRUCTIONS=25000000 \
    SANDBOX_LIMITS_MAX_MEMORY_BYTES=268435456 \
    SANDBOX_LIMITS_TIMEOUT_MS=2000

EXPOSE 3000

# distroless:nonroot already runs as an unprivileged UID; no USER directive needed.
ENTRYPOINT ["/app/backend"]

# ============================================================================
# THREAT MODEL & KNOWN LIMITATIONS — see SANDBOX_SECURITY.md for detail
# ============================================================================
#
# Mitigated by this configuration:
#   - Raw/packet socket creation: blocked by dropping CAP_NET_RAW (cap_drop:
#     ALL in docker-compose.sandbox.yml), which the kernel enforces on
#     socket(..., SOCK_RAW/SOCK_PACKET, ...) regardless of seccomp.
#   - Host filesystem access: read_only root filesystem + minimal distroless
#     image (no shell, no package manager) + only /tmp and /var/run writable.
#   - Privilege escalation / kernel tampering: seccomp denies ptrace, mount,
#     setns, unshare, the setuid/setgid family, module loading, and kexec by
#     default (sandbox-seccomp.json is deny-by-default; only an explicit
#     allow-list of syscalls the service needs is permitted).
#
# NOT mitigated by this configuration (documented, not silently ignored):
#   - This image runs the same trust boundary as the rest of the backend
#     process — it isolates the process from the host, not WASM guest code
#     from the host Rust process. WASM sandboxing itself is the
#     responsibility of the Soroban WASM interpreter in-process.
#   - True kernel-level isolation (gVisor user-space kernel, Firecracker
#     micro-VMs) is not wired up here. It requires a compatible container
#     runtime on the host (runsc/firecracker-containerd) that this repo does
#     not currently provision; tracked as a follow-up, not claimed as done.
#   - Side-channel attacks (Spectre/Meltdown class) are out of scope for
#     container-level isolation entirely.
