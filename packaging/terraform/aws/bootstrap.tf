# The execution role already reads the runtime secrets; the master password is used only here, so it gets its own policy
resource "aws_iam_policy" "bootstrap_secrets" {
  count = var.create_database ? 1 : 0
  name  = "${local.name}-bootstrap-secrets-access"
  policy = jsonencode({
    Version = "2012-10-17"
    Statement = [{
      Effect   = "Allow"
      Action   = ["secretsmanager:GetSecretValue"]
      Resource = [aws_secretsmanager_secret.db_master_password[0].arn]
    }]
  })

  tags = local.tags
}

resource "aws_iam_role_policy_attachment" "task_execution_bootstrap_secrets" {
  count      = var.create_database ? 1 : 0
  role       = aws_iam_role.task_execution.name
  policy_arn = aws_iam_policy.bootstrap_secrets[0].arn
}

locals {
  # Every apply re-runs these statements: the CREATE is guarded by the exception and a repeated GRANT is a no-op
  bootstrap_sql = <<-SQL
    DO $$
    BEGIN
      CREATE USER ${var.db_username};
    EXCEPTION WHEN duplicate_object THEN NULL;
    END $$;
    GRANT rds_iam TO ${var.db_username};
    GRANT ALL PRIVILEGES ON DATABASE ${var.db_name} TO ${var.db_username};
    GRANT ALL ON SCHEMA public TO ${var.db_username};
    ALTER DEFAULT PRIVILEGES IN SCHEMA public GRANT ALL ON TABLES TO ${var.db_username};
    ALTER DEFAULT PRIVILEGES IN SCHEMA public GRANT ALL ON SEQUENCES TO ${var.db_username};
  SQL
}

resource "aws_ecs_task_definition" "bootstrap_db" {
  count                    = var.create_database ? 1 : 0
  family                   = "${local.name}-bootstrap-db"
  network_mode             = "awsvpc"
  requires_compatibilities = ["FARGATE"]
  cpu                      = 256
  memory                   = 512
  execution_role_arn       = aws_iam_role.task_execution.arn
  task_role_arn            = aws_iam_role.task.arn

  container_definitions = jsonencode([{
    name      = "psql"
    image     = "postgres:16-alpine"
    essential = true

    environment = [
      { name = "PGHOST", value = aws_rds_cluster.this[0].endpoint },
      { name = "PGPORT", value = tostring(aws_rds_cluster.this[0].port) },
      { name = "PGUSER", value = var.db_master_username },
      { name = "PGDATABASE", value = var.db_name },
      { name = "BOOTSTRAP_SQL", value = local.bootstrap_sql },
    ]
    secrets = [
      # The ":password::" suffix makes ECS read one field out of the JSON secret
      { name = "PGPASSWORD", valueFrom = "${aws_secretsmanager_secret.db_master_password[0].arn}:password::" },
    ]

    entryPoint = ["sh", "-c"]
    command    = ["echo \"$BOOTSTRAP_SQL\" | psql -v ON_ERROR_STOP=1"]

    logConfiguration = {
      logDriver = "awslogs"
      options = {
        awslogs-group         = aws_cloudwatch_log_group.bootstrap_db[0].name
        awslogs-region        = var.region
        awslogs-stream-prefix = "bootstrap"
      }
    }
  }])

  tags = local.tags
}

# Runs on apply only: the task needs the cluster's endpoint, the secret value and a role that can read it
resource "terraform_data" "bootstrap_db" {
  count = var.create_database ? 1 : 0

  triggers_replace = {
    cluster_resource_id = aws_rds_cluster.this[0].cluster_resource_id
    task_def_revision   = aws_ecs_task_definition.bootstrap_db[0].revision
  }

  provisioner "local-exec" {
    interpreter = ["bash", "-c"]
    environment = {
      CLUSTER  = aws_ecs_cluster.this.name
      TASK_DEF = aws_ecs_task_definition.bootstrap_db[0].arn
      SUBNETS  = join(",", local.private_subnet_ids)
      SG       = join(",", local.task_security_group_ids)
      REGION   = var.region
      LOG_GRP  = aws_cloudwatch_log_group.bootstrap_db[0].name
    }
    command = <<-EOT
      set -euo pipefail
      task_arn=$(aws ecs run-task --region "$REGION" --cluster "$CLUSTER" \
        --launch-type FARGATE --task-definition "$TASK_DEF" \
        --network-configuration "awsvpcConfiguration={subnets=[$SUBNETS],securityGroups=[$SG],assignPublicIp=DISABLED}" \
        --query 'tasks[0].taskArn' --output text)
      echo "bootstrap task: $task_arn"
      aws ecs wait tasks-stopped --region "$REGION" --cluster "$CLUSTER" --tasks "$task_arn"
      task_id=$(echo "$task_arn" | awk -F/ '{print $NF}')
      exit_code=$(aws ecs describe-tasks --region "$REGION" --cluster "$CLUSTER" --tasks "$task_id" \
        --query 'tasks[0].containers[0].exitCode' --output text)
      if [ "$exit_code" != "0" ]; then
        echo "Bootstrap failed (exit=$exit_code). Logs: $LOG_GRP" >&2
        exit 1
      fi
    EOT
  }

  # A task reads its secrets by ARN, so Terraform sees no edge to the version write or to the cluster accepting connections
  depends_on = [
    aws_rds_cluster_instance.writer,
    aws_iam_role_policy_attachment.task_execution_bootstrap_secrets,
    aws_secretsmanager_secret_version.db_master_password,
  ]
}
