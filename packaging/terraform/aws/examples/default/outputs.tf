output "alb_url" {
  description = "Proxy URL. The dashboard is served at / and the API at /v1/*."
  value       = module.betterlite.alb_url
}

output "alb_dns_name" {
  description = "Public DNS name of the load balancer."
  value       = module.betterlite.alb_dns_name
}

output "ecs_cluster" {
  description = "ECS cluster name."
  value       = module.betterlite.ecs_cluster
}

output "migration_run_command" {
  description = "Break-glass command that runs the migration task by hand, for when an apply could not."
  value       = module.betterlite.migration_run_command
}

output "db_bootstrap_sql" {
  description = "SQL to run once as the master DB user to create the IAM-authenticated application user."
  value       = module.betterlite.db_bootstrap_sql
}
