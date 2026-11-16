data "aws_iam_policy_document" "task_assume" {
  statement {
    actions = ["sts:AssumeRole"]
    principals {
      type        = "Service"
      identifiers = ["ecs-tasks.amazonaws.com"]
    }
  }
}

resource "aws_iam_role" "task_execution" {
  name               = "${local.name}-task-execution"
  assume_role_policy = data.aws_iam_policy_document.task_assume.json

  tags = local.tags
}

resource "aws_iam_role_policy_attachment" "task_execution" {
  role       = aws_iam_role.task_execution.name
  policy_arn = "arn:aws:iam::aws:policy/service-role/AmazonECSTaskExecutionRolePolicy"
}

# An extra secret may carry the ":jsonKey::" valueFrom suffix; IAM matches the bare secret ARN, so keep the first seven fields
locals {
  extra_secret_arns = distinct([
    for v in values(var.extra_secrets) :
    join(":", slice(split(":", v), 0, 7))
  ])
}

# The execution role resolves these while starting a task; the runtime gets no Secrets Manager access of its own
data "aws_iam_policy_document" "secrets_access" {
  statement {
    actions = ["secretsmanager:GetSecretValue"]
    resources = concat(
      [aws_secretsmanager_secret.master_key.arn],
      aws_secretsmanager_secret.license[*].arn,
      aws_secretsmanager_secret.ui_password[*].arn,
      aws_secretsmanager_secret.database_url[*].arn,
      aws_secretsmanager_secret.redis_url[*].arn,
      local.extra_secret_arns,
      var.otel_headers_secret_arn == "" ? [] : [var.otel_headers_secret_arn],
    )
  }
}

resource "aws_iam_policy" "secrets_access" {
  name   = "${local.name}-secrets-access"
  policy = data.aws_iam_policy_document.secrets_access.json

  tags = local.tags
}

resource "aws_iam_role_policy_attachment" "task_execution_secrets" {
  role       = aws_iam_role.task_execution.name
  policy_arn = aws_iam_policy.secrets_access.arn
}

# The running container mints IAM-signed Postgres tokens for the application user with this role
resource "aws_iam_role" "task" {
  name               = "${local.name}-task"
  assume_role_policy = data.aws_iam_policy_document.task_assume.json

  tags = local.tags
}

data "aws_caller_identity" "current" {}

# IAM auth exists only on the module's own cluster; an existing database is reached with the database_url credentials
data "aws_iam_policy_document" "rds_iam_connect" {
  count = var.create_database ? 1 : 0

  statement {
    actions = ["rds-db:connect"]
    resources = [
      "arn:aws:rds-db:${var.region}:${data.aws_caller_identity.current.account_id}:dbuser:${aws_rds_cluster.this[0].cluster_resource_id}/${var.db_username}",
    ]
  }
}

resource "aws_iam_policy" "rds_iam_connect" {
  count  = var.create_database ? 1 : 0
  name   = "${local.name}-rds-iam-connect"
  policy = data.aws_iam_policy_document.rds_iam_connect[0].json

  tags = local.tags
}

resource "aws_iam_role_policy_attachment" "task_rds_iam_connect" {
  count      = var.create_database ? 1 : 0
  role       = aws_iam_role.task.name
  policy_arn = aws_iam_policy.rds_iam_connect[0].arn
}
