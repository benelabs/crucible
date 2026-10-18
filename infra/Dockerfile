# Multi-architecture Dockerfile for the Crucible backend.
# Supports linux/amd64 and linux/arm64 via Docker Buildx.
#
# Usage:
#   docker buildx build --platform linux/amd64,linux/arm64 -t crucible:latest .
#
# Runtime hardening:
#   - Distroless nonroot user (UID 65532)
#   - Intended to run with readOnlyRootFilesystem: true
#   - Scratch / temporary writes must go to /tmp (mount as tmpfs)

# Stage 1: Dependency planning
FROM lukemathwalker/cargo-chef:latest AS chef
WORKDIR /app

FROM chef AS planner
COPY . .
RUN cargo chef prepare --bin backend --recipe-path recipe.json

# Stage 2: Build dependencies (cached layer)
FROM chef AS builder
COPY --from=planner /app/recipe.json recipe.json
RUN CARGO_BUILD_JOBS=2 cargo chef cook --release --bin backend --recipe-path recipe.json

# Stage 3: Full build
COPY . .
RUN CARGO_BUILD_JOBS=2 cargo build --release --bin backend

# Stage 4: Runtime image (read-only root filesystem ready)
FROM gcr.io/distroless/cc-debian12:nonroot AS runtime
WORKDIR /app

# Ensure /tmp exists for scratch files when the root filesystem is mounted read-only
# and /tmp is provided as a writable tmpfs / emptyDir volume at deploy time.
COPY --from=builder --chown=nonroot:nonroot /app/target/release/backend /app/backend
COPY --from=builder --chown=nonroot:nonroot /app/backend/migrations /app/migrations

USER nonroot:nonroot

EXPOSE 3000
ENTRYPOINT ["/app/backend"]
