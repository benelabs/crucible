.PHONY: test lint doc build build-contracts build-backend run-backend db-migrate clean-test-env

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

# Tear down local integration-test Docker services and delete ephemeral volumes
# (e.g. crucible-postgres-data) so dangling test DBs do not accumulate (#1038).
clean-test-env:
	@if [ -x backend/scripts/clean-test-env.sh ]; then \
		backend/scripts/clean-test-env.sh; \
	else \
		docker compose -f backend/docker-compose.yml down -v --remove-orphans; \
	fi

doc:
	cargo doc --workspace --no-deps --all-features --open