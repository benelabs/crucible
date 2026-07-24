#!/usr/bin/env bash
set -euo pipefail

# Automated Zero-Downtime Migration Safety Checker
MIGRATIONS_DIR="${1:-backend/migrations}"

echo "=== Checking SQL migrations in $MIGRATIONS_DIR for breaking DDL ==="

UNSAFE_FOUND=0

for sql_file in "$MIGRATIONS_DIR"/*.sql; do
    [[ -e "$sql_file" ]] || continue

    # Check for DROP COLUMN without ZERO_DOWNTIME_CONTRACT flag
    if grep -iq "DROP COLUMN" "$sql_file" && ! grep -iq "CONTRACT" "$sql_file"; then
        echo "❌ [FAIL] Unsafe DROP COLUMN detected in $sql_file without Phase 2 contract annotation!"
        UNSAFE_FOUND=1
    fi

    # Check for RENAME COLUMN (breaks running API instances)
    if grep -iq "RENAME COLUMN" "$sql_file"; then
        echo "❌ [FAIL] Unsafe RENAME COLUMN detected in $sql_file. Use Expand & Contract pattern instead!"
        UNSAFE_FOUND=1
    fi

    # Check for ADD COLUMN NOT NULL without DEFAULT
    if grep -iE "ADD COLUMN.*NOT NULL" "$sql_file" | grep -ivq "DEFAULT"; then
        echo "❌ [FAIL] Unsafe ADD COLUMN NOT NULL without DEFAULT detected in $sql_file!"
        UNSAFE_FOUND=1
    fi
done

if [[ $UNSAFE_FOUND -eq 1 ]]; then
    echo "=========================================================="
    echo "Migration safety check FAILED! Breaking DDL detected."
    echo "Please refer to backend/migrations/ZERO_DOWNTIME_GUIDE.md"
    echo "=========================================================="
    exit 1
else
    echo "✅ [SUCCESS] All SQL migration files pass zero-downtime safety checks!"
    exit 0
fi
