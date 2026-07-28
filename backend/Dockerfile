# Stage 1: Plan
FROM lukemathwalker/cargo-chef:latest AS chef
WORKDIR /app

FROM chef AS planner
COPY . .
RUN cargo chef prepare --recipe-path recipe.json

# Stage 2: Build
FROM chef AS builder
COPY --from=planner /app/recipe.json recipe.json
RUN CARGO_BUILD_JOBS=2 cargo chef cook --release --recipe-path recipe.json

COPY . .
RUN CARGO_BUILD_JOBS=2 cargo build --release --bin backend

# Stage 3: Runtime
FROM gcr.io/distroless/cc-debian12 AS runtime
WORKDIR /app
COPY --from=builder /app/target/release/backend /app/backend
COPY --from=builder /app/backend/migrations /app/migrations
COPY --from=builder /app/backend/.env.example /app/.env

EXPOSE 3000
ENTRYPOINT ["./backend"]
