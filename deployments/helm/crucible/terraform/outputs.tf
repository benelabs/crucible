# deployments/helm/crucible/terraform/outputs.tf

output "cluster_name" {
  description = "EKS cluster name."
  value       = module.eks.cluster_name
}

output "cluster_endpoint" {
  description = "EKS API server endpoint."
  value       = module.eks.cluster_endpoint
  sensitive   = false
}

output "cluster_certificate_authority_data" {
  description = "Base64-encoded certificate authority data for the cluster."
  value       = module.eks.cluster_certificate_authority_data
  sensitive   = true
}

output "vpc_id" {
  description = "VPC ID."
  value       = module.vpc.vpc_id
}

output "private_subnet_ids" {
  description = "Private subnet IDs."
  value       = module.vpc.private_subnets
}

output "backend_irsa_role_arn" {
  description = "IAM Role ARN assigned to the crucible-backend ServiceAccount (IRSA)."
  value       = module.backend_irsa.iam_role_arn
}

output "kms_key_arn" {
  description = "ARN of the KMS key used for envelope encryption."
  value       = aws_kms_key.crucible.arn
}

output "kms_key_alias" {
  description = "Alias of the crucible KMS key."
  value       = aws_kms_alias.crucible.name
}
