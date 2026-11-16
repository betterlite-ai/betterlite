mock_provider "aws" {
  override_during = plan

  mock_data "aws_iam_policy_document" {
    defaults = {
      json = "{\"Version\":\"2012-10-17\",\"Statement\":[]}"
    }
  }

  # The task definition embeds the cluster endpoint and port, and the secret JSON embeds both
  mock_resource "aws_rds_cluster" {
    defaults = {
      cluster_resource_id = "cluster-ABCDEFGHIJKLMNOPQRSTUVW"
      endpoint            = "acme-litellm-test.cluster-cdefghijklmn.us-east-1.rds.amazonaws.com"
      port                = 5432
    }
  }

  mock_data "aws_caller_identity" {
    defaults = {
      account_id = "111122223333"
    }
  }
}

mock_provider "random" {
  override_during = plan
}

variables {
  region = "us-east-1"
  tenant = "acme"
  env    = "test"
  azs    = ["us-east-1a", "us-east-1b"]
}

run "create_database_builds_one_bootstrap_task_and_one_runner" {
  command = plan

  assert {
    condition = alltrue([
      length(aws_ecs_task_definition.bootstrap_db) == 1,
      length(terraform_data.bootstrap_db) == 1,
      length(aws_cloudwatch_log_group.bootstrap_db) == 1,
      length(aws_iam_policy.bootstrap_secrets) == 1,
      length(aws_iam_role_policy_attachment.task_execution_bootstrap_secrets) == 1,
      length(aws_secretsmanager_secret.db_master_password) == 1,
      length(aws_secretsmanager_secret_version.db_master_password) == 1,
    ])
    error_message = "create_database = true must build the bootstrap task, its runner, its log group, its secret and the policy that reads it."
  }

  assert {
    condition = alltrue([
      aws_ecs_task_definition.bootstrap_db[0].family == "${local.name}-bootstrap-db",
      aws_ecs_task_definition.bootstrap_db[0].network_mode == "awsvpc",
      tostring(aws_ecs_task_definition.bootstrap_db[0].cpu) == "256",
      tostring(aws_ecs_task_definition.bootstrap_db[0].memory) == "512",
      contains(aws_ecs_task_definition.bootstrap_db[0].requires_compatibilities, "FARGATE"),
      aws_ecs_task_definition.bootstrap_db[0].execution_role_arn == aws_iam_role.task_execution.arn,
      aws_ecs_task_definition.bootstrap_db[0].task_role_arn == aws_iam_role.task.arn,
    ])
    error_message = "The bootstrap task must be a 256 CPU and 512 MiB Fargate task on the stack's two roles."
  }

  assert {
    condition = alltrue([
      aws_cloudwatch_log_group.bootstrap_db[0].name == "/ecs/${local.name}/bootstrap-db",
      aws_cloudwatch_log_group.bootstrap_db[0].retention_in_days == var.log_retention_days,
      aws_iam_role_policy_attachment.task_execution_bootstrap_secrets[0].role == aws_iam_role.task_execution.name,
      aws_iam_role_policy_attachment.task_execution_bootstrap_secrets[0].policy_arn == aws_iam_policy.bootstrap_secrets[0].arn,
      jsondecode(aws_iam_policy.bootstrap_secrets[0].policy).Statement[0].Effect == "Allow",
      jsondecode(aws_iam_policy.bootstrap_secrets[0].policy).Statement[0].Action == ["secretsmanager:GetSecretValue"],
      jsondecode(aws_iam_policy.bootstrap_secrets[0].policy).Statement[0].Resource == [aws_secretsmanager_secret.db_master_password[0].arn],
    ])
    error_message = "The bootstrap policy must let the execution role read only the master password secret, and the log group must carry the stack name and retention."
  }
}

run "bootstrap_container_runs_psql_with_the_sql" {
  command = plan

  assert {
    condition = alltrue([
      jsondecode(aws_ecs_task_definition.bootstrap_db[0].container_definitions)[0].name == "psql",
      jsondecode(aws_ecs_task_definition.bootstrap_db[0].container_definitions)[0].image == "postgres:16-alpine",
      jsondecode(aws_ecs_task_definition.bootstrap_db[0].container_definitions)[0].essential == true,
      jsondecode(aws_ecs_task_definition.bootstrap_db[0].container_definitions)[0].entryPoint == ["sh", "-c"],
      jsondecode(aws_ecs_task_definition.bootstrap_db[0].container_definitions)[0].command == ["echo \"$BOOTSTRAP_SQL\" | psql -v ON_ERROR_STOP=1"],
      jsondecode(aws_ecs_task_definition.bootstrap_db[0].container_definitions)[0].logConfiguration.options["awslogs-group"] == "/ecs/${local.name}/bootstrap-db",
      jsondecode(aws_ecs_task_definition.bootstrap_db[0].container_definitions)[0].logConfiguration.options["awslogs-region"] == var.region,
      jsondecode(aws_ecs_task_definition.bootstrap_db[0].container_definitions)[0].logConfiguration.options["awslogs-stream-prefix"] == "bootstrap",
    ])
    error_message = "The bootstrap container must run psql from postgres:16-alpine against the bootstrap log group, stopping on the first SQL error."
  }

  assert {
    condition = alltrue([
      for expected in [
        "PGHOST=${aws_rds_cluster.this[0].endpoint}",
        "PGPORT=${tostring(aws_rds_cluster.this[0].port)}",
        "PGUSER=${var.db_master_username}",
        "PGDATABASE=${var.db_name}",
        "BOOTSTRAP_SQL=${local.bootstrap_sql}",
        ] : contains([
          for variable in jsondecode(aws_ecs_task_definition.bootstrap_db[0].container_definitions)[0].environment :
          "${variable.name}=${variable.value}"
      ], expected)
    ])
    error_message = "The container must take the cluster host, port, master user, database name and the bootstrap SQL from the module."
  }

  assert {
    condition     = jsondecode(aws_ecs_task_definition.bootstrap_db[0].container_definitions)[0].secrets[0].name == "PGPASSWORD" && endswith(jsondecode(aws_ecs_task_definition.bootstrap_db[0].container_definitions)[0].secrets[0].valueFrom, ":password::") && startswith(jsondecode(aws_ecs_task_definition.bootstrap_db[0].container_definitions)[0].secrets[0].valueFrom, aws_secretsmanager_secret.db_master_password[0].arn)
    error_message = "PGPASSWORD must come from the password field of the master password secret."
  }
}

run "bootstrap_sql_creates_the_iam_user_and_grants" {
  command = plan

  assert {
    condition = alltrue([
      strcontains(local.bootstrap_sql, "CREATE USER ${var.db_username}"),
      strcontains(local.bootstrap_sql, "EXCEPTION WHEN duplicate_object THEN NULL"),
      strcontains(local.bootstrap_sql, "GRANT rds_iam TO ${var.db_username}"),
      strcontains(local.bootstrap_sql, "GRANT ALL PRIVILEGES ON DATABASE ${var.db_name} TO ${var.db_username}"),
      strcontains(local.bootstrap_sql, "GRANT ALL ON SCHEMA public TO ${var.db_username}"),
      strcontains(local.bootstrap_sql, "ALTER DEFAULT PRIVILEGES IN SCHEMA public GRANT ALL ON TABLES TO ${var.db_username}"),
      strcontains(local.bootstrap_sql, "ALTER DEFAULT PRIVILEGES IN SCHEMA public GRANT ALL ON SEQUENCES TO ${var.db_username}"),
    ])
    error_message = "The bootstrap SQL must create the user idempotently, grant rds_iam, the database, the schema and the default privileges."
  }

  assert {
    condition     = strcontains(local.bootstrap_sql, "DO $$") && strcontains(local.bootstrap_sql, "END $$;")
    error_message = "The CREATE USER must sit in a DO block so a second apply does not fail on the existing user."
  }
}

run "master_password_secret_holds_the_connection_fields" {
  command = plan

  assert {
    condition = alltrue([
      jsondecode(nonsensitive(aws_secretsmanager_secret_version.db_master_password[0].secret_string))["username"] == var.db_master_username,
      jsondecode(nonsensitive(aws_secretsmanager_secret_version.db_master_password[0].secret_string))["password"] == nonsensitive(random_password.db_master_password[0].result),
      jsondecode(nonsensitive(aws_secretsmanager_secret_version.db_master_password[0].secret_string))["host"] == aws_rds_cluster.this[0].endpoint,
      jsondecode(nonsensitive(aws_secretsmanager_secret_version.db_master_password[0].secret_string))["port"] == aws_rds_cluster.this[0].port,
      jsondecode(nonsensitive(aws_secretsmanager_secret_version.db_master_password[0].secret_string))["dbname"] == var.db_name,
    ])
    error_message = "The master password secret must be the JSON ECS reads the password field out of, carrying the cluster host, port and database."
  }

  assert {
    condition     = aws_secretsmanager_secret.db_master_password[0].name == "${local.name}-db-master-password" && aws_secretsmanager_secret.db_master_password[0].recovery_window_in_days == 0
    error_message = "The master password secret must be named after the stack and deleted at once on destroy."
  }
}

run "runner_triggers_on_the_cluster_and_the_task_revision" {
  command = plan

  assert {
    condition = alltrue([
      terraform_data.bootstrap_db[0].triggers_replace["cluster_resource_id"] == aws_rds_cluster.this[0].cluster_resource_id,
      tostring(terraform_data.bootstrap_db[0].triggers_replace["task_def_revision"]) == tostring(aws_ecs_task_definition.bootstrap_db[0].revision),
      length(keys(terraform_data.bootstrap_db[0].triggers_replace)) == 2,
    ])
    error_message = "A new cluster or a new task definition revision must re-run the bootstrap."
  }
}

run "without_create_database_there_is_no_bootstrap" {
  command = plan

  variables {
    create_database = false
    database_url    = "postgresql://litellm:pw@db.internal:5432/litellm"
  }

  assert {
    condition = alltrue([
      length(aws_ecs_task_definition.bootstrap_db) == 0,
      length(terraform_data.bootstrap_db) == 0,
      length(aws_cloudwatch_log_group.bootstrap_db) == 0,
      length(aws_iam_policy.bootstrap_secrets) == 0,
      length(aws_iam_role_policy_attachment.task_execution_bootstrap_secrets) == 0,
      length(aws_secretsmanager_secret.db_master_password) == 0,
      length(aws_secretsmanager_secret_version.db_master_password) == 0,
    ])
    error_message = "An existing database is already usable through its URL, so no bootstrap task, runner, log group, secret or policy may exist."
  }

  assert {
    condition     = length(aws_secretsmanager_secret.database_url) == 1
    error_message = "The supplied database_url must still reach the proxy through its secret."
  }
}
