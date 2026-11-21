#!/usr/bin/env bash
# Lightweight documentation hygiene guard.
#
# Fails when tracked documentation contains placeholder repository URLs,
# machine-specific absolute paths, or when the README badges stop pointing at
# the canonical repository's workflows. Safe to run from any directory.

set -euo pipefail

repo_root="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
cd "$repo_root"

readonly REPO_SLUG="benelabs/crucible"

# Literal strings that must never appear in tracked documentation.
readonly -a FORBIDDEN_PATTERNS=(
  'your-org/crucible'
  'anumukul/crucible'
  'somemorewater/crucible'
  'file:///'
  '/Users/'
  '/home/'
  'C:\Users\'
)

# README status badges that must resolve to a workflow in this repository.
readonly -a EXPECTED_BADGES=(
  "https://github.com/${REPO_SLUG}/actions/workflows/ci.yml/badge.svg"
  "https://github.com/${REPO_SLUG}/actions/workflows/backend-ci.yml/badge.svg"
  "https://github.com/${REPO_SLUG}/actions/workflows/security.yml/badge.svg"
)

doc_files=()
while IFS= read -r -d '' file; do
  doc_files+=("$file")
done < <(
  git ls-files -z -- \
    '*.md' \
    'CONTRIBUTING.md' \
    'README.md' \
    'docs/**' \
    '.github/ISSUE_TEMPLATE/**'
)

if (( ${#doc_files[@]} == 0 )); then
  echo 'docs: no tracked documentation files found' >&2
  exit 1
fi

status=0

for pattern in "${FORBIDDEN_PATTERNS[@]}"; do
  while IFS= read -r match; do
    printf 'docs: forbidden %q in %s\n' "$pattern" "$match" >&2
    status=1
  done < <(grep -nH -F -e "$pattern" "${doc_files[@]}" || true)
done

for badge in "${EXPECTED_BADGES[@]}"; do
  if ! grep -Fq "$badge" README.md; then
    printf 'docs: README.md is missing badge %s\n' "$badge" >&2
    status=1
  fi
done

for doc in README.md CONTRIBUTING.md; do
  if ! grep -Fq "https://github.com/${REPO_SLUG}" "$doc"; then
    printf 'docs: %s does not reference %s\n' "$doc" "$REPO_SLUG" >&2
    status=1
  fi
done

if (( status != 0 )); then
  echo 'docs check failed; fix the findings above before merging.' >&2
  exit 1
fi

printf 'docs check passed (%d files scanned).\n' "${#doc_files[@]}"
