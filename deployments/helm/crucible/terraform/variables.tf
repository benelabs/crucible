# deployments/helm/crucible/terraform/variables.tf

variable "aws_region" {
  type        = string
  description = "AWS region to deploy into."
  default     = "us-east-1"
}

variable "environment" {
  type        = string
  description = "Deployment environment name (development | staging | production)."
  default     = "production"

  validation {
    condition     = contains(["development", "staging", "production"], var.environment)
    error_message = "environment must be one of: development, staging, production."
  }
}

variable "kubernetes_version" {
  type        = string
  description = "EKS Kubernetes version."
  default     = "1.30"
}

variable "vpc_cidr" {
  type        = string
  description = "CIDR block for the VPC."
  default     = "10.0.0.0/16"
}

variable "availability_zones" {
  type        = list(string)
  description = "Availability zones to use."
  default     = ["us-east-1a", "us-east-1b", "us-east-1c"]
}

variable "private_subnet_cidrs" {
  type        = list(string)
  description = "CIDR blocks for private subnets (one per AZ)."
  default     = ["10.0.1.0/24", "10.0.2.0/24", "10.0.3.0/24"]
}

variable "public_subnet_cidrs" {
  type        = list(string)
  description = "CIDR blocks for public subnets (one per AZ)."
  default     = ["10.0.101.0/24", "10.0.102.0/24", "10.0.103.0/24"]
}

variable "node_instance_types" {
  type        = list(string)
  description = "EC2 instance types for the managed node group."
  default     = ["t3.medium"]
}

variable "node_min_size" {
  type        = number
  description = "Minimum number of nodes in the node group."
  default     = 2
}

variable "node_max_size" {
  type        = number
  description = "Maximum number of nodes (for Cluster Autoscaler)."
  default     = 10
}

variable "node_desired_size" {
  type        = number
  description = "Desired number of nodes at launch."
  default     = 2
}

variable "vault_address" {
  type        = string
  description = "Vault server URL reachable from within the EKS cluster."
  # No default — must be supplied per environment.
}
