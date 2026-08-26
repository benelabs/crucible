#!/usr/bin/env bash
#
# Location: tests/security/verify_pgp_key.sh
# Automated PGP key verification test.
#
# Ensures the PGP fingerprint documented in SECURITY.md matches the
# canonical public key committed at tests/security/security_contact.pub.asc.
# Exits non-zero (failing the build) if the two drift apart.
#
# Usage:
#   tests/security/verify_pgp_key.sh [path/to/SECURITY.md] [path/to/key.asc]

set -euo pipefail

REPO_ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)"
SECURITY_MD="${1:-$REPO_ROOT/SECURITY.md}"
PUBKEY="${2:-$REPO_ROOT/tests/security/security_contact.pub.asc}"

if ! command -v gpg >/dev/null 2>&1; then
  echo "::error:: gpg is required to verify the PGP key" >&2
  exit 1
fi

if [[ ! -f "$SECURITY_MD" ]]; then
  echo "::error:: SECURITY.md not found at $SECURITY_MD" >&2
  exit 1
fi

if [[ ! -f "$PUBKEY" ]]; then
  echo "::error:: public key not found at $PUBKEY" >&2
  exit 1
fi

# Extract the documented fingerprint from SECURITY.md.
# Expected line format: - **PGP fingerprint:** `6E0D 3546 2433 132B ...`
DOCUMENTED_RAW="$(grep -Eo '`[0-9A-F ]+`' "$SECURITY_MD" \
  | tr -d '`' | head -1 || true)"

if [[ -z "$DOCUMENTED_RAW" ]]; then
  echo "::error:: could not find a PGP fingerprint in $SECURITY_MD" >&2
  exit 1
fi

DOCUMENTED="$(echo "$DOCUMENTED_RAW" | tr -d '[:space:]' | tr '[:lower:]' '[:upper:]')"

# Import the committed key into an isolated keyring and read its fingerprint.
TMP_HOME="$(mktemp -d)"
trap 'rm -rf "$TMP_HOME"' EXIT
export GNUPGHOME="$TMP_HOME"
chmod 700 "$TMP_HOME"

gpg --batch --import "$PUBKEY" >/dev/null 2>&1

ACTUAL_RAW="$(gpg --list-keys --with-colons 2>/dev/null \
  | awk -F: '/^fpr:/{print $10; exit}')"

if [[ -z "$ACTUAL_RAW" ]]; then
  echo "::error:: failed to read fingerprint from $PUBKEY" >&2
  exit 1
fi

ACTUAL="$(echo "$ACTUAL_RAW" | tr -d '[:space:]' | tr '[:lower:]' '[:upper:]')"

echo "Documented fingerprint: $DOCUMENTED"
echo "Actual key fingerprint: $ACTUAL"

if [[ "$DOCUMENTED" != "$ACTUAL" ]]; then
  echo "::error:: PGP fingerprint mismatch! SECURITY.md is out of sync with the published key." >&2
  exit 1
fi

echo "::success:: PGP fingerprint verified: $ACTUAL"
