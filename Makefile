.PHONY: test lint doc build build-contracts build-backend run-backend db-migrate

test:
	cargo test --workspace --all-features

lint:
	cargo clippy --workspace -- -D warnings
	cargo fmt --all --check

build: build-contracts build-backend

build-contracts:
	cargo build --package crucible-macros
	cargo build --package crucible

build-backend:
	cargo build --package backend

run-backend:
	cargo run --package backend

db-migrate:
	sqlx migrate run --source backend/migrations

doc:
	cargo doc --workspace --no-deps --all-features --open