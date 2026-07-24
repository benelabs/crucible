variable "environment" { type = string }
variable "vpc_id" { type = string }
variable "subnet_ids" { type = list(string) }

resource "aws_db_subnet_group" "main" {
  name       = "crucible-${var.environment}-db-subnet-group"
  subnet_ids = var.subnet_ids

  tags = {
    Name = "crucible-${var.environment}-db-subnet-group"
  }
}

resource "aws_db_instance" "postgres" {
  allocated_storage     = 20
  max_allocated_storage = 100
  engine                = "postgres"
  engine_version        = "15"
  instance_class        = "db.t3.micro"
  db_name               = "crucible"
  username              = "crucible_admin"
  password              = "SecureChangeMe123!" # Placeholder for secret manager integration
  storage_encrypted     = true
  db_subnet_group_name  = aws_db_subnet_group.main.name
  skip_final_snapshot   = true

  tags = {
    Name        = "crucible-${var.environment}-db"
    Environment = var.environment
    ManagedBy   = "terraform"
  }
}

output "db_endpoint" {
  value = aws_db_instance.postgres.endpoint
}
