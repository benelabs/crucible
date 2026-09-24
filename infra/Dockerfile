# Multi-architecture Dockerfile for the Crucible backend.
# Supports linux/amd64 and linux/arm64 via Docker Buildx.
#
# Usage:
#   docker buildx build --platform linux/amd64,linux/arm64 -t crucible:latest .

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

# Stage 4: Runtime image
FROM gcr.io/distroless/cc-debian12 AS runtime
WORKDIR /app
COPY --from=builder /app/target/release/backend /app/backend
COPY --from=builder /app/backend/migrations /app/migrations
COPY --from=builder /app/backend/.env.example /app/.env

EXPOSE 3000
ENTRYPOINT ["./backend"]
