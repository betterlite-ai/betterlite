mock_provider "aws" {
  mock_data "aws_iam_policy_document" {
    defaults = {
      json = "{\"Version\":\"2012-10-17\",\"Statement\":[]}"
    }
  }

  mock_data "aws_caller_identity" {
    defaults = {
      account_id = "111122223333"
    }
  }
}

mock_provider "random" {}

# A plan leaves every computed attribute unknown, so each value an assertion reads is fixed here for the plan phase
override_resource {
  target          = random_id.s3_suffix
  values          = { hex = "0a1b2c3d" }
  override_during = plan
}

override_resource {
  target = aws_rds_cluster.this
  values = {
    cluster_resource_id = "cluster-ABCDEFGHIJKLMNOPQRSTUVW"
    endpoint            = "acme-litellm-test.cluster-cdefghijklmn.us-east-1.rds.amazonaws.com"
    reader_endpoint     = "acme-litellm-test.cluster-ro-cdefghijklmn.us-east-1.rds.amazonaws.com"
    port                = 5432
  }
  override_during = plan
}

override_resource {
  target = aws_elasticache_replication_group.this
  values = {
    primary_endpoint_address = "acme-litellm-test-redis.abc123.0001.use1.cache.amazonaws.com"
    port                     = 6379
  }
  override_during = plan
}

override_resource {
  target          = aws_secretsmanager_secret.master_key
  values          = { arn = "arn:aws:secretsmanager:us-east-1:111122223333:secret:acme-litellm-test-master-key-AAAAAA" }
  override_during = plan
}

override_resource {
  target          = aws_secretsmanager_secret.license
  values          = { arn = "arn:aws:secretsmanager:us-east-1:111122223333:secret:acme-litellm-test-license-BBBBBB" }
  override_during = plan
}

override_resource {
  target          = aws_secretsmanager_secret.ui_password
  values          = { arn = "arn:aws:secretsmanager:us-east-1:111122223333:secret:acme-litellm-test-ui-password-CCCCCC" }
  override_during = plan
}

override_resource {
  target          = aws_secretsmanager_secret.database_url
  values          = { arn = "arn:aws:secretsmanager:us-east-1:111122223333:secret:acme-litellm-test-database-url-DDDDDD" }
  override_during = plan
}

override_resource {
  target          = aws_secretsmanager_secret.redis_url
  values          = { arn = "arn:aws:secretsmanager:us-east-1:111122223333:secret:acme-litellm-test-redis-url-EEEEEE" }
  override_during = plan
}

override_resource {
  target = aws_lb_target_group.betterlite
  values = {
    arn        = "arn:aws:elasticloadbalancing:us-east-1:111122223333:targetgroup/acme-litellm-test/0123456789abcdef"
    arn_suffix = "targetgroup/acme-litellm-test/0123456789abcdef"
  }
  override_during = plan
}

override_resource {
  target          = aws_security_group.tasks
  values          = { id = "sg-0123456789abcdef0" }
  override_during = plan
}

variables {
  region              = "us-east-1"
  tenant              = "acme"
  env                 = "test"
  azs                 = ["us-east-1a", "us-east-1b"]
  image               = "ghcr.io/example/betterlite:v1"
  allow_plaintext_alb = true
}

run "service_runs_one_betterlite_container_on_port_4000" {
  command = plan

  assert {
    condition = alltrue([
      aws_ecs_task_definition.betterlite.family == local.name,
      aws_ecs_task_definition.betterlite.network_mode == "awsvpc",
      tostring(aws_ecs_task_definition.betterlite.cpu) == "1024",
      tostring(aws_ecs_task_definition.betterlite.memory) == "4096",
      contains(aws_ecs_task_definition.betterlite.requires_compatibilities, "FARGATE"),
      aws_ecs_service.betterlite.launch_type == "FARGATE",
      aws_ecs_service.betterlite.desired_count == 2,
      aws_ecs_service.betterlite.deployment_minimum_healthy_percent == 50,
      aws_ecs_service.betterlite.deployment_maximum_percent == 200,
    ])
    error_message = "The service must be one 1024 CPU / 4096 MiB Fargate task definition with 2 tasks and deployment percentages 50/200."
  }

  assert {
    condition = alltrue([
      jsondecode(aws_ecs_task_definition.betterlite.container_definitions)[0].name == "betterlite",
      jsondecode(aws_ecs_task_definition.betterlite.container_definitions)[0].image == var.image,
      jsondecode(aws_ecs_task_definition.betterlite.container_definitions)[0].essential == true,
      jsondecode(aws_ecs_task_definition.betterlite.container_definitions)[0].command == ["--port", "4000"],
      jsondecode(aws_ecs_task_definition.betterlite.container_definitions)[0].portMappings[0].containerPort == 4000,
      jsondecode(aws_ecs_task_definition.betterlite.container_definitions)[0].portMappings[0].protocol == "tcp",
      !contains(keys(jsondecode(aws_ecs_task_definition.betterlite.container_definitions)[0]), "healthCheck"),
    ])
    error_message = "The container must be betterlite on port 4000 started with --port 4000, with no container health check because the image has no curl."
  }

  assert {
    condition = alltrue([
      jsondecode(aws_ecs_task_definition.betterlite.container_definitions)[0].logConfiguration.options["awslogs-group"] == "/ecs/${local.name}/betterlite",
      jsondecode(aws_ecs_task_definition.betterlite.container_definitions)[0].logConfiguration.options["awslogs-region"] == var.region,
      jsondecode(aws_ecs_task_definition.betterlite.container_definitions)[0].logConfiguration.options["awslogs-stream-prefix"] == "betterlite",
      aws_cloudwatch_log_group.betterlite.name == "/ecs/${local.name}/betterlite",
    ])
    error_message = "The container must log to the module's betterlite log group in the stack region."
  }

  assert {
    condition = alltrue([
      tolist(aws_ecs_service.betterlite.load_balancer[*].target_group_arn) == tolist([aws_lb_target_group.betterlite.arn]),
      tolist(aws_ecs_service.betterlite.load_balancer[*].container_name) == tolist(["betterlite"]),
      tolist(aws_ecs_service.betterlite.load_balancer[*].container_port) == tolist([4000]),
      aws_ecs_service.betterlite.network_configuration[0].assign_public_ip == false,
    ])
    error_message = "The service must register the container with the single target group and run without a public IP."
  }
}

run "supplied_vpc_places_the_service_in_its_private_subnets" {
  command = plan

  variables {
    vpc_id             = "vpc-00000000000000001"
    public_subnet_ids  = ["subnet-pub-a", "subnet-pub-b"]
    private_subnet_ids = ["subnet-priv-a", "subnet-priv-b"]
  }

  assert {
    condition = alltrue([
      toset(aws_ecs_service.betterlite.network_configuration[0].subnets) == toset(var.private_subnet_ids),
      toset(aws_ecs_service.betterlite.network_configuration[0].security_groups) == toset(local.task_security_group_ids),
      aws_ecs_service.betterlite.network_configuration[0].assign_public_ip == false,
    ])
    error_message = "The service must run in the supplied private subnets with the module's task security groups and no public IP."
  }
}

run "managed_stores_fill_the_environment_and_the_master_key_is_a_secret" {
  command = plan

  assert {
    condition = alltrue([
      for expected in [
        "IAM_TOKEN_DB_AUTH=true",
        "DATABASE_HOST=${aws_rds_cluster.this[0].endpoint}",
        "DATABASE_PORT=${tostring(aws_rds_cluster.this[0].port)}",
        "DATABASE_USER=${var.db_username}",
        "DATABASE_NAME=${var.db_name}",
        "DATABASE_HOST_READ_REPLICA=${aws_rds_cluster.this[0].reader_endpoint}",
        "DATABASE_PORT_READ_REPLICA=${tostring(aws_rds_cluster.this[0].port)}",
        "REDIS_HOST=${aws_elasticache_replication_group.this[0].primary_endpoint_address}",
        "REDIS_PORT=${tostring(aws_elasticache_replication_group.this[0].port)}",
        "REDIS_SSL=true",
        "S3_BUCKET_NAME=${aws_s3_bucket.this.bucket}",
        "S3_REGION_NAME=${var.region}",
        "AWS_REGION=${var.region}",
        "AWS_REGION_NAME=${var.region}",
        "STORE_MODEL_IN_DB=true",
        ] : contains([
          for entry in jsondecode(aws_ecs_task_definition.betterlite.container_definitions)[0].environment :
          "${entry.name}=${entry.value}"
      ], expected)
    ])
    error_message = "The module cluster, cache, bucket and region must reach the container as environment, with STORE_MODEL_IN_DB on because a database exists."
  }

  assert {
    condition = alltrue([
      !contains([for entry in jsondecode(aws_ecs_task_definition.betterlite.container_definitions)[0].environment : entry.name], "LITELLM_OTEL_V2"),
      !contains([for entry in jsondecode(aws_ecs_task_definition.betterlite.container_definitions)[0].environment : entry.name], "LITELLM_PGBOUNCER_ENABLED"),
      !contains([for entry in jsondecode(aws_ecs_task_definition.betterlite.container_definitions)[0].environment : entry.name], "LITELLM_CONFIG_BUCKET_NAME"),
    ])
    error_message = "OTel, the connection pool and the config bucket must stay out of the environment until their variables turn them on."
  }

  assert {
    condition = alltrue([
      length(jsondecode(aws_ecs_task_definition.betterlite.container_definitions)[0].secrets) == 1,
      jsondecode(aws_ecs_task_definition.betterlite.container_definitions)[0].secrets[0].name == "LITELLM_MASTER_KEY",
      jsondecode(aws_ecs_task_definition.betterlite.container_definitions)[0].secrets[0].valueFrom == aws_secretsmanager_secret.master_key.arn,
    ])
    error_message = "The master key must arrive only as a secret valueFrom, never in the environment."
  }

  assert {
    condition     = !contains([for entry in jsondecode(aws_ecs_task_definition.betterlite.container_definitions)[0].environment : entry.name], "LITELLM_MASTER_KEY")
    error_message = "The master key must never be written into the environment list."
  }
}

run "supplied_database_and_cache_use_their_url_secrets" {
  command = plan

  variables {
    create_database = false
    database_url    = "postgresql://litellm:pw@db.internal:5432/litellm"
    create_redis    = false
    redis_url       = "rediss://:pw@cache.internal:6379"
  }

  assert {
    condition = alltrue([
      for absent in ["IAM_TOKEN_DB_AUTH", "DATABASE_HOST", "DATABASE_HOST_READ_REPLICA", "REDIS_HOST", "REDIS_SSL"] :
      !contains([for entry in jsondecode(aws_ecs_task_definition.betterlite.container_definitions)[0].environment : entry.name], absent)
    ])
    error_message = "A supplied database and cache are reached through their URLs, so no discrete host, port or IAM variables may be set."
  }

  assert {
    condition = alltrue([
      contains([for entry in jsondecode(aws_ecs_task_definition.betterlite.container_definitions)[0].environment : "${entry.name}=${entry.value}"], "STORE_MODEL_IN_DB=true"),
      contains([for entry in jsondecode(aws_ecs_task_definition.betterlite.container_definitions)[0].environment : "${entry.name}=${entry.value}"], "S3_REGION_NAME=${var.region}"),
    ])
    error_message = "An existing database still stores models in it, and the bucket variables stay in place."
  }

  assert {
    condition = alltrue([
      toset([for secret in jsondecode(aws_ecs_task_definition.betterlite.container_definitions)[0].secrets : secret.name]) == toset(["LITELLM_MASTER_KEY", "DATABASE_URL", "REDIS_URL"]),
      [for secret in jsondecode(aws_ecs_task_definition.betterlite.container_definitions)[0].secrets : secret.valueFrom if secret.name == "DATABASE_URL"] == [aws_secretsmanager_secret.database_url[0].arn],
      [for secret in jsondecode(aws_ecs_task_definition.betterlite.container_definitions)[0].secrets : secret.valueFrom if secret.name == "REDIS_URL"] == [aws_secretsmanager_secret.redis_url[0].arn],
    ])
    error_message = "DATABASE_URL and REDIS_URL must come from the module's URL secrets."
  }

  assert {
    condition = alltrue([
      length(aws_ecs_task_definition.migrations) == 1,
      length(terraform_data.migration) == 1,
      length(aws_secretsmanager_secret.db_master_password) == 0,
      length(aws_secretsmanager_secret.database_url) == 1,
      length(aws_secretsmanager_secret.redis_url) == 1,
    ])
    error_message = "An existing database still needs the migration task and its runner, and only the URL secrets exist."
  }
}

run "no_store_runs_without_database_or_cache" {
  command = plan

  variables {
    create_database     = false
    create_redis        = false
    autoscaling_enabled = false
    desired_count       = 1
    # Above one, so a process ceiling read from min_capacity instead of desired_count fires the check
    min_capacity = 2
  }

  assert {
    condition = alltrue([
      for absent in ["IAM_TOKEN_DB_AUTH", "DATABASE_HOST", "DATABASE_PORT", "STORE_MODEL_IN_DB", "REDIS_HOST", "REDIS_PORT", "REDIS_SSL"] :
      !contains([for entry in jsondecode(aws_ecs_task_definition.betterlite.container_definitions)[0].environment : entry.name], absent)
    ])
    error_message = "Without a database or cache no DATABASE_*, REDIS_* or STORE_MODEL_IN_DB variable may reach the container."
  }

  assert {
    condition = alltrue([
      length(jsondecode(aws_ecs_task_definition.betterlite.container_definitions)[0].secrets) == 1,
      [for secret in jsondecode(aws_ecs_task_definition.betterlite.container_definitions)[0].secrets : secret.name] == ["LITELLM_MASTER_KEY"],
      length(aws_ecs_task_definition.migrations) == 0,
      length(terraform_data.migration) == 0,
      length(aws_cloudwatch_log_group.migrations) == 0,
    ])
    error_message = "A stack without a database has no migration task, runner or log group, and only the master key as a secret."
  }

  assert {
    condition     = local.max_processes == 1
    error_message = "With autoscaling off the process ceiling is desired_count, so one task must leave the redis-less check quiet."
  }
}

run "caller_extra_env_replaces_the_module_value" {
  command = plan

  variables {
    otel_endpoint = "http://collector.internal:4318"
    extra_env = {
      S3_BUCKET_NAME    = "caller-bucket"
      OTEL_SERVICE_NAME = "caller-service"
      CUSTOM_FLAG       = "on"
    }
  }

  assert {
    condition     = length([for entry in jsondecode(aws_ecs_task_definition.betterlite.container_definitions)[0].environment : entry.name]) == length(distinct([for entry in jsondecode(aws_ecs_task_definition.betterlite.container_definitions)[0].environment : entry.name]))
    error_message = "ECS rejects duplicate environment names, so a caller name must replace the module's entry instead of adding a second one."
  }

  assert {
    condition = alltrue([
      [for entry in jsondecode(aws_ecs_task_definition.betterlite.container_definitions)[0].environment : entry.value if entry.name == "S3_BUCKET_NAME"] == ["caller-bucket"],
      [for entry in jsondecode(aws_ecs_task_definition.betterlite.container_definitions)[0].environment : entry.value if entry.name == "OTEL_SERVICE_NAME"] == ["caller-service"],
      [for entry in jsondecode(aws_ecs_task_definition.betterlite.container_definitions)[0].environment : entry.value if entry.name == "CUSTOM_FLAG"] == ["on"],
    ])
    error_message = "The caller's extra_env must win over both a module-set name and an OTel name, and new names must be added."
  }

  assert {
    condition     = !contains([for entry in jsondecode(aws_ecs_task_definition.betterlite.container_definitions)[0].environment : entry.value if entry.name == "S3_BUCKET_NAME"], aws_s3_bucket.this.bucket)
    error_message = "The module's bucket value must be dropped once the caller sets S3_BUCKET_NAME."
  }
}

run "supplied_secrets_and_otel_headers_reach_the_container" {
  command = plan

  variables {
    litellm_license         = "license-key"
    ui_password             = "ui-password"
    otel_headers_secret_arn = "arn:aws:secretsmanager:us-east-1:111122223333:secret:otel-headers-FFFFFF"
    extra_secrets = {
      OPENAI_API_KEY = "arn:aws:secretsmanager:us-east-1:111122223333:secret:openai-ABCDEF"
    }
  }

  assert {
    condition = alltrue([
      toset([for secret in jsondecode(aws_ecs_task_definition.betterlite.container_definitions)[0].secrets : secret.name]) == toset(["LITELLM_MASTER_KEY", "LITELLM_LICENSE", "UI_PASSWORD", "OTEL_HEADERS", "OPENAI_API_KEY"]),
      [for secret in jsondecode(aws_ecs_task_definition.betterlite.container_definitions)[0].secrets : secret.valueFrom if secret.name == "LITELLM_LICENSE"] == [aws_secretsmanager_secret.license[0].arn],
      [for secret in jsondecode(aws_ecs_task_definition.betterlite.container_definitions)[0].secrets : secret.valueFrom if secret.name == "UI_PASSWORD"] == [aws_secretsmanager_secret.ui_password[0].arn],
      [for secret in jsondecode(aws_ecs_task_definition.betterlite.container_definitions)[0].secrets : secret.valueFrom if secret.name == "OTEL_HEADERS"] == [var.otel_headers_secret_arn],
      [for secret in jsondecode(aws_ecs_task_definition.betterlite.container_definitions)[0].secrets : secret.valueFrom if secret.name == "OPENAI_API_KEY"] == [var.extra_secrets["OPENAI_API_KEY"]],
    ])
    error_message = "The license, UI password, OTel headers and caller secrets must each arrive as their own valueFrom entry."
  }
}

run "otel_block_follows_the_endpoint" {
  command = plan

  variables {
    otel_endpoint                = "http://collector.internal:4318"
    otel_exporter                = "otlp_grpc"
    otel_capture_message_content = "prompt_and_completion"
  }

  assert {
    condition = alltrue([
      for expected in [
        "LITELLM_OTEL_V2=true",
        "OTEL_EXPORTER=otlp_grpc",
        "OTEL_ENDPOINT=http://collector.internal:4318",
        "OTEL_ENVIRONMENT_NAME=${var.env}",
        "OTEL_INSTRUMENTATION_GENAI_CAPTURE_MESSAGE_CONTENT=prompt_and_completion",
        "OTEL_SERVICE_NAME=${local.name}",
        ] : contains([
          for entry in jsondecode(aws_ecs_task_definition.betterlite.container_definitions)[0].environment :
          "${entry.name}=${entry.value}"
      ], expected)
    ])
    error_message = "Setting otel_endpoint must add the whole OTel block, name the service after the stack and fall back to var.env for the environment name."
  }
}

run "otel_environment_name_overrides_env" {
  command = plan

  variables {
    otel_endpoint         = "http://collector.internal:4318"
    otel_environment_name = "prod-eu"
  }

  assert {
    condition     = [for entry in jsondecode(aws_ecs_task_definition.betterlite.container_definitions)[0].environment : entry.value if entry.name == "OTEL_ENVIRONMENT_NAME"] == ["prod-eu"]
    error_message = "A non-empty otel_environment_name must replace the var.env fallback."
  }
}

run "proxy_config_publishes_the_bucket_key_and_hash" {
  command = plan

  variables {
    proxy_config = { model_list = [] }
  }

  assert {
    condition = alltrue([
      for expected in [
        "LITELLM_CONFIG_BUCKET_NAME=${aws_s3_bucket.this.bucket}",
        "LITELLM_CONFIG_BUCKET_OBJECT_KEY=config/config.yaml",
        "LITELLM_CONFIG_BUCKET_TYPE=s3",
        "AWS_DEFAULT_REGION=${var.region}",
        "PROXY_CONFIG_HASH=${md5(yamlencode(var.proxy_config))}",
        ] : contains([
          for entry in jsondecode(aws_ecs_task_definition.betterlite.container_definitions)[0].environment :
          "${entry.name}=${entry.value}"
      ], expected)
    ])
    error_message = "A proxy_config must point the container at the uploaded object and hash the config so an edit makes a new task revision."
  }

  assert {
    condition     = length([for entry in jsondecode(aws_ecs_task_definition.betterlite.container_definitions)[0].environment : entry if entry.name == "AWS_DEFAULT_REGION"]) == 1
    error_message = "AWS_DEFAULT_REGION is what betterlite reads for the config bucket region, and it must appear exactly once."
  }
}

run "migration_task_mirrors_the_service" {
  command = plan

  assert {
    condition = alltrue([
      length(aws_ecs_task_definition.migrations) == 1,
      length(terraform_data.migration) == 1,
      aws_ecs_task_definition.migrations[0].family == "${local.name}-migrations",
      tostring(aws_ecs_task_definition.migrations[0].cpu) == "512",
      tostring(aws_ecs_task_definition.migrations[0].memory) == "1024",
      contains(aws_ecs_task_definition.migrations[0].requires_compatibilities, "FARGATE"),
      aws_cloudwatch_log_group.migrations[0].name == "/ecs/${local.name}/migrations",
    ])
    error_message = "The migration task must be a 512 CPU / 1024 MiB Fargate task in the stack's migrations log group, with its runner."
  }

  assert {
    condition = alltrue([
      jsondecode(aws_ecs_task_definition.migrations[0].container_definitions)[0].name == "migrations",
      jsondecode(aws_ecs_task_definition.migrations[0].container_definitions)[0].image == var.image,
      jsondecode(aws_ecs_task_definition.migrations[0].container_definitions)[0].entryPoint == ["betterlite-db-admin"],
      jsondecode(aws_ecs_task_definition.migrations[0].container_definitions)[0].command == ["migrate"],
      jsondecode(aws_ecs_task_definition.migrations[0].container_definitions)[0].environment == jsondecode(aws_ecs_task_definition.betterlite.container_definitions)[0].environment,
      jsondecode(aws_ecs_task_definition.migrations[0].container_definitions)[0].secrets == jsondecode(aws_ecs_task_definition.betterlite.container_definitions)[0].secrets,
    ])
    error_message = "The migration container must run betterlite-db-admin migrate from the same image with the service's environment and secrets."
  }
}

run "redis_less_check_fires_for_three_tasks" {
  command = plan

  variables {
    create_redis = false
    # Below two, so only max_capacity can push the process ceiling over the limit
    desired_count = 1
    max_capacity  = 3
  }

  expect_failures = [
    check.redis_less_rate_limits_are_per_process,
  ]
}
