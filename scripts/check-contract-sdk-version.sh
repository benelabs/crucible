#!/usr/bin/env bash
set -euo pipefail

root="$(cd "$(dirname "$0")/.." && pwd)"
expected="workspace"
bad=0

while IFS= read -r file; do
  # skip non-contract crates? issue says "all contract crates"
  case "$file" in
    "$root"/contracts/*/Cargo.toml) ;;
    *) continue ;;
  esac
  if grep -q '^soroban-sdk[[:space:]]*=' "$file"; then
    if ! grep -q '^soroban-sdk[[:space:]]*=[[:space:]]*{[[:space:]]*workspace[[:space:]]*=[[:space:]]*true' "$file"; then
      echo "Non-workspace soroban-sdk dependency in $file" >&2
      bad=1
    fi
  fi
done < <(find "$root/contracts" -name Cargo.toml -type f | sort)

if [ "$bad" -ne 0 ]; then
  exit 1
fi
echo "All contract crates use workspace soroban-sdk"
