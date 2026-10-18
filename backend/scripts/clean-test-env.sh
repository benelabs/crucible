#!/usr/bin/env bash
# Remove local integration-test Docker services and their ephemeral volumes
# (crucible-postgres-data, crucible-redis-data, etc.) so disk is not left
# consumed after test runs. See issue #1038 / `make clean-test-env`.
set -euo pipefail

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
cd "$ROOT"

echo "Stopping backend compose stack and deleting volumes..."
docker compose -f docker-compose.yml down -v --remove-orphans
echo "Clean test env complete."
