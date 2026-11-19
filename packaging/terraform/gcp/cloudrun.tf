locals {
  # the module always creates Cloud SQL, so the pool precondition guards a future create_database switch
  database_enabled = true

  service_env = concat(
    [
      { name = "DATABASE_HOST", value = google_sql_database_instance.writer.private_ip_address },
      { name = "DATABASE_PORT", value = "5432" },
      { name = "DATABASE_USER", value = var.db_username },
      { name = "DATABASE_NAME", value = var.db_name },
      { name = "DATABASE_HOST_READ_REPLICA", value = google_sql_database_instance.reader.private_ip_address },
      { name = "DATABASE_PORT_READ_REPLICA", value = "5432" },
    ],
    local.redis_enabled ? [
      { name = "REDIS_HOST", value = google_redis_instance.this[0].host },
      { name = "REDIS_PORT", value = tostring(google_redis_instance.this[0].port) },
    ] : [],
    var.redis_transit_encryption ? [
      { name = "REDIS_SSL", value = "true" },
    ] : [],
    [
      { name = "STORE_MODEL_IN_DB", value = "true" },
      { name = "GCS_BUCKET_NAME", value = google_storage_bucket.data.name },
    ],
    local.proxy_config_enabled ? [
      { name = "LITELLM_CONFIG_BUCKET_NAME", value = google_storage_bucket.config[0].name },
      { name = "LITELLM_CONFIG_BUCKET_OBJECT_KEY", value = "config.yaml" },
      { name = "LITELLM_CONFIG_BUCKET_TYPE", value = "gcs" },
      { name = "PROXY_CONFIG_HASH", value = local.proxy_config_hash },
    ] : [],
    var.connection_pool_enabled ? [
      { name = "LITELLM_PGBOUNCER_ENABLED", value = "true" },
      { name = "LITELLM_PGBOUNCER_MAX_DB_CONNECTIONS", value = tostring(var.pool_max_db_connections) },
      { name = "LITELLM_PGBOUNCER_MAX_CLIENT_CONN", value = tostring(var.pool_max_client_conn) },
    ] : [],
    var.otel_endpoint != "" ? [
      { name = "OTEL_ENDPOINT", value = var.otel_endpoint },
    ] : [],
  )

  # Cloud Run rejects two env entries with the same name, so a caller name replaces the module value
  service_env_final = concat(
    [for item in local.service_env : item if !contains(keys(var.extra_env), item.name)],
    [for name, value in var.extra_env : { name = name, value = value }],
  )

  service_env_secrets = concat(
    [
      { name = "LITELLM_MASTER_KEY", secret = google_secret_manager_secret.master_key.id, version = "latest" },
      { name = "DATABASE_PASSWORD", secret = google_secret_manager_secret.db_password.id, version = "latest" },
    ],
    local.license_set ? [
      { name = "LITELLM_LICENSE", secret = google_secret_manager_secret.license[0].id, version = "latest" },
    ] : [],
    local.ui_password_set ? [
      { name = "UI_PASSWORD", secret = google_secret_manager_secret.ui_password[0].id, version = "latest" },
    ] : [],
    [for name, secret in var.extra_secrets : { name = name, secret = secret, version = "latest" }],
  )

  migration_env = [
    { name = "DATABASE_HOST", value = google_sql_database_instance.writer.private_ip_address },
    { name = "DATABASE_PORT", value = "5432" },
    { name = "DATABASE_USER", value = var.db_username },
    { name = "DATABASE_NAME", value = var.db_name },
  ]

  migration_env_secrets = [
    { name = "DATABASE_PASSWORD", secret = google_secret_manager_secret.db_password.id, version = "latest" },
  ]
}

resource "google_cloud_run_v2_service" "betterlite" {
  count = var.create_runtime ? 1 : 0

  name                = local.name
  location            = var.region
  ingress             = "INGRESS_TRAFFIC_INTERNAL_LOAD_BALANCER"
  labels              = local.labels
  deletion_protection = false

  lifecycle {
    precondition {
      condition     = !var.connection_pool_enabled || local.database_enabled
      error_message = "connection_pool_enabled needs the Cloud SQL database this module creates."
    }
  }

  template {
    service_account                  = google_service_account.runtime.email
    max_instance_request_concurrency = var.concurrency
    timeout                          = "${var.timeout_seconds}s"

    vpc_access {
      connector = google_vpc_access_connector.this[0].id
      egress    = "PRIVATE_RANGES_ONLY"
    }

    scaling {
      min_instance_count = var.min_instances
      max_instance_count = var.max_instances
    }

    containers {
      image = var.image
      args  = ["--port", "4000"]

      ports {
        container_port = 4000
      }

      resources {
        limits = {
          cpu    = var.cpu
          memory = var.memory
        }
      }

      dynamic "env" {
        for_each = local.service_env_final
        content {
          name  = env.value.name
          value = env.value.value
        }
      }

      dynamic "env" {
        for_each = local.service_env_secrets
        content {
          name = env.value.name
          value_source {
            secret_key_ref {
              secret  = env.value.secret
              version = env.value.version
            }
          }
        }
      }

      startup_probe {
        http_get {
          path = "/health/readiness"
          port = 4000
        }
        initial_delay_seconds = 10
        period_seconds        = 10
        timeout_seconds       = 5
        failure_threshold     = 12
      }

      liveness_probe {
        http_get {
          path = "/health/liveliness"
          port = 4000
        }
        period_seconds  = 30
        timeout_seconds = 5
      }
    }
  }

  depends_on = [
    terraform_data.migration,
    google_secret_manager_secret_iam_member.master_key,
    google_secret_manager_secret_iam_member.db_password,
    google_secret_manager_secret_iam_member.license,
    google_secret_manager_secret_iam_member.ui_password,
    google_secret_manager_secret_iam_member.extra_secrets,
    google_sql_user.app,
    google_storage_bucket_iam_member.data_runtime,
    google_storage_bucket_iam_member.config_runtime,
  ]
}

# The load balancer is the gate: its NEGs may invoke the service, the internet cannot reach it directly
resource "google_cloud_run_v2_service_iam_member" "betterlite_allusers" {
  count = var.create_runtime ? 1 : 0

  project  = var.project_id
  location = google_cloud_run_v2_service.betterlite[0].location
  name     = google_cloud_run_v2_service.betterlite[0].name
  role     = "roles/run.invoker"
  member   = "allUsers"
}

resource "google_cloud_run_v2_job" "migrate" {
  count = var.create_runtime ? 1 : 0

  name                = "${local.name}-migrate"
  location            = var.region
  labels              = local.labels
  deletion_protection = false

  template {
    template {
      service_account = google_service_account.runtime.email
      max_retries     = 0

      vpc_access {
        connector = google_vpc_access_connector.this[0].id
        egress    = "PRIVATE_RANGES_ONLY"
      }

      containers {
        image   = var.image
        command = ["betterlite-db-admin"]
        args    = ["migrate"]

        dynamic "env" {
          for_each = local.migration_env
          content {
            name  = env.value.name
            value = env.value.value
          }
        }

        dynamic "env" {
          for_each = local.migration_env_secrets
          content {
            name = env.value.name
            value_source {
              secret_key_ref {
                secret  = env.value.secret
                version = env.value.version
              }
            }
          }
        }
      }
    }
  }

  depends_on = [
    google_secret_manager_secret_iam_member.db_password,
    google_sql_user.app,
  ]
}
