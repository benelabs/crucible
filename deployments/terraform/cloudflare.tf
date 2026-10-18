# Location: deployments/terraform/cloudflare.tf
# Production requirement: Edge CDN Caching & DDoS Shield Integration (Cloudflare)
#
# Creates zone-level SSL Full (Strict), WAF rules that block known botnets and
# exploit probes, edge cache settings for static assets, and API rate limits.
# Origin ingress is restricted to Cloudflare anycast ranges so the origin IP
# cannot be hit directly during a layer-7 flood.

data "cloudflare_ip_ranges" "cloudflare" {}

# ---------------------------------------------------------------------------
# SSL Full (Strict) + transport hardening
# ---------------------------------------------------------------------------

resource "cloudflare_zone_settings_override" "crucible" {
  zone_id = var.cloudflare_zone_id

  settings {
    ssl                      = "strict"
    always_use_https         = "on"
    min_tls_version          = "1.2"
    tls_1_3                  = "on"
    automatic_https_rewrites = "on"
    opportunistic_encryption = "on"
    security_level           = "high"
    challenge_ttl            = 1800
    brotli                   = "on"
    http3                    = "on"
    zero_rtt                 = "on"
    websockets               = "on"
    ip_geolocation           = "on"
    email_obfuscation        = "on"
    hotlink_protection       = "on"
    server_side_exclude      = "on"
    waf                      = "on"
    browser_check            = "on"
    always_online            = "on"

    security_header {
      enabled            = true
      include_subdomains = true
      nosniff            = true
      preload            = true
      max_age            = 31536000
    }
  }
}

resource "cloudflare_authenticated_origin_pulls" "crucible" {
  zone_id = var.cloudflare_zone_id
  enabled = true
}

# ---------------------------------------------------------------------------
# Custom WAF — botnet signatures, exploit probes, high threat score
# ---------------------------------------------------------------------------

resource "cloudflare_ruleset" "waf_custom" {
  zone_id     = var.cloudflare_zone_id
  name        = "crucible-waf-custom"
  description = "Block known botnets, scanners, and credential-stuffing probes"
  kind        = "zone"
  phase       = "http_request_firewall_custom"

  rules {
    action = "block"
    expression = trimspace(<<-EXPR
      (cf.threat_score gt 14)
      or (cf.client.bot and not cf.bot_management.verified_bot)
      or (http.user_agent contains "masscan")
      or (http.user_agent contains "nikto")
      or (http.user_agent contains "sqlmap")
      or (http.user_agent contains "zgrab")
      or (http.user_agent contains "GTBot")
      or (http.user_agent contains "AhrefsBot" and not http.request.uri.path.extension in {"js" "css" "png" "svg"})
    EXPR
    )
    description = "Block high threat-score clients and known botnet / scanner UAs"
    enabled     = true
  }

  rules {
    action = "block"
    expression = trimspace(<<-EXPR
      (http.request.uri.path contains "/.env")
      or (http.request.uri.path contains "/wp-admin")
      or (http.request.uri.path contains "/wp-login")
      or (http.request.uri.path contains "/xmlrpc.php")
      or (http.request.uri.path contains "/vendor/phpunit")
      or (http.request.uri.query contains "etc/passwd")
      or (http.request.uri.query contains "union select")
    EXPR
    )
    description = "Block common origin-probe and injection paths"
    enabled     = true
  }
}

# Cloudflare Managed Ruleset + OWASP Core (account/zone managed phase).
# IDs: https://developers.cloudflare.com/waf/managed-rules/
resource "cloudflare_ruleset" "waf_managed" {
  zone_id     = var.cloudflare_zone_id
  name        = "crucible-waf-managed"
  description = "Execute Cloudflare Managed and OWASP Core rulesets"
  kind        = "zone"
  phase       = "http_request_firewall_managed"

  rules {
    action = "execute"
    action_parameters {
      id = "efb7b8c949ac4650a09736fc376e9aee"
    }
    expression  = "true"
    description = "Cloudflare Managed Ruleset"
    enabled     = true
  }

  rules {
    action = "execute"
    action_parameters {
      id = "4814384a9e5d4991b9815dcfc25d2f1f"
    }
    expression  = "true"
    description = "Cloudflare OWASP Core Ruleset"
    enabled     = true
  }
}

# ---------------------------------------------------------------------------
# Rate limiting — protect backend API endpoints
# ---------------------------------------------------------------------------

resource "cloudflare_ruleset" "api_rate_limit" {
  zone_id     = var.cloudflare_zone_id
  name        = "crucible-api-rate-limit"
  description = "Per-IP budget for /api/* to absorb layer-7 floods"
  kind        = "zone"
  phase       = "http_ratelimit"

  rules {
    action = "block"
    action_parameters {
      response {
        status_code  = 429
        content      = "{\"error\":\"rate_limited\"}"
        content_type = "application/json"
      }
    }
    expression  = "(http.request.uri.path matches \"^/api/\")"
    description = "API ${var.api_rate_limit_per_minute} req/min per IP"
    enabled     = true
    ratelimit {
      characteristics     = ["cf.colo.id", "ip.src"]
      period              = 60
      requests_per_period = var.api_rate_limit_per_minute
      mitigation_timeout  = 600
    }
  }
}

# ---------------------------------------------------------------------------
# Edge cache — static assets (JS/CSS/WASM/fonts/images)
# ---------------------------------------------------------------------------

resource "cloudflare_ruleset" "static_asset_cache" {
  zone_id     = var.cloudflare_zone_id
  name        = "crucible-static-cache"
  description = "Cache hashed frontend assets at the edge"
  kind        = "zone"
  phase       = "http_request_cache_settings"

  rules {
    action = "set_cache_settings"
    action_parameters {
      cache = true
      edge_ttl {
        mode    = "override_origin"
        default = 86400
      }
      browser_ttl {
        mode    = "override_origin"
        default = 14400
      }
    }
    expression = trimspace(<<-EXPR
      (http.request.uri.path.extension in {"js" "css" "mjs" "png" "jpg" "jpeg" "gif" "svg" "webp" "avif" "woff" "woff2" "ttf" "ico" "wasm" "map"})
    EXPR
    )
    description = "Cache static assets for 24h at the edge"
    enabled     = true
  }

  rules {
    action = "set_cache_settings"
    action_parameters {
      cache = false
    }
    expression  = "(http.request.uri.path matches \"^/api/\") or (http.request.uri.path matches \"^/health\")"
    description = "Never cache API or health endpoints"
    enabled     = true
  }
}

# ---------------------------------------------------------------------------
# Origin ingress — Cloudflare IP ranges only (AWS security group)
# ---------------------------------------------------------------------------

resource "aws_security_group" "origin_cloudflare_only" {
  count       = var.origin_vpc_id == "" ? 0 : 1
  name        = "crucible-${var.environment}-origin-cloudflare-only"
  description = "Allow HTTPS only from Cloudflare anycast ranges"
  vpc_id      = var.origin_vpc_id

  tags = {
    Name        = "crucible-${var.environment}-origin-cloudflare-only"
    Environment = var.environment
    ManagedBy   = "terraform"
    Purpose     = "origin-ip-masking"
  }
}

resource "aws_vpc_security_group_ingress_rule" "cloudflare_https_v4" {
  for_each = var.origin_vpc_id == "" ? toset([]) : toset(data.cloudflare_ip_ranges.cloudflare.ipv4_cidr_blocks)

  security_group_id = aws_security_group.origin_cloudflare_only[0].id
  description       = "Cloudflare IPv4 anycast"
  ip_protocol       = "tcp"
  from_port         = var.origin_https_port
  to_port           = var.origin_https_port
  cidr_ipv4         = each.value
}

resource "aws_vpc_security_group_ingress_rule" "cloudflare_https_v6" {
  for_each = var.origin_vpc_id == "" ? toset([]) : toset(data.cloudflare_ip_ranges.cloudflare.ipv6_cidr_blocks)

  security_group_id = aws_security_group.origin_cloudflare_only[0].id
  description       = "Cloudflare IPv6 anycast"
  ip_protocol       = "tcp"
  from_port         = var.origin_https_port
  to_port           = var.origin_https_port
  cidr_ipv6         = each.value
}

resource "aws_vpc_security_group_egress_rule" "origin_all" {
  count             = var.origin_vpc_id == "" ? 0 : 1
  security_group_id = aws_security_group.origin_cloudflare_only[0].id
  ip_protocol       = "-1"
  cidr_ipv4         = "0.0.0.0/0"
  description       = "Origin egress (package updates, AWS APIs, replica traffic)"
}
