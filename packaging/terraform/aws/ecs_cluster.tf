resource "aws_ecs_cluster" "this" {
  name = local.name

  setting {
    name  = "containerInsights"
    value = "enabled"
  }

  tags = local.tags
}

resource "aws_cloudwatch_log_group" "betterlite" {
  name              = "/ecs/${local.name}/betterlite"
  retention_in_days = var.log_retention_days

  tags = local.tags
}

resource "aws_cloudwatch_log_group" "migrations" {
  count             = local.database_enabled ? 1 : 0
  name              = "/ecs/${local.name}/migrations"
  retention_in_days = var.log_retention_days

  tags = local.tags
}

resource "aws_cloudwatch_log_group" "bootstrap_db" {
  count             = var.create_database ? 1 : 0
  name              = "/ecs/${local.name}/bootstrap-db"
  retention_in_days = var.log_retention_days

  tags = local.tags
}
