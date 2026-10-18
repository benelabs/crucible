#!/usr/bin/env bash
# Automated verification that public DNS for the Crucible origin is masked
# behind Cloudflare anycast IPs (orange-cloud / proxied records). Direct
# origin addresses must never appear in public DNS.
#
# Usage:
#   deployments/scripts/verify-origin-masking.sh [--mock] [--hostname api.crucible.dev]
#
# Live mode resolves HOSTNAME and checks every A/AAAA against Cloudflare
# published ranges. Mock mode uses fixture addresses so CI can run offline.
set -euo pipefail

MOCK=0
HOSTNAME="${DR_DNS_NAME:-api.crucible.dev}"
ORIGIN_IP="${DR_ORIGIN_IP:-}"

while [[ $# -gt 0 ]]; do
  case "$1" in
    --mock) MOCK=1; shift ;;
    --hostname) HOSTNAME="${2:-}"; shift 2 ;;
    --origin-ip) ORIGIN_IP="${2:-}"; shift 2 ;;
    -h|--help)
      sed -n '2,12p' "$0" | sed 's/^# \?//'
      exit 0
      ;;
    *) echo "unknown argument: $1" >&2; exit 1 ;;
  esac
done

log() { printf '[origin-mask] %s\n' "$*"; }

# Bundled Cloudflare anycast prefixes used when the live list cannot be fetched
# (mock / offline). Source: https://www.cloudflare.com/ips-v4 and /ips-v6
BUNDLED_RANGES=$(cat <<'EOF'
173.245.48.0/20
103.21.244.0/22
103.22.200.0/22
103.31.4.0/22
141.101.64.0/18
108.162.192.0/18
190.93.240.0/20
188.114.96.0/20
197.234.240.0/22
198.41.128.0/17
162.158.0.0/15
104.16.0.0/13
104.24.0.0/14
172.64.0.0/13
131.0.72.0/22
2400:cb00::/32
2606:4700::/32
2803:f800::/32
2405:b500::/32
2405:8100::/32
2a06:98c0::/29
2c0f:f248::/32
EOF
)

fetch_ranges() {
  if [[ "$MOCK" -eq 1 ]]; then
    printf '%s\n' "$BUNDLED_RANGES"
    return
  fi
  local v4 v6
  v4="$(curl -fsS --max-time 10 https://www.cloudflare.com/ips-v4 || true)"
  v6="$(curl -fsS --max-time 10 https://www.cloudflare.com/ips-v6 || true)"
  if [[ -z "$v4" ]]; then
    log "WARNING: could not fetch live Cloudflare IP lists; using bundled ranges"
    printf '%s\n' "$BUNDLED_RANGES"
    return
  fi
  printf '%s\n%s\n' "$v4" "$v6"
}

resolve_public_ips() {
  if [[ "$MOCK" -eq 1 ]]; then
    # 104.16.0.1 sits inside 104.16.0.0/13 (Cloudflare). 2606:4700::1 is CF IPv6.
    printf '%s\n' "104.16.0.1" "2606:4700::1"
    return
  fi
  { getent ahostsv4 "$HOSTNAME" 2>/dev/null | awk '{print $1}' | sort -u
    getent ahostsv6 "$HOSTNAME" 2>/dev/null | awk '{print $1}' | sort -u
    dig +short A "$HOSTNAME" 2>/dev/null || true
    dig +short AAAA "$HOSTNAME" 2>/dev/null || true
  } | grep -E '^[0-9a-fA-F:.]+$' | sort -u
}

RANGES="$(fetch_ranges)"
PUBLIC_IPS="$(resolve_public_ips)"
[[ -n "$PUBLIC_IPS" ]] || { log "ERROR: no public addresses resolved for ${HOSTNAME}"; exit 1; }

export RANGES PUBLIC_IPS HOSTNAME ORIGIN_IP
python3 - <<'PY'
import ipaddress
import os
import sys

ranges = [ipaddress.ip_network(line.strip()) for line in os.environ["RANGES"].splitlines() if line.strip()]
public_ips = [ipaddress.ip_address(line.strip()) for line in os.environ["PUBLIC_IPS"].splitlines() if line.strip()]
origin = os.environ.get("ORIGIN_IP", "").strip()
hostname = os.environ.get("HOSTNAME", "")

failed = False
for ip in public_ips:
    if any(ip in net for net in ranges):
        print(f"[origin-mask] OK  {ip} is a Cloudflare anycast address")
        continue
    print(f"[origin-mask] FAIL {ip} is NOT in Cloudflare IP ranges (origin leak for {hostname})")
    failed = True

if origin:
    origin_ip = ipaddress.ip_address(origin)
    if origin_ip in public_ips:
        print(f"[origin-mask] FAIL origin {origin} appears in public DNS for {hostname}")
        failed = True
    else:
        print(f"[origin-mask] OK  origin {origin} is not published in DNS")

if failed:
    sys.exit(1)

print("[origin-mask] origin IP masking verified")
PY
