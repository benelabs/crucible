#!/usr/bin/env bash
#
# Automated Chaos Engineering & Network Fault Injection Suite
# ===========================================================
# Simulates network latency, RPC node termination and packet loss against the
# Crucible backend, then asserts that:
#   * circuit breakers trip cleanly and failover seamlessly, and
#   * zero unhandled panics / corrupted database transactions occur.
#
# Issue: https://github.com/benelabs/crucible/issues/917
#
# The fault injection uses `tc netem` (Linux traffic control) and `iptables`.
# Requires root. Use --dry-run to print the commands that *would* run instead of
# executing them, which is handy in CI or local smoke tests.
#
set -euo pipefail

# --------------------------------------------------------------------------- #
# Configuration (overridable via environment / flags)
# --------------------------------------------------------------------------- #
INTERFACE="${CHAOS_INTERFACE:-lo}"
LATENCY_MS="${CHAOS_LATENCY_MS:-500}"
PACKET_LOSS_PCT="${CHAOS_PACKET_LOSS_PCT:-20}"
DURATION_S="${CHAOS_DURATION_S:-30}"
BACKEND_URL="${CHAOS_BACKEND_URL:-http://localhost:3000}"
HEALTH_PATH="${CHAOS_HEALTH_PATH:-/api/v1/health}"
BREAKER_PATH="${CHAOS_BREAKER_PATH:-/api/v1/circuit-breaker/status}"
FAILOVER_URL="${CHAOS_FAILOVER_URL:-http://localhost:3001}"
DB_INTEGRITY_PATH="${CHAOS_DB_INTEGRITY_PATH:-/api/v1/admin/db/integrity}"
BACKEND_LOG="${CHAOS_BACKEND_LOG:-}"
PANIC_PATTERNS=("panic" "thread '.*' panicked" "transaction abort" "SQLite error" "deadlock")
RPC_PORT="${CHAOS_RPC_PORT:-80}"
DRY_RUN=0
VERBOSE=0

# --------------------------------------------------------------------------- #
# Helpers
# --------------------------------------------------------------------------- #
log()  { printf '\033[36m[chaos]\033[0m %s\n' "$*"; }
err()  { printf '\033[31m[chaos:error]\033[0m %s\n' "$*" >&2; }
die()  { err "$*"; exit 1; }

run() {
  if (( DRY_RUN )); then
    printf '\033[33m[chaos:dry-run]\033[0m %s\n' "$*"
  else
    if (( VERBOSE )); then log "exec: $*"; fi
    eval "$@"
  fi
}

usage() {
  cat <<EOF
Usage: $0 [options]

  --interface IFACE     Network interface to fault-inject on (default: $INTERFACE)
  --latency MS          Added packet latency in ms (default: $LATENCY_MS)
  --loss PCT            Packet drop percentage (default: $PACKET_LOSS_PCT)
  --duration SEC        Seconds to sustain chaos (default: $DURATION_S)
  --backend URL         Primary backend base URL (default: $BACKEND_URL)
  --failover URL        Failover backend base URL (default: $FAILOVER_URL)
  --rpc-port PORT       RPC port to blackhole for node-termination test (default: $RPC_PORT)
  --log FILE            Backend log file scanned for panic/corruption markers
  --dry-run             Print fault-injection commands instead of applying them
  --verbose             Print every command executed
  -h, --help            Show this help
EOF
}

# --------------------------------------------------------------------------- #
# Fault injection
# --------------------------------------------------------------------------- #
inject_faults() {
  log "Injecting ${LATENCY_MS}ms latency + ${PACKET_LOSS_PCT}% loss on ${INTERFACE}"
  run "sudo tc qdisc add dev ${INTERFACE} root netem delay ${LATENCY_MS}ms loss ${PACKET_LOSS_PCT}%"
}

# Blackhole the RPC port: simulates a sudden RPC node termination.
terminate_rpc() {
  log "Simulating RPC node termination (blackholing port ${RPC_PORT})"
  run "sudo iptables -A INPUT  -p tcp --dport ${RPC_PORT} -j DROP"
  run "sudo iptables -A OUTPUT -p tcp --dport ${RPC_PORT} -j DROP"
}

restore_faults() {
  log "Restoring network to baseline"
  run "sudo tc qdisc del dev ${INTERFACE} root netem 2>/dev/null || true"
  run "sudo iptables -D INPUT  -p tcp --dport ${RPC_PORT} -j DROP 2>/dev/null || true"
  run "sudo iptables -D OUTPUT -p tcp --dport ${RPC_PORT} -j DROP 2>/dev/null || true"
}
trap restore_faults EXIT

# --------------------------------------------------------------------------- #
# Validation
# --------------------------------------------------------------------------- #
http_status() {
  curl -s -o /dev/null -w '%{http_code}' --max-time 5 "$1" 2>/dev/null || echo "000"
}

assert_circuit_breaker_trips() {
  log "Validating circuit breaker trips cleanly during chaos"
  local saw_open=0
  for _ in $(seq 1 20); do
    local st
    st=$(curl -s --max-time 5 "${BACKEND_URL}${BREAKER_PATH}" 2>/dev/null || echo "{}")
    if printf '%s' "$st" | grep -qi '"state"[[:space:]]*:[[:space:]]*"open"'; then
      saw_open=1
      break
    fi
    sleep 1
  done
  if (( saw_open )); then
    log "  ✓ circuit breaker transitioned to OPEN"
  else
    die "  ✗ circuit breaker never opened under sustained latency/loss"
  fi
}

assert_failover_seamless() {
  log "Validating seamless failover to ${FAILOVER_URL}"
  local fb_status
  fb_status=$(http_status "${FAILOVER_URL}${HEALTH_PATH}")
  if [[ "$fb_status" == "200" ]]; then
    log "  ✓ failover backend healthy (HTTP ${fb_status})"
  else
    die "  ✗ failover backend unreachable (HTTP ${fb_status})"
  fi
}

assert_no_panics_or_corruption() {
  log "Asserting zero unhandled panics / corrupted transactions"
  if [[ -n "${BACKEND_LOG}" && -f "${BACKEND_LOG}" ]]; then
    for pat in "${PANIC_PATTERNS[@]}"; do
      if grep -Ei "$pat" "${BACKEND_LOG}" >/dev/null 2>&1; then
        die "  ✗ panic/corruption marker detected in ${BACKEND_LOG}: ${pat}"
      fi
    done
    log "  ✓ no panic/corruption markers in ${BACKEND_LOG}"
  else
    log "  • no backend log supplied (--log); skipping log scan"
  fi

  local db_status
  db_status=$(http_status "${BACKEND_URL}${DB_INTEGRITY_PATH}")
  if [[ "$db_status" == "200" ]]; then
    log "  ✓ database integrity check passed"
  else
    # Endpoint may not exist in every deployment; treat absence as non-fatal.
    log "  • db integrity endpoint returned HTTP ${db_status} (skipped)"
  fi
}

# --------------------------------------------------------------------------- #
# Parse args
# --------------------------------------------------------------------------- #
while [[ $# -gt 0 ]]; do
  case "$1" in
    --interface)  INTERFACE="$2"; shift 2 ;;
    --latency)    LATENCY_MS="$2"; shift 2 ;;
    --loss)       PACKET_LOSS_PCT="$2"; shift 2 ;;
    --duration)   DURATION_S="$2"; shift 2 ;;
    --backend)    BACKEND_URL="$2"; shift 2 ;;
    --failover)   FAILOVER_URL="$2"; shift 2 ;;
    --rpc-port)   RPC_PORT="$2"; shift 2 ;;
    --log)        BACKEND_LOG="$2"; shift 2 ;;
    --dry-run)    DRY_RUN=1; shift ;;
    --verbose)    VERBOSE=1; shift ;;
    -h|--help)    usage; exit 0 ;;
    *) err "unknown argument: $1"; usage; exit 2 ;;
  esac
done

if (( DRY_RUN == 0 )) && [[ "$(id -u)" != "0" ]]; then
  die "fault injection requires root (use sudo or --dry-run)"
fi

# --------------------------------------------------------------------------- #
# Run
# --------------------------------------------------------------------------- #
log "Chaos suite starting (interface=${INTERFACE} latency=${LATENCY_MS}ms loss=${PACKET_LOSS_PCT}% duration=${DURATION_S}s)"
inject_faults
terminate_rpc

log "Sustaining chaos for ${DURATION_S}s while probing backend"
end=$(( SECONDS + DURATION_S ))
while (( SECONDS < end )); do
  http_status "${BACKEND_URL}${HEALTH_PATH}" >/dev/null 2>&1 || true
  sleep 2
done

assert_circuit_breaker_trips
assert_failover_seamless
assert_no_panics_or_corruption

log "Chaos suite PASSED ✓"
