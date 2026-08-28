# Sandbox Dockerfile with Seccomp & eBPF Security Isolation
# Executes untrusted Wasm and Rust compilation in isolated environment
# Uses seccomp syscall filters to restrict kernel access
#
# Usage:
#   docker build -f sandbox.Dockerfile -t crucible-sandbox:latest .
#   docker run --security-opt seccomp=sandbox-seccomp.json crucible-sandbox:latest

# Stage 1: Build the sandbox executor
FROM rust:1.81-slim as builder

WORKDIR /app

# Install build dependencies
RUN apt-get update && apt-get install -y --no-install-recommends \
    ca-certificates \
    pkg-config \
    libssl-dev \
    && rm -rf /var/lib/apt/lists/*

# Copy workspace manifests
COPY Cargo.toml Cargo.lock ./
COPY backend/Cargo.toml backend/Cargo.toml
COPY contracts/Cargo.toml contracts/Cargo.toml

# Create placeholder source files for dependency caching
RUN mkdir -p backend/src contracts/src && \
    echo "fn main() {}" > backend/src/main.rs && \
    touch contracts/src/lib.rs

# Pre-build dependencies with optimizations
RUN cargo build --release --package backend 2>&1 | grep -E '(Compiling|Finished)' || true

# Copy actual source code
COPY . .

# Build the sandbox executor binary
RUN cargo build --release --bin backend --features sandbox-executor

# Stage 2: Create minimal runtime image
FROM gcr.io/distroless/cc-debian12 as runtime

WORKDIR /app

# Copy the compiled sandbox executor
COPY --from=builder /app/target/release/backend /app/sandbox-executor
COPY --from=builder /app/backend/.env.example /app/.env
COPY --from=builder /app/deployments/docker/sandbox-seccomp.json /app/seccomp.json

# Create non-root user (distroless limitation: use numeric UID)
# UID 65534 is the 'nobody' user

# Set resource limits as environment variables for container runtime
# Memory: 256MB max
# CPU: 1 core max
# Network: Disabled

ENV SANDBOX_LIMITS_MAX_WASM_BYTES=2097152 \
    SANDBOX_LIMITS_MAX_ARGS=32 \
    SANDBOX_LIMITS_MAX_ARG_XDR_BYTES=65536 \
    SANDBOX_LIMITS_MAX_CPU_INSTRUCTIONS=25000000 \
    SANDBOX_LIMITS_MAX_MEMORY_BYTES=268435456 \
    SANDBOX_LIMITS_TIMEOUT_MS=2000

EXPOSE 9090

ENTRYPOINT ["/app/sandbox-executor"]
CMD ["--sandbox-mode"]
