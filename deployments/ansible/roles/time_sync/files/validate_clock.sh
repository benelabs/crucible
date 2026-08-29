#!/usr/bin/env bash
#
# Location: deployments/ansible/roles/time_sync/files/validate_clock.sh
# Validation test: confirm the system clock is synchronized and the measured
# offset stays below the allowed threshold (default 10ms). Exits non-zero when
# the clock is not synchronized or the skew is too large, so Ansible (and CI)
# can fail a misconfigured node before it participates in simulations.
#
# Usage:
#   MAX_OFFSET_MS=10 /usr/local/bin/validate_clock.sh

set -euo pipefail

MAX_OFFSET_MS="${MAX_OFFSET_MS:-10}"

if ! command -v chronyc >/dev/null 2>&1; then
  echo "::error:: chronyc not found; is chrony installed?" >&2
  exit 1
fi

TRACKING="$(chronyc -n tracking)" || {
  echo "::error:: failed to query chronyd" >&2
  exit 1
}

# Reject unsynchronized clocks.
if echo "$TRACKING" | grep -qi "Leap status.*Not synchronised"; then
  echo "::error:: clock is NOT synchronized" >&2
  echo "$TRACKING" >&2
  exit 1
fi

# Extract the "Last offset" value (seconds, may be negative).
OFFSET_SEC="$(echo "$TRACKING" | awk -F: '/^Last offset/ {
  gsub(/[ \t]+/, "", $2);
  # strip trailing unit word if present
  sub(/seconds.*/, "", $2);
  print $2
}')"

if [[ -z "$OFFSET_SEC" ]]; then
  echo "::error:: could not parse 'Last offset' from chronyc tracking" >&2
  echo "$TRACKING" >&2
  exit 1
fi

# Absolute value.
ABS_OFFSET_SEC="$(awk -v v="$OFFSET_SEC" 'BEGIN { print (v < 0 ? -v : v) }')"
OFFSET_MS="$(awk -v v="$ABS_OFFSET_SEC" 'BEGIN { printf "%.6f", v * 1000 }')"
MAX_OFFSET_SEC="$(awk -v ms="$MAX_OFFSET_MS" 'BEGIN { print ms / 1000 }')"

echo "Measured clock offset: ${OFFSET_MS} ms (limit: ${MAX_OFFSET_MS} ms)"

# Compare numerically.
WITHIN="$(awk -v off="$ABS_OFFSET_SEC" -v lim="$MAX_OFFSET_SEC" 'BEGIN { print (off <= lim) ? 1 : 0 }')"

if [[ "$WITHIN" != "1" ]]; then
  echo "::error:: clock offset ${OFFSET_MS} ms exceeds allowed ${MAX_OFFSET_MS} ms" >&2
  exit 1
fi

echo "::success:: clock synchronized within tolerance (offset ${OFFSET_MS} ms)"
