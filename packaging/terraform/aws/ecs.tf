resource "aws_ecs_task_definition" "betterlite" {
  lifecycle {
    precondition {
      condition     = !var.connection_pool_enabled || local.database_enabled
      error_message = "connection_pool_enabled needs a database: set create_database = true or pass database_url."
    }

    precondition {
      condition     = var.otel_endpoint == "" || var.otel_exporter != ""
      error_message = "otel_exporter must not be empty when otel_endpoint is set."
    }
  }

  family                   = local.name
  network_mode             = "awsvpc"
  requires_compatibilities = ["FARGATE"]
  cpu                      = var.cpu
  memory                   = var.memory
  execution_role_arn       = aws_iam_role.task_execution.arn
  task_role_arn            = aws_iam_role.task.arn

  container_definitions = jsonencode([{
    name      = "betterlite"
    image     = var.image
    essential = true

    portMappings = [{ containerPort = 4000, protocol = "tcp" }]
    environment  = local.environment
    secrets      = local.secrets
    command      = ["--port", "4000"]

    # No container health check: the image ships no curl, so the ALB target group polls /health/readiness
    logConfiguration = {
      logDriver = "awslogs"
      options = {
        awslogs-group         = aws_cloudwatch_log_group.betterlite.name
        awslogs-region        = var.region
        awslogs-stream-prefix = "betterlite"
      }
    }
  }])

  tags = local.tags
}

resource "aws_ecs_service" "betterlite" {
  name            = local.name
  cluster         = aws_ecs_cluster.this.id
  task_definition = aws_ecs_task_definition.betterlite.arn
  desired_count   = var.desired_count
  launch_type     = "FARGATE"

  network_configuration {
    subnets          = local.private_subnet_ids
    security_groups  = local.task_security_group_ids
    assign_public_ip = false
  }

  load_balancer {
    target_group_arn = aws_lb_target_group.betterlite.arn
    container_name   = "betterlite"
    container_port   = 4000
  }

  deployment_minimum_healthy_percent = 50
  deployment_maximum_percent         = 200

  # Application Auto Scaling owns the count once the target in autoscaling.tf exists
  lifecycle {
    ignore_changes = [desired_count]
  }

  # A task reads its secrets by ARN, which gives Terraform no edge to the version that writes the value
  depends_on = [
    aws_lb_listener.http,
    aws_lb_listener.https,
    terraform_data.migration,
    aws_secretsmanager_secret_version.master_key,
    aws_secretsmanager_secret_version.license,
    aws_secretsmanager_secret_version.ui_password,
    aws_secretsmanager_secret_version.database_url,
    aws_secretsmanager_secret_version.redis_url,
  ]

  tags = local.tags
}
