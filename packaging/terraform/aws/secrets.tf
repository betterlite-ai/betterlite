resource "random_password" "master_key" {
  length      = 48
  special     = false
  min_lower   = 4
  min_upper   = 4
  min_numeric = 4
}

# The generated fallback keeps the sk- prefix the proxy gives its own keys, so an operator-supplied key and a generated one read the same
resource "aws_secretsmanager_secret" "master_key" {
  name                    = "${local.name}-master-key"
  description             = "LITELLM_MASTER_KEY for the proxy."
  recovery_window_in_days = 0

  tags = local.tags
}

resource "aws_secretsmanager_secret_version" "master_key" {
  secret_id     = aws_secretsmanager_secret.master_key.id
  secret_string = coalesce(var.litellm_master_key, "sk-${random_password.master_key.result}")
}

resource "aws_secretsmanager_secret" "license" {
  count = var.litellm_license == "" ? 0 : 1

  name                    = "${local.name}-license"
  description             = "LITELLM_LICENSE for the proxy."
  recovery_window_in_days = 0

  tags = local.tags
}

resource "aws_secretsmanager_secret_version" "license" {
  count = var.litellm_license == "" ? 0 : 1

  secret_id     = aws_secretsmanager_secret.license[0].id
  secret_string = var.litellm_license
}

resource "aws_secretsmanager_secret" "ui_password" {
  count = var.ui_password == "" ? 0 : 1

  name                    = "${local.name}-ui-password"
  description             = "UI_PASSWORD for the admin UI."
  recovery_window_in_days = 0

  tags = local.tags
}

resource "aws_secretsmanager_secret_version" "ui_password" {
  count = var.ui_password == "" ? 0 : 1

  secret_id     = aws_secretsmanager_secret.ui_password[0].id
  secret_string = var.ui_password
}

# The bootstrap task reads one JSON field out of this secret; the proxy never reads it, since it authenticates to Aurora with an IAM token
resource "aws_secretsmanager_secret" "db_master_password" {
  count                   = var.create_database ? 1 : 0
  name                    = "${local.name}-db-master-password"
  description             = "Aurora master-user password, used only to create the IAM-authenticated application user."
  recovery_window_in_days = 0

  tags = local.tags
}

resource "aws_secretsmanager_secret_version" "db_master_password" {
  count     = var.create_database ? 1 : 0
  secret_id = aws_secretsmanager_secret.db_master_password[0].id
  secret_string = jsonencode({
    username = var.db_master_username
    password = random_password.db_master_password[0].result
    host     = aws_rds_cluster.this[0].endpoint
    port     = aws_rds_cluster.this[0].port
    dbname   = var.db_name
  })
}

resource "aws_secretsmanager_secret" "database_url" {
  count = local.byo_database ? 1 : 0

  name                    = "${local.name}-database-url"
  description             = "DATABASE_URL for an existing Postgres, used when create_database = false."
  recovery_window_in_days = 0

  tags = local.tags
}

resource "aws_secretsmanager_secret_version" "database_url" {
  count = local.byo_database ? 1 : 0

  secret_id     = aws_secretsmanager_secret.database_url[0].id
  secret_string = var.database_url
}

resource "aws_secretsmanager_secret" "redis_url" {
  count = local.byo_redis ? 1 : 0

  name                    = "${local.name}-redis-url"
  description             = "REDIS_URL for an existing Redis, used when create_redis = false."
  recovery_window_in_days = 0

  tags = local.tags
}

resource "aws_secretsmanager_secret_version" "redis_url" {
  count = local.byo_redis ? 1 : 0

  secret_id     = aws_secretsmanager_secret.redis_url[0].id
  secret_string = var.redis_url
}
