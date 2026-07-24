output "vpc_id" {
  value = module.networking.vpc_id
}

output "db_endpoint" {
  value = module.database.db_endpoint
}

output "repository_url" {
  value = module.registry.repository_url
}
