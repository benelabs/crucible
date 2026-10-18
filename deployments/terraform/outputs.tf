output "cloudflare_ssl_mode" {
  value       = "strict"
  description = "SSL mode enforced by cloudflare_zone_settings_override (Full Strict)."
}

output "origin_security_group_id" {
  value       = try(aws_security_group.origin_cloudflare_only[0].id, null)
  description = "Security group that admits only Cloudflare anycast ranges to the origin."
}

output "cloudflare_ipv4_cidrs" {
  value       = data.cloudflare_ip_ranges.cloudflare.ipv4_cidr_blocks
  description = "Cloudflare IPv4 prefixes used to lock down origin ingress."
}

output "route53_primary_health_check_id" {
  value       = try(aws_route53_health_check.primary[0].id, null)
  description = "Route53 health check attached to the PRIMARY failover record."
}

output "verify_origin_masking_command" {
  value       = "bash deployments/scripts/verify-origin-masking.sh --hostname ${var.dns_name}"
  description = "Command to confirm public DNS returns Cloudflare IPs only."
}
