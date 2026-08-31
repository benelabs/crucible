# deployments/helm/crucible/terraform/main.tf
#
# Production requirement: Kubernetes Helm Charts & Production IaC (closes #916)
#
# This Terraform module provisions:
#   - AWS EKS cluster with managed node groups
#   - Helm release for the crucible chart
#   - HPA-ready node groups with cluster-autoscaler annotations
#   - IRSA (IAM Roles for Service Accounts) for the backend Vault auth
#
# Usage:
#   cd deployments/helm/crucible/terraform
#   terraform init
#   terraform apply -var-file=terraform.tfvars

terraform {
  required_version = ">= 1.5.0"

  required_providers {
    aws = {
      source  = "hashicorp/aws"
      version = "~> 5.0"
    }
    helm = {
      source  = "hashicorp/helm"
      version = "~> 2.13"
    }
    kubernetes = {
      source  = "hashicorp/kubernetes"
      version = "~> 2.29"
    }
  }

  # Remote state — adjust bucket/key per environment.
  backend "s3" {
    bucket         = "crucible-tf-state-bucket"
    key            = "helm/crucible/terraform.tfstate"
    region         = "us-east-1"
    encrypt        = true
    dynamodb_table = "crucible-tf-locks"
  }
}

# ── Providers ──────────────────────────────────────────────────────────────────

provider "aws" {
  region = var.aws_region

  default_tags {
    tags = {
      Project     = "crucible"
      Environment = var.environment
      ManagedBy   = "terraform"
    }
  }
}

# EKS cluster authentication for the Helm and Kubernetes providers.
data "aws_eks_cluster" "crucible" {
  name = module.eks.cluster_name
}

data "aws_eks_cluster_auth" "crucible" {
  name = module.eks.cluster_name
}

provider "helm" {
  kubernetes {
    host                   = data.aws_eks_cluster.crucible.endpoint
    cluster_ca_certificate = base64decode(data.aws_eks_cluster.crucible.certificate_authority[0].data)
    token                  = data.aws_eks_cluster_auth.crucible.token
  }
}

provider "kubernetes" {
  host                   = data.aws_eks_cluster.crucible.endpoint
  cluster_ca_certificate = base64decode(data.aws_eks_cluster.crucible.certificate_authority[0].data)
  token                  = data.aws_eks_cluster_auth.crucible.token
}

# ── Networking ─────────────────────────────────────────────────────────────────

module "vpc" {
  source  = "terraform-aws-modules/vpc/aws"
  version = "5.8.1"

  name = "crucible-${var.environment}"
  cidr = var.vpc_cidr

  azs             = var.availability_zones
  private_subnets = var.private_subnet_cidrs
  public_subnets  = var.public_subnet_cidrs

  enable_nat_gateway   = true
  single_nat_gateway   = var.environment != "production"
  enable_dns_hostnames = true
  enable_dns_support   = true

  # EKS requires these tags on subnets for load-balancer auto-discovery.
  private_subnet_tags = {
    "kubernetes.io/cluster/${local.cluster_name}" = "shared"
    "kubernetes.io/role/internal-elb"             = "1"
  }
  public_subnet_tags = {
    "kubernetes.io/cluster/${local.cluster_name}" = "shared"
    "kubernetes.io/role/elb"                      = "1"
  }
}

# ── EKS Cluster ────────────────────────────────────────────────────────────────

locals {
  cluster_name = "crucible-${var.environment}"
}

module "eks" {
  source  = "terraform-aws-modules/eks/aws"
  version = "20.11.1"

  cluster_name    = local.cluster_name
  cluster_version = var.kubernetes_version

  vpc_id                         = module.vpc.vpc_id
  subnet_ids                     = module.vpc.private_subnets
  cluster_endpoint_public_access = var.environment != "production"

  # Enable IRSA (IAM Roles for Service Accounts) — required for Vault AWS auth.
  enable_irsa = true

  # Managed node groups with cluster-autoscaler annotations.
  eks_managed_node_groups = {
    # General-purpose nodes for the backend and frontend.
    app = {
      name           = "crucible-app-${var.environment}"
      instance_types = var.node_instance_types
      min_size       = var.node_min_size
      max_size       = var.node_max_size
      desired_size   = var.node_desired_size

      # Labels consumed by Cluster Autoscaler and the Helm chart node selectors.
      labels = {
        role        = "app"
        environment = var.environment
      }

      # Required by Cluster Autoscaler.
      tags = {
        "k8s.io/cluster-autoscaler/enabled"                           = "true"
        "k8s.io/cluster-autoscaler/${local.cluster_name}"             = "owned"
      }
    }
  }

  # Grant cluster-admin to the Terraform execution role.
  enable_cluster_creator_admin_permissions = true
}

# ── IRSA: backend IAM role ─────────────────────────────────────────────────────
# The backend's ServiceAccount (`crucible-backend`) is annotated with this
# role ARN so pods can authenticate to AWS KMS and Secrets Manager via
# Vault's AWS IAM auth method.

module "backend_irsa" {
  source  = "terraform-aws-modules/iam/aws//modules/iam-role-for-service-accounts-eks"
  version = "5.39.1"

  role_name = "crucible-backend-${var.environment}"

  oidc_providers = {
    main = {
      provider_arn               = module.eks.oidc_provider_arn
      namespace_service_accounts = ["crucible:crucible-backend"]
    }
  }

  # Attach a policy that allows KMS decrypt and Secrets Manager read.
  role_policy_arns = {
    kms    = aws_iam_policy.backend_kms.arn
    sm     = aws_iam_policy.backend_secrets_manager.arn
  }
}

resource "aws_iam_policy" "backend_kms" {
  name        = "crucible-backend-kms-${var.environment}"
  description = "Allow crucible backend to use the KMS key for envelope encryption"
  policy = jsonencode({
    Version = "2012-10-17"
    Statement = [
      {
        Effect = "Allow"
        Action = [
          "kms:Decrypt",
          "kms:GenerateDataKey",
          "kms:DescribeKey",
        ]
        Resource = aws_kms_key.crucible.arn
      }
    ]
  })
}

resource "aws_iam_policy" "backend_secrets_manager" {
  name        = "crucible-backend-secrets-${var.environment}"
  description = "Allow crucible backend to read secrets from Secrets Manager"
  policy = jsonencode({
    Version = "2012-10-17"
    Statement = [
      {
        Effect   = "Allow"
        Action   = ["secretsmanager:GetSecretValue"]
        Resource = "arn:aws:secretsmanager:${var.aws_region}:*:secret:crucible/*"
      }
    ]
  })
}

# ── KMS key for envelope encryption ───────────────────────────────────────────
resource "aws_kms_key" "crucible" {
  description             = "Crucible ${var.environment} KMS key for envelope encryption"
  deletion_window_in_days = 30
  enable_key_rotation     = true  # AWS rotates annually; Vault Transit handles app-level rotation.

  tags = {
    Name = "crucible-${var.environment}"
  }
}

resource "aws_kms_alias" "crucible" {
  name          = "alias/crucible-${var.environment}"
  target_key_id = aws_kms_key.crucible.key_id
}

# ── Helm release: crucible ────────────────────────────────────────────────────

resource "helm_release" "crucible" {
  name             = "crucible"
  chart            = "${path.module}/.."  # Points to deployments/helm/crucible/
  namespace        = "crucible"
  create_namespace = true
  atomic           = true   # Roll back on failure.
  cleanup_on_fail  = true
  wait             = true
  timeout          = 600    # 10 minutes for all pods to become ready.

  # Values file — base defaults.
  values = [
    file("${path.module}/../values.yaml"),
    file("${path.module}/../values-${var.environment}.yaml"),
  ]

  # Per-environment overrides that must not be stored in plain YAML.
  set {
    name  = "global.environment"
    value = var.environment
  }

  set {
    name  = "backend.serviceAccount.annotations.eks\\.amazonaws\\.com/role-arn"
    value = module.backend_irsa.iam_role_arn
  }

  set {
    name  = "backend.vault.address"
    value = var.vault_address
  }

  depends_on = [
    module.eks,
  ]
}

# ── Cluster Autoscaler ─────────────────────────────────────────────────────────

resource "helm_release" "cluster_autoscaler" {
  name       = "cluster-autoscaler"
  repository = "https://kubernetes.github.io/autoscaler"
  chart      = "cluster-autoscaler"
  version    = "9.37.0"
  namespace  = "kube-system"

  set {
    name  = "autoDiscovery.clusterName"
    value = local.cluster_name
  }
  set {
    name  = "awsRegion"
    value = var.aws_region
  }

  depends_on = [module.eks]
}
