#!/usr/bin/env bash
# Verify the SQLx strategy documented in backend/README.md:
# queries stay runtime-checked so a clean checkout builds without a database.
# Compile-time query macros are only allowed together with a committed .sqlx cache.
set -euo pipefail

repo_root="$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)"
cd "$repo_root"

if [ -f backend/compile_errors.txt ]; then
    echo "error: backend/compile_errors.txt is a stale diagnostic log; remove it" >&2
    exit 1
fi

if git grep -n -E 'sqlx::(query|query_as|query_scalar)!' -- 'backend/**/*.rs' >/dev/null; then
    if ! compgen -G '.sqlx/query-*.json' >/dev/null; then
        cat >&2 <<'EOF'
error: compile-time SQLx query macros require a committed query cache.

Either keep the query runtime-checked (sqlx::query / query_as / query_scalar),
or run `cargo sqlx prepare` and commit the generated .sqlx directory.
See backend/README.md ("SQLx strategy").
EOF
        exit 1
    fi
fi

# SQLx records one row per migration version in _sqlx_migrations, so two files
# sharing a version make `sqlx migrate run` fail on a fresh database.
duplicate_versions="$(
    find backend/migrations -maxdepth 1 -name '*.sql' -printf '%f\n' \
        | sed -nE 's/^([0-9]+)_.*/\1/p' \
        | sort \
        | uniq -d
)"
if [ -n "$duplicate_versions" ]; then
    echo "error: duplicate SQLx migration version(s): $duplicate_versions" >&2
    exit 1
fi

echo "SQLx strategy OK"
