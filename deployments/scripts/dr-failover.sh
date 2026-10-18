#!/usr/bin/env bash
# Location: deployments/scripts/dr-failover.sh
# Production requirement: Disaster Recovery (DR) Multi-Region Failover Plan & Drill Harness
#
# Promotes a secondary-region PostgreSQL read-replica to primary and diverts
# DNS (Route53 and/or Cloudflare) so API traffic follows the new writer.
# Target RTO: < 60 seconds.
#
# Usage:
#   deployments/scripts/dr-failover.sh [--mode mock|live] [--drill] [--force]
#                                      [--provider route53|cloudflare|all]
#                                      [--engine rds|aurora]
#
# Environment (live mode):
#   DR_PRIMARY_REGION          AWS region of the current writer (default: us-east-1)
#   DR_SECONDARY_REGION        AWS region of the replica         (default: us-west-2)
#   DR_RDS_REPLICA_ID          RDS read-replica instance id to promote
#   DR_RDS_CLUSTER_ID          Aurora cluster id (when --engine aurora)
#   DR_HEALTH_URL              Primary health endpoint (default: https://api.crucible.dev/health/ready)
#   DR_SECONDARY_HEALTH_URL    Secondary health endpoint
#   DR_SECONDARY_IP            Public IPv4 of the secondary origin
#   DR_DNS_NAME                FQDN to fail over (default: api.crucible.dev)
#   DR_ROUTE53_ZONE_ID         Route53 hosted zone id
#   DR_ROUTE53_RECORD_SET_ID   Failover set identifier for PRIMARY
#   CLOUDFLARE_ZONE_ID         Cloudflare zone id
#   CLOUDFLARE_API_TOKEN       Cloudflare API token (DNS + LB edit)
#   CLOUDFLARE_DNS_RECORD_ID   A/AAAA record id to retarget
#   CLOUDFLARE_LB_ID           Optional load-balancer id (pool steering)
#   CLOUDFLARE_SECONDARY_POOL  Optional origin pool id to mark healthy/enabled
#   AWS_PROFILE / AWS credentials as usual for the AWS CLI
set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
STATE_DIR="${DR_STATE_DIR:-${TMPDIR:-/tmp}/crucible-dr-$$}"
MODE="live"
DRILL=0
FORCE=0
PROVIDER="all"
ENGINE="rds"
RTO_BUDGET_MS=60000

PRIMARY_REGION="${DR_PRIMARY_REGION:-us-east-1}"
SECONDARY_REGION="${DR_SECONDARY_REGION:-us-west-2}"
HEALTH_URL="${DR_HEALTH_URL:-https://api.crucible.dev/health/ready}"
SECONDARY_HEALTH_URL="${DR_SECONDARY_HEALTH_URL:-https://api-standby.crucible.dev/health/ready}"
DNS_NAME="${DR_DNS_NAME:-api.crucible.dev}"
SECONDARY_IP="${DR_SECONDARY_IP:-}"

log() { printf '[dr-failover] %s %s\n' "$(date -u +%Y-%m-%dT%H:%M:%SZ)" "$*"; }
die() { log "ERROR: $*"; exit 1; }

now_ms() {
  # GNU date (CI / Linux). Fall back to second precision.
  if date +%s%N >/dev/null 2>&1; then
    local ns
    ns="$(date +%s%N)"
    printf '%s' "$((ns / 1000000))"
  else
    printf '%s' "$(($(date +%s) * 1000))"
  fi
}

usage() {
  sed -n '2,28p' "$0" | sed 's/^# \?//'
}

while [[ $# -gt 0 ]]; do
  case "$1" in
    --mode) MODE="${2:-}"; shift 2 ;;
    --drill) DRILL=1; MODE="mock"; shift ;;
    --force) FORCE=1; shift ;;
    --provider) PROVIDER="${2:-}"; shift 2 ;;
    --engine) ENGINE="${2:-}"; shift 2 ;;
    -h|--help) usage; exit 0 ;;
    *) die "unknown argument: $1" ;;
  esac
done

[[ "$MODE" == "mock" || "$MODE" == "live" ]] || die "mode must be mock|live"
[[ "$PROVIDER" == "route53" || "$PROVIDER" == "cloudflare" || "$PROVIDER" == "all" ]] || die "provider must be route53|cloudflare|all"
[[ "$ENGINE" == "rds" || "$ENGINE" == "aurora" ]] || die "engine must be rds|aurora"

mkdir -p "$STATE_DIR"
cleanup() {
  if [[ "$MODE" == "mock" && -z "${DR_KEEP_STATE:-}" ]]; then
    rm -rf "$STATE_DIR"
  fi
}
trap cleanup EXIT

# ---------------------------------------------------------------------------
# Health
# ---------------------------------------------------------------------------
primary_is_healthy() {
  if [[ "$MODE" == "mock" ]]; then
    [[ "$(cat "$STATE_DIR/primary_health" 2>/dev/null || echo down)" == "up" ]]
    return
  fi
  curl -fsS --max-time 3 "$HEALTH_URL" >/dev/null 2>&1
}

wait_for_secondary_health() {
  local deadline=$(( $(now_ms) + 45000 ))
  if [[ "$MODE" == "mock" ]]; then
    echo "up" > "$STATE_DIR/secondary_health"
    log "mock secondary region ${SECONDARY_REGION} accepting traffic"
    return 0
  fi
  local url="${SECONDARY_HEALTH_URL}"
  while (( $(now_ms) < deadline )); do
    if curl -fsS --max-time 2 "$url" >/dev/null 2>&1; then
      log "secondary health check passed: $url"
      return 0
    fi
    sleep 1
  done
  log "WARNING: secondary health URL did not pass within 45s (continuing; DNS may still converge)"
  return 0
}

# ---------------------------------------------------------------------------
# PostgreSQL replica promotion
# ---------------------------------------------------------------------------
promote_replica_mock() {
  log "mock: promoting ${SECONDARY_REGION} read-replica to primary"
  echo "replica" > "$STATE_DIR/role_${PRIMARY_REGION}"
  echo "promoting" > "$STATE_DIR/role_${SECONDARY_REGION}"
  # Simulate WAL replay / promotion fence (sub-second in mock, still exercises the path).
  sleep 0.2
  echo "failed" > "$STATE_DIR/role_${PRIMARY_REGION}"
  echo "primary" > "$STATE_DIR/role_${SECONDARY_REGION}"
  echo "${SECONDARY_REGION}" > "$STATE_DIR/writer_region"
  log "mock: ${SECONDARY_REGION} is now the writer"
}

promote_replica_rds() {
  local replica_id="${DR_RDS_REPLICA_ID:-}"
  [[ -n "$replica_id" ]] || die "DR_RDS_REPLICA_ID is required for live RDS promotion"
  log "promoting RDS read-replica ${replica_id} in ${SECONDARY_REGION}"
  aws rds promote-read-replica \
    --region "$SECONDARY_REGION" \
    --db-instance-identifier "$replica_id" >/dev/null
  aws rds wait db-instance-available \
    --region "$SECONDARY_REGION" \
    --db-instance-identifier "$replica_id"
  log "RDS replica ${replica_id} is available as a standalone writer"
}

promote_replica_aurora() {
  local cluster_id="${DR_RDS_CLUSTER_ID:-}"
  [[ -n "$cluster_id" ]] || die "DR_RDS_CLUSTER_ID is required for live Aurora failover"
  log "failing Aurora cluster ${cluster_id} over to ${SECONDARY_REGION}"
  aws rds failover-db-cluster \
    --region "$PRIMARY_REGION" \
    --db-cluster-identifier "$cluster_id" >/dev/null
  aws rds wait db-cluster-available \
    --region "$PRIMARY_REGION" \
    --db-cluster-identifier "$cluster_id"
  log "Aurora cluster ${cluster_id} failover completed"
}

promote_replica() {
  if [[ "$MODE" == "mock" ]]; then
    promote_replica_mock
    return
  fi
  command -v aws >/dev/null 2>&1 || die "aws CLI is required in live mode"
  case "$ENGINE" in
    rds) promote_replica_rds ;;
    aurora) promote_replica_aurora ;;
  esac
}

# ---------------------------------------------------------------------------
# DNS failover — Route53
# ---------------------------------------------------------------------------
failover_route53_mock() {
  log "mock Route53: PRIMARY ${PRIMARY_REGION} unhealthy, SECONDARY ${SECONDARY_REGION} active"
  cat > "$STATE_DIR/route53.json" <<EOF
{"name":"${DNS_NAME}","type":"A","failover":"SECONDARY","region":"${SECONDARY_REGION}","health":"ok"}
EOF
}

failover_route53_live() {
  local zone_id="${DR_ROUTE53_ZONE_ID:-}"
  [[ -n "$zone_id" ]] || die "DR_ROUTE53_ZONE_ID is required for Route53 failover"
  [[ -n "$SECONDARY_IP" ]] || die "DR_SECONDARY_IP is required for Route53 failover"
  log "Route53: upserting ${DNS_NAME} -> ${SECONDARY_IP} (failover SECONDARY)"
  local batch
  batch="$(mktemp)"
  cat > "$batch" <<EOF
{
  "Comment": "Crucible DR failover $(date -u +%FT%TZ)",
  "Changes": [{
    "Action": "UPSERT",
    "ResourceRecordSet": {
      "Name": "${DNS_NAME}",
      "Type": "A",
      "TTL": 30,
      "SetIdentifier": "crucible-secondary",
      "Failover": "SECONDARY",
      "ResourceRecords": [{"Value": "${SECONDARY_IP}"}]
    }
  }]
}
EOF
  aws route53 change-resource-record-sets \
    --hosted-zone-id "$zone_id" \
    --change-batch "file://${batch}" >/dev/null
  rm -f "$batch"
}

failover_route53() {
  if [[ "$PROVIDER" != "route53" && "$PROVIDER" != "all" ]]; then
    return
  fi
  if [[ "$MODE" == "mock" ]]; then
    failover_route53_mock
  else
    failover_route53_live
  fi
}

# ---------------------------------------------------------------------------
# DNS failover — Cloudflare
# ---------------------------------------------------------------------------
failover_cloudflare_mock() {
  log "mock Cloudflare: steering ${DNS_NAME} to ${SECONDARY_REGION}"
  cat > "$STATE_DIR/cloudflare.json" <<EOF
{"name":"${DNS_NAME}","proxied":true,"region":"${SECONDARY_REGION}","origin":"${SECONDARY_IP:-203.0.113.20}"}
EOF
}

failover_cloudflare_live() {
  local zone_id="${CLOUDFLARE_ZONE_ID:-}"
  local token="${CLOUDFLARE_API_TOKEN:-}"
  local record_id="${CLOUDFLARE_DNS_RECORD_ID:-}"
  [[ -n "$zone_id" && -n "$token" ]] || die "CLOUDFLARE_ZONE_ID and CLOUDFLARE_API_TOKEN are required"
  [[ -n "$SECONDARY_IP" ]] || die "DR_SECONDARY_IP is required for Cloudflare failover"

  if [[ -n "$record_id" ]]; then
    log "Cloudflare: PATCH DNS record ${record_id} -> ${SECONDARY_IP}"
    curl -fsS -X PATCH \
      "https://api.cloudflare.com/client/v4/zones/${zone_id}/dns_records/${record_id}" \
      -H "Authorization: Bearer ${token}" \
      -H "Content-Type: application/json" \
      --data "{\"content\":\"${SECONDARY_IP}\",\"ttl\":60,\"proxied\":true}" >/dev/null
  fi

  if [[ -n "${CLOUDFLARE_LB_ID:-}" && -n "${CLOUDFLARE_SECONDARY_POOL:-}" ]]; then
    log "Cloudflare: enabling secondary origin pool ${CLOUDFLARE_SECONDARY_POOL}"
    curl -fsS -X PATCH \
      "https://api.cloudflare.com/client/v4/zones/${zone_id}/load_balancers/${CLOUDFLARE_LB_ID}" \
      -H "Authorization: Bearer ${token}" \
      -H "Content-Type: application/json" \
      --data "{\"default_pools\":[\"${CLOUDFLARE_SECONDARY_POOL}\"]}" >/dev/null
  fi
}

failover_cloudflare() {
  if [[ "$PROVIDER" != "cloudflare" && "$PROVIDER" != "all" ]]; then
    return
  fi
  if [[ "$MODE" == "mock" ]]; then
    failover_cloudflare_mock
  else
    failover_cloudflare_live
  fi
}

# ---------------------------------------------------------------------------
# Orchestration
# ---------------------------------------------------------------------------
run_failover() {
  log "mode=${MODE} provider=${PROVIDER} engine=${ENGINE} primary=${PRIMARY_REGION} secondary=${SECONDARY_REGION}"

  if [[ "$FORCE" -eq 0 ]] && primary_is_healthy; then
    die "primary still healthy at ${HEALTH_URL}; pass --force to fail over anyway"
  fi
  log "primary declared unhealthy (or --force); beginning failover"

  promote_replica
  failover_route53
  failover_cloudflare
  wait_for_secondary_health

  echo "${SECONDARY_REGION}" > "$STATE_DIR/active_region"
  log "failover complete; writer=${SECONDARY_REGION} dns=${DNS_NAME}"
}

run_drill() {
  log "starting mock DR drill (RTO budget ${RTO_BUDGET_MS}ms)"
  mkdir -p "$STATE_DIR"
  echo "up" > "$STATE_DIR/primary_health"
  echo "replica" > "$STATE_DIR/role_${PRIMARY_REGION}"
  echo "replica" > "$STATE_DIR/role_${SECONDARY_REGION}"
  echo "${PRIMARY_REGION}" > "$STATE_DIR/writer_region"

  # Simulate datacenter outage.
  echo "down" > "$STATE_DIR/primary_health"
  FORCE=1
  MODE="mock"

  local start end elapsed
  start="$(now_ms)"
  run_failover
  end="$(now_ms)"
  elapsed=$((end - start))

  local writer
  writer="$(cat "$STATE_DIR/writer_region")"
  [[ "$writer" == "$SECONDARY_REGION" ]] || die "drill: writer is ${writer}, expected ${SECONDARY_REGION}"
  [[ -f "$STATE_DIR/route53.json" ]] || die "drill: Route53 failover record missing"
  [[ -f "$STATE_DIR/cloudflare.json" ]] || die "drill: Cloudflare failover record missing"

  log "drill RTO: ${elapsed}ms (budget ${RTO_BUDGET_MS}ms)"
  if (( elapsed >= RTO_BUDGET_MS )); then
    die "drill failed: RTO ${elapsed}ms exceeds ${RTO_BUDGET_MS}ms"
  fi
  log "drill passed"
  printf 'DR_DRILL_RTO_MS=%s\n' "$elapsed"
}

if [[ "$DRILL" -eq 1 ]]; then
  run_drill
else
  run_failover
fi
