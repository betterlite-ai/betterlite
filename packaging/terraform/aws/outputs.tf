output "alb_dns_name" {
  description = "Public DNS name of the load balancer."
  value       = aws_lb.this.dns_name
}

output "alb_url" {
  description = "Proxy URL. Switches scheme on acm_certificate_arn: the dashboard is served at / and the API at /v1/*."
  value       = "${local.tls_enabled ? "https" : "http"}://${aws_lb.this.dns_name}"
}

output "ecs_cluster" {
  description = "ECS cluster name."
  value       = aws_ecs_cluster.this.name
}

output "vpc_id" {
  description = "VPC the stack runs in, whether the module created it or the caller passed it in via `vpc_id`."
  value       = local.vpc_id
}

output "task_security_group_id" {
  description = "Security group attached to the ECS tasks. Allow inbound from this group on an existing database or Redis reached over `database_url` / `redis_url`."
  value       = aws_security_group.tasks.id
}

output "aurora_writer_endpoint" {
  description = "Aurora writer endpoint (the cluster endpoint), which the container takes as DATABASE_HOST. Null when `create_database = false`."
  value       = one(aws_rds_cluster.this[*].endpoint)
}

output "aurora_reader_endpoint" {
  description = "Aurora reader endpoint, which the container takes as DATABASE_HOST_READ_REPLICA. Null when `create_database = false`."
  value       = one(aws_rds_cluster.this[*].reader_endpoint)
}

output "redis_endpoint" {
  description = "ElastiCache Redis primary endpoint as host:port. Null when `create_redis = false`."
  value       = one([for group in aws_elasticache_replication_group.this : "${group.primary_endpoint_address}:${group.port}"])
}

output "s3_bucket" {
  description = "S3 bucket name, exposed to the container as S3_BUCKET_NAME."
  value       = aws_s3_bucket.this.bucket
}

output "master_key_secret_arn" {
  description = "Secrets Manager ARN holding LITELLM_MASTER_KEY. Fetch it with `aws secretsmanager get-secret-value --secret-id <arn>`."
  value       = aws_secretsmanager_secret.master_key.arn
}

output "db_master_password_secret_arn" {
  description = "Secrets Manager ARN holding the Aurora master credentials, which only the bootstrap task reads. Null when `create_database = false`."
  value       = one(aws_secretsmanager_secret.db_master_password[*].arn)
}

output "db_bootstrap_sql" {
  description = "SQL to run once as the master DB user to create the IAM-authenticated application user. Empty when `create_database = false`."
  value       = var.create_database ? local.bootstrap_sql : ""
}

output "migration_run_command" {
  description = "Shell command that runs the migration task by hand, for when an apply could not. Empty when the stack has no database."
  value = !local.database_enabled ? "" : format(
    "aws ecs run-task --cluster %s --launch-type FARGATE --task-definition %s --network-configuration 'awsvpcConfiguration={subnets=[%s],securityGroups=[%s],assignPublicIp=DISABLED}' --region %s",
    aws_ecs_cluster.this.name,
    aws_ecs_task_definition.migrations[0].arn,
    join(",", local.private_subnet_ids),
    join(",", local.task_security_group_ids),
    var.region,
  )
}
