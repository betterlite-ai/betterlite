locals {
  otel_enabled          = var.otel_endpoint != ""
  otel_environment_name = var.otel_environment_name != "" ? var.otel_environment_name : var.env

  managed_database_env = var.create_database ? [
    { name = "IAM_TOKEN_DB_AUTH", value = "true" },
    { name = "DATABASE_HOST", value = aws_rds_cluster.this[0].endpoint },
    { name = "DATABASE_PORT", value = tostring(aws_rds_cluster.this[0].port) },
    { name = "DATABASE_USER", value = var.db_username },
    { name = "DATABASE_NAME", value = var.db_name },
    { name = "DATABASE_HOST_READ_REPLICA", value = aws_rds_cluster.this[0].reader_endpoint },
    { name = "DATABASE_PORT_READ_REPLICA", value = tostring(aws_rds_cluster.this[0].port) },
  ] : []

  managed_redis_env = var.create_redis ? [
    { name = "REDIS_HOST", value = aws_elasticache_replication_group.this[0].primary_endpoint_address },
    { name = "REDIS_PORT", value = tostring(aws_elasticache_replication_group.this[0].port) },
    # The replication group runs with transit encryption on, so the client must speak rediss://
    { name = "REDIS_SSL", value = "true" },
  ] : []

  otel_env = local.otel_enabled ? [
    { name = "LITELLM_OTEL_V2", value = "true" },
    { name = "OTEL_EXPORTER", value = var.otel_exporter },
    { name = "OTEL_ENDPOINT", value = var.otel_endpoint },
    { name = "OTEL_ENVIRONMENT_NAME", value = local.otel_environment_name },
    { name = "OTEL_INSTRUMENTATION_GENAI_CAPTURE_MESSAGE_CONTENT", value = var.otel_capture_message_content },
    { name = "OTEL_SERVICE_NAME", value = local.name },
  ] : []

  pool_env = var.connection_pool_enabled ? [
    { name = "LITELLM_PGBOUNCER_ENABLED", value = "true" },
    { name = "LITELLM_PGBOUNCER_MAX_DB_CONNECTIONS", value = tostring(var.pool_max_db_connections) },
    { name = "LITELLM_PGBOUNCER_MAX_CLIENT_CONN", value = tostring(var.pool_max_client_conn) },
  ] : []

  proxy_config_enabled = length(keys(var.proxy_config)) > 0

  proxy_config_env = local.proxy_config_enabled ? [
    { name = "LITELLM_CONFIG_BUCKET_NAME", value = aws_s3_bucket.this.bucket },
    { name = "LITELLM_CONFIG_BUCKET_OBJECT_KEY", value = aws_s3_object.proxy_config[0].key },
    { name = "LITELLM_CONFIG_BUCKET_TYPE", value = "s3" },
    { name = "AWS_DEFAULT_REGION", value = var.region },
    # Hashing the config makes an edit a new task revision, which restarts the tasks onto it
    { name = "PROXY_CONFIG_HASH", value = md5(yamlencode(var.proxy_config)) },
  ] : []

  module_environment = concat(
    local.managed_database_env,
    local.managed_redis_env,
    [
      { name = "S3_BUCKET_NAME", value = aws_s3_bucket.this.bucket },
      { name = "S3_REGION_NAME", value = var.region },
      { name = "AWS_REGION", value = var.region },
      { name = "AWS_REGION_NAME", value = var.region },
    ],
    local.database_enabled ? [{ name = "STORE_MODEL_IN_DB", value = "true" }] : [],
    local.otel_env,
    local.pool_env,
    local.proxy_config_env,
  )

  # ECS rejects two entries with the same name, and the caller's extra_env is the one that must win
  environment = concat(
    [for entry in local.module_environment : entry if !contains(keys(var.extra_env), entry.name)],
    [for name, value in var.extra_env : { name = name, value = value }],
  )

  module_secrets = concat(
    [
      { name = "LITELLM_MASTER_KEY", valueFrom = aws_secretsmanager_secret.master_key.arn },
    ],
    var.litellm_license == "" ? [] : [
      { name = "LITELLM_LICENSE", valueFrom = aws_secretsmanager_secret.license[0].arn },
    ],
    var.ui_password == "" ? [] : [
      { name = "UI_PASSWORD", valueFrom = aws_secretsmanager_secret.ui_password[0].arn },
    ],
    local.byo_database ? [
      { name = "DATABASE_URL", valueFrom = aws_secretsmanager_secret.database_url[0].arn },
    ] : [],
    local.byo_redis ? [
      { name = "REDIS_URL", valueFrom = aws_secretsmanager_secret.redis_url[0].arn },
    ] : [],
    var.otel_headers_secret_arn == "" ? [] : [
      { name = "OTEL_HEADERS", valueFrom = var.otel_headers_secret_arn },
    ],
  )

  secrets = concat(
    local.module_secrets,
    [for name, arn in var.extra_secrets : { name = name, valueFrom = arn }],
  )

  # Without Redis every task counts its own rate limits, so the ceiling is the task count
  max_processes = var.autoscaling_enabled ? var.max_capacity : var.desired_count
}

# A check rather than a precondition: running without Redis is a legitimate choice
check "redis_less_rate_limits_are_per_process" {
  assert {
    condition     = local.redis_enabled || local.max_processes <= 1
    error_message = "No Redis is configured while the service can run up to ${local.max_processes} tasks, so per-key RPM/TPM limits, budgets and cooldowns apply per task and a caller can multiply them across tasks. Set `create_redis = true`, pass `redis_url`, or hold the service to one task (`autoscaling_enabled = false`, `desired_count = 1`)."
  }
}
