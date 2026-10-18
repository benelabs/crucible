# Route53 failover records + health checks used by deployments/scripts/dr-failover.sh.
# PRIMARY is the us-east-1 origin; SECONDARY is the us-west-2 standby. Cloudflare
# remains the public edge — these records are the origin hostnames CF connects to.

resource "aws_route53_health_check" "primary" {
  count             = var.route53_zone_id == "" || var.primary_origin_ip == "" ? 0 : 1
  ip_address        = var.primary_origin_ip
  fqdn              = "origin.${var.dns_name}"
  port              = 443
  type              = "HTTPS"
  resource_path     = var.healthcheck_path
  failure_threshold = 2
  request_interval  = 10
  measure_latency   = true
  enable_sni        = true
  regions           = ["us-east-1", "us-west-1", "us-west-2"]

  tags = {
    Name        = "crucible-${var.environment}-primary-origin"
    Environment = var.environment
  }
}

resource "aws_route53_health_check" "secondary" {
  count             = var.route53_zone_id == "" || var.secondary_origin_ip == "" ? 0 : 1
  ip_address        = var.secondary_origin_ip
  fqdn              = "origin.${var.dns_name}"
  port              = 443
  type              = "HTTPS"
  resource_path     = var.healthcheck_path
  failure_threshold = 2
  request_interval  = 10
  measure_latency   = true
  enable_sni        = true
  regions           = ["us-east-1", "us-west-1", "us-west-2"]

  tags = {
    Name        = "crucible-${var.environment}-secondary-origin"
    Environment = var.environment
  }
}

resource "aws_route53_record" "origin_primary" {
  count           = var.route53_zone_id == "" || var.primary_origin_ip == "" ? 0 : 1
  zone_id         = var.route53_zone_id
  name            = "origin.${var.dns_name}"
  type            = "A"
  ttl             = 30
  set_identifier  = "crucible-primary"
  health_check_id = aws_route53_health_check.primary[0].id
  failover_routing_policy {
    type = "PRIMARY"
  }
  records = [var.primary_origin_ip]
}

resource "aws_route53_record" "origin_secondary" {
  count           = var.route53_zone_id == "" || var.secondary_origin_ip == "" ? 0 : 1
  zone_id         = var.route53_zone_id
  name            = "origin.${var.dns_name}"
  type            = "A"
  ttl             = 30
  set_identifier  = "crucible-secondary"
  health_check_id = aws_route53_health_check.secondary[0].id
  failover_routing_policy {
    type = "SECONDARY"
  }
  records = [var.secondary_origin_ip]
}
