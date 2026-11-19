resource "aws_ecs_task_definition" "migrations" {
  count                    = local.database_enabled ? 1 : 0
  family                   = "${local.name}-migrations"
  network_mode             = "awsvpc"
  requires_compatibilities = ["FARGATE"]
  cpu                      = 512
  memory                   = 1024
  execution_role_arn       = aws_iam_role.task_execution.arn
  task_role_arn            = aws_iam_role.task.arn

  container_definitions = jsonencode([{
    name      = "migrations"
    image     = var.image
    essential = true

    # betterlite-db-admin runs the schema migration; the image's default command serves traffic instead
    entryPoint  = ["betterlite-db-admin"]
    command     = ["migrate"]
    environment = local.environment
    secrets     = local.secrets

    logConfiguration = {
      logDriver = "awslogs"
      options = {
        awslogs-group         = aws_cloudwatch_log_group.migrations[0].name
        awslogs-region        = var.region
        awslogs-stream-prefix = "migrations"
      }
    }
  }])

  tags = local.tags
}

# Runs on apply only: the task needs the cluster, the subnets and a role that can read the secrets
resource "terraform_data" "migration" {
  count = local.database_enabled ? 1 : 0

  triggers_replace = {
    task_def_revision = aws_ecs_task_definition.migrations[0].revision
    bootstrap_id      = join(",", terraform_data.bootstrap_db[*].id)
  }

  provisioner "local-exec" {
    interpreter = ["bash", "-c"]
    environment = {
      CLUSTER  = aws_ecs_cluster.this.name
      TASK_DEF = aws_ecs_task_definition.migrations[0].arn
      SUBNETS  = join(",", local.private_subnet_ids)
      SG       = join(",", local.task_security_group_ids)
      REGION   = var.region
      LOG_GRP  = aws_cloudwatch_log_group.migrations[0].name
    }
    command = <<-EOT
      set -euo pipefail
      task_arn=$(aws ecs run-task --region "$REGION" --cluster "$CLUSTER" \
        --launch-type FARGATE --task-definition "$TASK_DEF" \
        --network-configuration "awsvpcConfiguration={subnets=[$SUBNETS],securityGroups=[$SG],assignPublicIp=DISABLED}" \
        --query 'tasks[0].taskArn' --output text)
      echo "migration task: $task_arn"
      aws ecs wait tasks-stopped --region "$REGION" --cluster "$CLUSTER" --tasks "$task_arn"
      task_id=$(echo "$task_arn" | awk -F/ '{print $NF}')
      exit_code=$(aws ecs describe-tasks --region "$REGION" --cluster "$CLUSTER" --tasks "$task_id" \
        --query 'tasks[0].containers[0].exitCode' --output text)
      if [ "$exit_code" != "0" ]; then
        echo "Migration failed (exit=$exit_code). Logs: $LOG_GRP" >&2
        exit 1
      fi
    EOT
  }

  # A container reads a secret by ARN, so Terraform sees no edge from the ARN to the version that gives it a value
  depends_on = [
    terraform_data.bootstrap_db,
    aws_iam_role_policy_attachment.task_execution_secrets,
    aws_secretsmanager_secret_version.master_key,
    aws_secretsmanager_secret_version.license,
    aws_secretsmanager_secret_version.ui_password,
    aws_secretsmanager_secret_version.database_url,
    aws_secretsmanager_secret_version.redis_url,
  ]
}
