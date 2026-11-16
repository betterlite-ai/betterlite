mock_provider "aws" {
  override_during = plan

  mock_data "aws_iam_policy_document" {
    defaults = {
      json = "{\"Version\":\"2012-10-17\",\"Statement\":[]}"
    }
  }

  # A plan leaves the cluster resource id unknown, and the rds-db resource ARN needs it
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

run "empty_master_key_falls_back_to_a_generated_sk_key" {
  command = plan

  assert {
    condition = alltrue([
      length(aws_secretsmanager_secret.master_key) == 1,
      aws_secretsmanager_secret.master_key.name == "${local.name}-master-key",
      aws_secretsmanager_secret.master_key.recovery_window_in_days == 0,
      random_password.master_key.length == 48,
      random_password.master_key.special == false,
      random_password.master_key.min_lower == 4,
      random_password.master_key.min_upper == 4,
      random_password.master_key.min_numeric == 4,
    ])
    error_message = "The master key secret must always exist and hold a 48-character alphanumeric generated key."
  }

  assert {
    condition     = startswith(nonsensitive(aws_secretsmanager_secret_version.master_key.secret_string), "sk-")
    error_message = "With litellm_master_key empty the secret version must hold the sk- prefixed generated key."
  }

  assert {
    condition     = nonsensitive(aws_secretsmanager_secret_version.master_key.secret_string) == "sk-${random_password.master_key.result}"
    error_message = "The generated value must be the random password behind an sk- prefix, not any other string."
  }
}

run "supplied_master_key_replaces_the_generated_one" {
  command = plan

  variables {
    litellm_master_key = "sk-operator-supplied-key"
  }

  assert {
    condition     = nonsensitive(aws_secretsmanager_secret_version.master_key.secret_string) == "sk-operator-supplied-key"
    error_message = "A supplied litellm_master_key must be written to the secret verbatim."
  }

  assert {
    condition     = !startswith(nonsensitive(aws_secretsmanager_secret_version.master_key.secret_string), "sk-${random_password.master_key.result}")
    error_message = "A supplied key must not be replaced by the generated one."
  }
}

run "empty_license_and_ui_password_create_no_secrets" {
  command = plan

  assert {
    condition = alltrue([
      length(aws_secretsmanager_secret.license) == 0,
      length(aws_secretsmanager_secret_version.license) == 0,
      length(aws_secretsmanager_secret.ui_password) == 0,
      length(aws_secretsmanager_secret_version.ui_password) == 0,
      length(aws_secretsmanager_secret.master_key) == 1,
    ])
    error_message = "Only the master key secret may exist when litellm_license and ui_password are empty."
  }
}

run "supplied_license_and_ui_password_create_their_secrets" {
  command = plan

  variables {
    litellm_license = "license-key-123"
    ui_password     = "ui-password-456"
  }

  assert {
    condition = alltrue([
      length(aws_secretsmanager_secret.license) == 1,
      aws_secretsmanager_secret.license[0].name == "${local.name}-license",
      aws_secretsmanager_secret.license[0].recovery_window_in_days == 0,
      nonsensitive(aws_secretsmanager_secret_version.license[0].secret_string) == var.litellm_license,
      length(aws_secretsmanager_secret.ui_password) == 1,
      aws_secretsmanager_secret.ui_password[0].name == "${local.name}-ui-password",
      aws_secretsmanager_secret.ui_password[0].recovery_window_in_days == 0,
      nonsensitive(aws_secretsmanager_secret_version.ui_password[0].secret_string) == var.ui_password,
    ])
    error_message = "A supplied license and UI password must each get a secret and a version holding the value."
  }
}

run "byo_urls_get_secrets_and_leave_no_master_password" {
  command = plan

  variables {
    create_database = false
    create_redis    = false
    database_url    = "postgresql://litellm:pw@db.internal:5432/litellm"
    redis_url       = "rediss://:pw@cache.internal:6379"
  }

  assert {
    condition = alltrue([
      length(aws_secretsmanager_secret.database_url) == 1,
      aws_secretsmanager_secret.database_url[0].name == "${local.name}-database-url",
      aws_secretsmanager_secret.database_url[0].recovery_window_in_days == 0,
      nonsensitive(aws_secretsmanager_secret_version.database_url[0].secret_string) == var.database_url,
      length(aws_secretsmanager_secret.redis_url) == 1,
      aws_secretsmanager_secret.redis_url[0].name == "${local.name}-redis-url",
      nonsensitive(aws_secretsmanager_secret_version.redis_url[0].secret_string) == var.redis_url,
    ])
    error_message = "A supplied database_url and redis_url must each get a secret and a version holding the value."
  }

  assert {
    condition = alltrue([
      length(aws_secretsmanager_secret.db_master_password) == 0,
      length(aws_secretsmanager_secret_version.db_master_password) == 0,
    ])
    error_message = "Without a module-created database there is no master password to store."
  }
}

run "no_byo_secrets_without_a_url" {
  command = plan

  variables {
    create_database = false
    create_redis    = false
  }

  assert {
    condition = alltrue([
      length(aws_secretsmanager_secret.database_url) == 0,
      length(aws_secretsmanager_secret_version.database_url) == 0,
      length(aws_secretsmanager_secret.redis_url) == 0,
      length(aws_secretsmanager_secret_version.redis_url) == 0,
    ])
    error_message = "Leaving database_url and redis_url empty must create no secret for either."
  }
}

run "secrets_access_policy_lists_the_runtime_secrets" {
  command = plan

  variables {
    litellm_license         = "license-key-123"
    ui_password             = "ui-password-456"
    otel_headers_secret_arn = "arn:aws:secretsmanager:us-east-1:111122223333:secret:otel-headers-AbCdEf"
    extra_secrets = {
      OPENAI_API_KEY    = "arn:aws:secretsmanager:us-east-1:111122223333:secret:openai-api-key-AbCdEf"
      ANTHROPIC_API_KEY = "arn:aws:secretsmanager:us-east-1:111122223333:secret:anthropic-key-XyZ123:apiKey::"
      OPENAI_JSON_KEY   = "arn:aws:secretsmanager:us-east-1:111122223333:secret:openai-api-key-AbCdEf:organization::"
    }
  }

  assert {
    condition = data.aws_iam_policy_document.secrets_access.statement[0].resources == toset([
      aws_secretsmanager_secret.master_key.arn,
      aws_secretsmanager_secret.license[0].arn,
      aws_secretsmanager_secret.ui_password[0].arn,
      "arn:aws:secretsmanager:us-east-1:111122223333:secret:openai-api-key-AbCdEf",
      "arn:aws:secretsmanager:us-east-1:111122223333:secret:anthropic-key-XyZ123",
      "arn:aws:secretsmanager:us-east-1:111122223333:secret:otel-headers-AbCdEf",
    ])
    error_message = "The execution role must read the master, license and UI secrets, the trimmed extra secret ARNs and the OTel headers secret, each listed once."
  }

  assert {
    condition     = alltrue([for statement in data.aws_iam_policy_document.secrets_access.statement : toset(statement.actions) == toset(["secretsmanager:GetSecretValue"])])
    error_message = "The secrets policy must grant only secretsmanager:GetSecretValue."
  }

  assert {
    condition = alltrue([
      aws_iam_role_policy_attachment.task_execution_secrets.role == aws_iam_role.task_execution.name,
      aws_iam_role_policy_attachment.task_execution_secrets.policy_arn == aws_iam_policy.secrets_access.arn,
      aws_iam_policy.secrets_access.name == "${local.name}-secrets-access",
    ])
    error_message = "The secrets policy must attach to the task execution role under the stack's name."
  }
}

run "no_otel_secret_and_no_extras_are_listed_by_default" {
  command = plan

  assert {
    condition     = data.aws_iam_policy_document.secrets_access.statement[0].resources == toset([aws_secretsmanager_secret.master_key.arn])
    error_message = "With no optional secret the policy must list only the master key secret."
  }
}

run "task_role_gets_rds_connect_and_the_assume_document" {
  command = plan

  assert {
    condition = alltrue([
      aws_iam_role.task_execution.name == "${local.name}-task-execution",
      aws_iam_role.task_execution.assume_role_policy == data.aws_iam_policy_document.task_assume.json,
      aws_iam_role.task.name == "${local.name}-task",
      aws_iam_role.task.assume_role_policy == data.aws_iam_policy_document.task_assume.json,
      aws_iam_role_policy_attachment.task_execution.policy_arn == "arn:aws:iam::aws:policy/service-role/AmazonECSTaskExecutionRolePolicy",
      aws_iam_role_policy_attachment.task_execution.role == aws_iam_role.task_execution.name,
    ])
    error_message = "Both roles must take the ecs-tasks assume document and the execution role the managed ECS task policy."
  }

  assert {
    condition     = contains([for principal in data.aws_iam_policy_document.task_assume.statement[0].principals : "${principal.type}:${join(",", principal.identifiers)}"], "Service:ecs-tasks.amazonaws.com")
    error_message = "The task roles must be assumable by ecs-tasks.amazonaws.com only."
  }

  assert {
    condition = alltrue([
      length(aws_iam_policy.rds_iam_connect) == 1,
      aws_iam_policy.rds_iam_connect[0].name == "${local.name}-rds-iam-connect",
      aws_iam_role_policy_attachment.task_rds_iam_connect[0].role == aws_iam_role.task.name,
      aws_iam_role_policy_attachment.task_rds_iam_connect[0].policy_arn == aws_iam_policy.rds_iam_connect[0].arn,
      data.aws_iam_policy_document.rds_iam_connect[0].statement[0].resources == toset(["arn:aws:rds-db:us-east-1:111122223333:dbuser:cluster-ABCDEFGHIJKLMNOPQRSTUVW/litellm_app"]),
    ])
    error_message = "The task role must gain rds-db:connect on the cluster resource id and the application user."
  }

  assert {
    condition     = length([for statement in data.aws_iam_policy_document.rds_iam_connect[0].statement : statement if toset(statement.actions) == toset(["rds-db:connect"])]) == 1
    error_message = "The rds-db policy must grant only rds-db:connect."
  }
}

run "rds_connect_policy_is_absent_without_the_cluster" {
  command = plan

  variables {
    create_database = false
    database_url    = "postgresql://litellm:pw@db.internal:5432/litellm"
  }

  assert {
    condition = alltrue([
      length(aws_iam_policy.rds_iam_connect) == 0,
      length(aws_iam_role_policy_attachment.task_rds_iam_connect) == 0,
      length(data.aws_iam_policy_document.rds_iam_connect) == 0,
    ])
    error_message = "An existing database is reached with its URL, so there is no cluster to grant rds-db:connect on."
  }
}

run "bucket_is_hardened_and_the_task_role_reads_it" {
  command = plan

  assert {
    condition = alltrue([
      aws_s3_bucket.this.bucket == "${local.name}-${random_id.s3_suffix.hex}",
      random_id.s3_suffix.byte_length == 4,
      aws_s3_bucket.this.force_destroy == false,
      aws_s3_bucket_versioning.this.versioning_configuration[0].status == "Enabled",
      one(aws_s3_bucket_server_side_encryption_configuration.this.rule).apply_server_side_encryption_by_default[0].sse_algorithm == "AES256",
      aws_s3_bucket_public_access_block.this.block_public_acls == true,
      aws_s3_bucket_public_access_block.this.block_public_policy == true,
      aws_s3_bucket_public_access_block.this.ignore_public_acls == true,
      aws_s3_bucket_public_access_block.this.restrict_public_buckets == true,
    ])
    error_message = "The bucket must be named after the stack with a four-byte suffix, and keep versioning, AES256 encryption and every public access block."
  }

  assert {
    condition = alltrue([
      aws_iam_policy.s3_access.name == "${local.name}-s3-access",
      aws_iam_role_policy_attachment.task_s3_access.role == aws_iam_role.task.name,
      aws_iam_role_policy_attachment.task_s3_access.policy_arn == aws_iam_policy.s3_access.arn,
      toset(data.aws_iam_policy_document.s3_access.statement[0].actions) == toset(["s3:ListBucket", "s3:GetBucketLocation"]),
      data.aws_iam_policy_document.s3_access.statement[0].resources == toset([aws_s3_bucket.this.arn]),
      toset(data.aws_iam_policy_document.s3_access.statement[1].actions) == toset(["s3:GetObject", "s3:PutObject", "s3:DeleteObject", "s3:AbortMultipartUpload", "s3:ListMultipartUploadParts"]),
      data.aws_iam_policy_document.s3_access.statement[1].resources == toset(["${aws_s3_bucket.this.arn}/*"]),
    ])
    error_message = "The task role must get list and location on the bucket and the five object actions on its objects."
  }
}

run "s3_force_destroy_reaches_the_bucket" {
  command = plan

  variables {
    s3_force_destroy = true
  }

  assert {
    condition     = aws_s3_bucket.this.force_destroy == true
    error_message = "s3_force_destroy = true must let a destroy drop a non-empty bucket."
  }
}

run "proxy_config_with_keys_is_uploaded_at_the_config_key" {
  command = plan

  variables {
    proxy_config = {
      model_list = [{
        model_name     = "gpt-4o"
        litellm_params = { model = "openai/gpt-4o" }
      }]
    }
  }

  assert {
    condition = alltrue([
      length(aws_s3_object.proxy_config) == 1,
      aws_s3_object.proxy_config[0].bucket == aws_s3_bucket.this.id,
      aws_s3_object.proxy_config[0].key == "config/config.yaml",
      aws_s3_object.proxy_config[0].content_type == "application/yaml",
      aws_s3_object.proxy_config[0].content == yamlencode(var.proxy_config),
    ])
    error_message = "A non-empty proxy_config must upload its YAML at config/config.yaml."
  }
}

run "empty_proxy_config_uploads_no_object" {
  command = plan

  assert {
    condition     = length(aws_s3_object.proxy_config) == 0
    error_message = "The default empty proxy_config must upload nothing."
  }
}

run "cluster_and_log_groups_exist_with_the_retention" {
  command = plan

  assert {
    condition = alltrue([
      aws_ecs_cluster.this.name == local.name,
      contains([for setting in aws_ecs_cluster.this.setting : "${setting.name}=${setting.value}"], "containerInsights=enabled"),
      aws_cloudwatch_log_group.betterlite.name == "/ecs/${local.name}/betterlite",
      aws_cloudwatch_log_group.betterlite.retention_in_days == var.log_retention_days,
      length(aws_cloudwatch_log_group.migrations) == 1,
      aws_cloudwatch_log_group.migrations[0].name == "/ecs/${local.name}/migrations",
      aws_cloudwatch_log_group.migrations[0].retention_in_days == var.log_retention_days,
    ])
    error_message = "The cluster must enable container insights and the betterlite and migrations log groups must carry the stack name and the retention variable."
  }

  assert {
    condition     = aws_cloudwatch_log_group.bootstrap_db[0].name == "/ecs/${local.name}/bootstrap-db"
    error_message = "The bootstrap log group must be named after the stack."
  }

  assert {
    condition     = alltrue([for group in aws_cloudwatch_log_group.migrations : group.retention_in_days == var.log_retention_days])
    error_message = "log_retention_days must reach the migrations log group."
  }
}

run "log_retention_variable_reaches_every_log_group" {
  command = plan

  variables {
    log_retention_days = 90
  }

  assert {
    condition = alltrue([
      aws_cloudwatch_log_group.betterlite.retention_in_days == 90,
      aws_cloudwatch_log_group.migrations[0].retention_in_days == 90,
      aws_cloudwatch_log_group.bootstrap_db[0].retention_in_days == 90,
    ])
    error_message = "A caller's log_retention_days must reach all three log groups."
  }
}

run "migrations_log_group_follows_the_database" {
  command = plan

  variables {
    create_database = false
    database_url    = "postgresql://litellm:pw@db.internal:5432/litellm"
  }

  assert {
    condition = alltrue([
      length(aws_cloudwatch_log_group.migrations) == 1,
      length(aws_cloudwatch_log_group.bootstrap_db) == 0,
    ])
    error_message = "A bring-your-own database still runs migrations, but has no cluster to bootstrap."
  }
}

run "migrations_log_group_disappears_without_a_database" {
  command = plan

  variables {
    create_database = false
    create_redis    = false
  }

  assert {
    condition = alltrue([
      length(aws_cloudwatch_log_group.migrations) == 0,
      length(aws_cloudwatch_log_group.bootstrap_db) == 0,
      length(aws_cloudwatch_log_group.betterlite) == 1,
    ])
    error_message = "With no database at all only the service log group remains."
  }
}
