variable "environment" { type = string }

resource "aws_ecr_repository" "app" {
  name                 = "crucible-${var.environment}-app"
  image_tag_mutability = "MUTABLE"

  image_scanning_configuration {
    scan_on_push = true
  }

  encryption_configuration {
    encryption_type = "KMS"
  }

  tags = {
    Name        = "crucible-${var.environment}-ecr"
    Environment = var.environment
    ManagedBy   = "terraform"
  }
}

output "repository_url" {
  value = aws_ecr_repository.app.repository_url
}
