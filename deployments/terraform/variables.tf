variable "cloudflare_api_token" {
  type        = string
  sensitive   = true
  default     = ""
  description = "Cloudflare API token with Zone.WAF, Zone.DNS, and Zone.SSL permissions. Empty for validate-only runs."
}

variable "cloudflare_zone_id" {
  type        = string
  default     = ""
  description = "Cloudflare zone ID for the Crucible production domain. Empty is valid for terraform validate."
}

variable "cloudflare_account_id" {
  type        = string
  default     = ""
  description = "Cloudflare account ID (required for load-balancer / account rulesets)."
}

variable "environment" {
  type    = string
  default = "production"
}

variable "aws_region" {
  type        = string
  default     = "us-east-1"
  description = "Primary origin region."
}

variable "aws_secondary_region" {
  type        = string
  default     = "us-west-2"
  description = "DR / standby origin region."
}

variable "origin_vpc_id" {
  type        = string
  default     = ""
  description = "VPC that hosts the origin. When empty, the Cloudflare-only security group is skipped."
}

variable "origin_https_port" {
  type    = number
  default = 443
}

variable "dns_name" {
  type        = string
  default     = "api.crucible.dev"
  description = "Public hostname steered by Cloudflare and Route53 failover records."
}

variable "primary_origin_ip" {
  type        = string
  default     = ""
  description = "Direct origin IPv4 in the primary region. Never published in public DNS."
}

variable "secondary_origin_ip" {
  type        = string
  default     = ""
  description = "Direct origin IPv4 in the secondary region. Never published in public DNS."
}

variable "route53_zone_id" {
  type        = string
  default     = ""
  description = "Route53 hosted zone for DNS failover health checks. Empty skips Route53 resources."
}

variable "healthcheck_path" {
  type    = string
  default = "/health/ready"
}

variable "api_rate_limit_per_minute" {
  type        = number
  default     = 100
  description = "Per-IP request budget for /api/* before Cloudflare mitigates."
}
