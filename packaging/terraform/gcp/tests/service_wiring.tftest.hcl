mock_provider "google" {
  override_during = plan

  mock_resource "google_compute_network" {
    defaults = {
      id        = "projects/test-project/global/networks/tenant-test-betterlite"
      self_link = "https://www.googleapis.com/compute/v1/projects/test-project/global/networks/tenant-test-betterlite"
    }
  }

  mock_resource "google_redis_instance" {
    defaults = {
      host = "10.0.0.4"
      port = 6379
    }
  }

  mock_resource "google_service_account" {
    defaults = {
      email = "betterlite@test-project.iam.gserviceaccount.com"
    }
  }

  # plan fills computed values only for types a mock_resource declares, so every id an assertion reads gets one
  mock_resource "google_vpc_access_connector" {
    defaults = {
      id = "projects/test-project/locations/us-central1/connectors/tenant-test-betterl-conn"
    }
  }

  mock_resource "google_cloud_run_v2_service" {
    defaults = {
      uri = "https://tenant-test-betterlite-abc123.us-central1.run.app"
    }
  }

  mock_resource "google_cloud_run_v2_job" {
    defaults = {
      id = "projects/test-project/locations/us-central1/jobs/tenant-test-betterlite-migrate"
    }
  }
}

mock_provider "random" {
  override_during = plan

  mock_resource "random_password" {}
  mock_resource "random_id" {
    defaults = {
      hex = "deadbeef"
    }
  }
}

# A mock_resource default is shared by every instance of its type, so the instances the assertions tell apart get their own
override_resource {
  target          = google_sql_database_instance.writer
  values          = { private_ip_address = "10.10.0.2" }
  override_during = plan
}

override_resource {
  target          = google_sql_database_instance.reader
  values          = { private_ip_address = "10.10.0.3" }
  override_during = plan
}

override_resource {
  target          = google_secret_manager_secret.master_key
  values          = { id = "projects/test-project/secrets/tenant-test-betterlite-master-key" }
  override_during = plan
}

override_resource {
  target          = google_secret_manager_secret.db_password
  values          = { id = "projects/test-project/secrets/tenant-test-betterlite-db-password" }
  override_during = plan
}

override_resource {
  target          = google_secret_manager_secret.license
  values          = { id = "projects/test-project/secrets/tenant-test-betterlite-license" }
  override_during = plan
}

override_resource {
  target          = google_secret_manager_secret.ui_password
  values          = { id = "projects/test-project/secrets/tenant-test-betterlite-ui-password" }
  override_during = plan
}

variables {
  project_id         = "test-project"
  tenant             = "tenant"
  env                = "test"
  litellm_master_key = "sk-test-master-key"
  image              = "us-central1-docker.pkg.dev/test-project/betterlite/betterlite:test"
  allow_plaintext_lb = true
}

run "default_service_env_and_secrets" {
  command = plan

  assert {
    condition = alltrue([
      length(google_cloud_run_v2_service.betterlite) == 1,
      google_cloud_run_v2_service.betterlite[0].name == "tenant-test-betterlite",
      google_cloud_run_v2_service.betterlite[0].location == "us-central1",
      google_cloud_run_v2_service.betterlite[0].ingress == "INGRESS_TRAFFIC_INTERNAL_LOAD_BALANCER",
      google_cloud_run_v2_service.betterlite[0].deletion_protection == false,
      google_cloud_run_v2_service.betterlite[0].template[0].service_account == google_service_account.runtime.email,
      google_cloud_run_v2_service.betterlite[0].template[0].max_instance_request_concurrency == 80,
      google_cloud_run_v2_service.betterlite[0].template[0].timeout == "300s",
      google_cloud_run_v2_service.betterlite[0].template[0].scaling[0].min_instance_count == 1,
      google_cloud_run_v2_service.betterlite[0].template[0].scaling[0].max_instance_count == 10,
      google_cloud_run_v2_service.betterlite[0].template[0].vpc_access[0].connector == google_vpc_access_connector.this[0].id,
      google_cloud_run_v2_service.betterlite[0].template[0].vpc_access[0].egress == "PRIVATE_RANGES_ONLY",
      google_cloud_run_v2_service.betterlite[0].template[0].containers[0].image == var.image,
      join(" ", google_cloud_run_v2_service.betterlite[0].template[0].containers[0].args) == "--port 4000",
      google_cloud_run_v2_service.betterlite[0].template[0].containers[0].ports[0].container_port == 4000,
      google_cloud_run_v2_service.betterlite[0].template[0].containers[0].resources[0].limits["cpu"] == "2",
      google_cloud_run_v2_service.betterlite[0].template[0].containers[0].resources[0].limits["memory"] == "4Gi",
    ])
    error_message = "The service must run one container on port 4000 with the module's defaults, the runtime account, the connector and the internal-and-LB ingress."
  }

  assert {
    condition = alltrue([
      google_cloud_run_v2_service.betterlite[0].template[0].containers[0].startup_probe[0].http_get[0].path == "/health/readiness",
      google_cloud_run_v2_service.betterlite[0].template[0].containers[0].startup_probe[0].http_get[0].port == 4000,
      google_cloud_run_v2_service.betterlite[0].template[0].containers[0].startup_probe[0].initial_delay_seconds == 10,
      google_cloud_run_v2_service.betterlite[0].template[0].containers[0].startup_probe[0].period_seconds == 10,
      google_cloud_run_v2_service.betterlite[0].template[0].containers[0].startup_probe[0].timeout_seconds == 5,
      google_cloud_run_v2_service.betterlite[0].template[0].containers[0].startup_probe[0].failure_threshold == 12,
      google_cloud_run_v2_service.betterlite[0].template[0].containers[0].liveness_probe[0].http_get[0].path == "/health/liveliness",
      google_cloud_run_v2_service.betterlite[0].template[0].containers[0].liveness_probe[0].http_get[0].port == 4000,
      google_cloud_run_v2_service.betterlite[0].template[0].containers[0].liveness_probe[0].period_seconds == 30,
      google_cloud_run_v2_service.betterlite[0].template[0].containers[0].liveness_probe[0].timeout_seconds == 5,
    ])
    error_message = "The readiness probe must poll /health/readiness every 10 seconds and the liveness probe must poll /health/liveliness on port 4000."
  }

  assert {
    condition = alltrue([
      length(local.service_env) == 10,
      contains([for item in local.service_env : item.name], "DATABASE_HOST"),
      contains([for item in local.service_env : item.name], "DATABASE_PORT"),
      contains([for item in local.service_env : item.name], "DATABASE_USER"),
      contains([for item in local.service_env : item.name], "DATABASE_NAME"),
      contains([for item in local.service_env : item.name], "DATABASE_HOST_READ_REPLICA"),
      contains([for item in local.service_env : item.name], "DATABASE_PORT_READ_REPLICA"),
      contains([for item in local.service_env : item.name], "REDIS_HOST"),
      contains([for item in local.service_env : item.name], "REDIS_PORT"),
      contains([for item in local.service_env : item.name], "STORE_MODEL_IN_DB"),
      contains([for item in local.service_env : item.name], "GCS_BUCKET_NAME"),
      one([for item in local.service_env : item.value if item.name == "DATABASE_HOST"]) == google_sql_database_instance.writer.private_ip_address,
      one([for item in local.service_env : item.value if item.name == "DATABASE_HOST_READ_REPLICA"]) == google_sql_database_instance.reader.private_ip_address,
      one([for item in local.service_env : item.value if item.name == "DATABASE_PORT"]) == "5432",
      one([for item in local.service_env : item.value if item.name == "DATABASE_PORT_READ_REPLICA"]) == "5432",
      one([for item in local.service_env : item.value if item.name == "DATABASE_USER"]) == "betterlite",
      one([for item in local.service_env : item.value if item.name == "DATABASE_NAME"]) == "betterlite",
      one([for item in local.service_env : item.value if item.name == "REDIS_HOST"]) == "10.0.0.4",
      one([for item in local.service_env : item.value if item.name == "REDIS_PORT"]) == "6379",
      one([for item in local.service_env : item.value if item.name == "STORE_MODEL_IN_DB"]) == "true",
      one([for item in local.service_env : item.value if item.name == "GCS_BUCKET_NAME"]) == google_storage_bucket.data.name,
      length([for item in local.service_env : item.name if item.name == "REDIS_SSL"]) == 0,
      length([for item in local.service_env : item.name if startswith(item.name, "LITELLM_PGBOUNCER")]) == 0,
      length([for item in local.service_env : item.name if startswith(item.name, "LITELLM_CONFIG_BUCKET")]) == 0,
      length([for item in local.service_env : item.name if item.name == "PROXY_CONFIG_HASH"]) == 0,
      length([for item in local.service_env : item.name if item.name == "OTEL_ENDPOINT"]) == 0,
    ])
    error_message = "The default environment must carry the writer and reader database settings, Redis, STORE_MODEL_IN_DB and the data bucket, and nothing from the disabled features."
  }

  assert {
    condition = alltrue([
      length(local.service_env_secrets) == 2,
      join(",", [for item in local.service_env_secrets : item.name]) == "LITELLM_MASTER_KEY,DATABASE_PASSWORD",
      one([for item in local.service_env_secrets : item.secret if item.name == "LITELLM_MASTER_KEY"]) == google_secret_manager_secret.master_key.id,
      one([for item in local.service_env_secrets : item.secret if item.name == "DATABASE_PASSWORD"]) == google_secret_manager_secret.db_password.id,
      alltrue([for item in local.service_env_secrets : item.version == "latest"]),
      length(google_cloud_run_v2_service.betterlite[0].template[0].containers[0].env) == length(local.service_env_final) + length(local.service_env_secrets),
      one([for env in google_cloud_run_v2_service.betterlite[0].template[0].containers[0].env : env.value if env.name == "DATABASE_HOST"]) == google_sql_database_instance.writer.private_ip_address,
      one([for env in google_cloud_run_v2_service.betterlite[0].template[0].containers[0].env : one(env.value_source).secret_key_ref[0].secret if env.name == "LITELLM_MASTER_KEY"]) == google_secret_manager_secret.master_key.id,
      one([for env in google_cloud_run_v2_service.betterlite[0].template[0].containers[0].env : one(env.value_source).secret_key_ref[0].secret if env.name == "DATABASE_PASSWORD"]) == google_secret_manager_secret.db_password.id,
    ])
    error_message = "The master key and the database password must reach the container as Secret Manager references, and the value entries and secret entries must together make up the container environment."
  }

  assert {
    condition = alltrue([
      length(google_cloud_run_v2_service_iam_member.betterlite_allusers) == 1,
      google_cloud_run_v2_service_iam_member.betterlite_allusers[0].project == "test-project",
      google_cloud_run_v2_service_iam_member.betterlite_allusers[0].location == "us-central1",
      google_cloud_run_v2_service_iam_member.betterlite_allusers[0].name == google_cloud_run_v2_service.betterlite[0].name,
      google_cloud_run_v2_service_iam_member.betterlite_allusers[0].role == "roles/run.invoker",
      google_cloud_run_v2_service_iam_member.betterlite_allusers[0].member == "allUsers",
    ])
    error_message = "The load balancer must be allowed to invoke the service through an allUsers roles/run.invoker binding."
  }

  assert {
    condition = alltrue([
      output.service_url == google_cloud_run_v2_service.betterlite[0].uri,
      output.cloud_run_service_name == google_cloud_run_v2_service.betterlite[0].name,
      output.migration_job_name == google_cloud_run_v2_job.migrate[0].name,
    ])
    error_message = "The runtime outputs must expose the service URL and the names of the service and the migration job."
  }
}

run "caller_extra_env_replaces_the_module_value" {
  command = plan

  variables {
    extra_env = {
      DATABASE_HOST = "db.example.internal"
      EXTRA_FLAG    = "on"
    }
  }

  assert {
    condition = alltrue([
      length(local.service_env_final) == 11,
      length([for item in local.service_env_final : item.name if item.name == "DATABASE_HOST"]) == 1,
      one([for item in local.service_env_final : item.value if item.name == "DATABASE_HOST"]) == "db.example.internal",
      one([for item in local.service_env_final : item.value if item.name == "EXTRA_FLAG"]) == "on",
      one([for env in google_cloud_run_v2_service.betterlite[0].template[0].containers[0].env : env.value if env.name == "DATABASE_HOST"]) == "db.example.internal",
      contains([for env in google_cloud_run_v2_service.betterlite[0].template[0].containers[0].env : env.name], "EXTRA_FLAG"),
    ])
    error_message = "A caller extra_env name must replace the module's entry instead of adding a second one with the same name."
  }
}

run "database_user_and_name_reach_the_service_and_the_job" {
  command = plan

  variables {
    db_username = "appuser"
    db_name     = "appdb"
  }

  assert {
    condition = alltrue([
      one([for env in google_cloud_run_v2_service.betterlite[0].template[0].containers[0].env : env.value if env.name == "DATABASE_USER"]) == "appuser",
      one([for env in google_cloud_run_v2_service.betterlite[0].template[0].containers[0].env : env.value if env.name == "DATABASE_NAME"]) == "appdb",
      one([for env in google_cloud_run_v2_service.betterlite[0].template[0].containers[0].env : env.value if env.name == "DATABASE_PORT"]) == "5432",
      one([for env in google_cloud_run_v2_job.migrate[0].template[0].template[0].containers[0].env : env.value if env.name == "DATABASE_USER"]) == "appuser",
      one([for env in google_cloud_run_v2_job.migrate[0].template[0].template[0].containers[0].env : env.value if env.name == "DATABASE_NAME"]) == "appdb",
      one([for env in google_cloud_run_v2_job.migrate[0].template[0].template[0].containers[0].env : env.value if env.name == "DATABASE_PORT"]) == "5432",
    ])
    error_message = "A caller's db_username and db_name must reach DATABASE_USER and DATABASE_NAME on the service and the migration job, with the fixed port 5432."
  }
}

run "license_and_ui_password_join_the_secrets" {
  command = plan

  variables {
    litellm_license = "license-abc"
    ui_password     = "ui-secret"
    extra_secrets = {
      OPENAI_API_KEY = "projects/test-project/secrets/openai-api-key"
    }
  }

  assert {
    condition = alltrue([
      length(local.service_env_secrets) == 5,
      contains([for item in local.service_env_secrets : item.name], "LITELLM_LICENSE"),
      contains([for item in local.service_env_secrets : item.name], "UI_PASSWORD"),
      one([for item in local.service_env_secrets : item.secret if item.name == "LITELLM_LICENSE"]) == google_secret_manager_secret.license[0].id,
      one([for item in local.service_env_secrets : item.secret if item.name == "UI_PASSWORD"]) == google_secret_manager_secret.ui_password[0].id,
      one([for item in local.service_env_secrets : item.secret if item.name == "OPENAI_API_KEY"]) == "projects/test-project/secrets/openai-api-key",
      join(",", [for item in local.service_env_secrets : item.name]) == "LITELLM_MASTER_KEY,DATABASE_PASSWORD,LITELLM_LICENSE,UI_PASSWORD,OPENAI_API_KEY",
    ])
    error_message = "A license, a UI password and every extra_secrets entry must join the master key and the database password as secret environment variables."
  }
}

run "redis_off_leaves_out_the_redis_env" {
  command = plan

  variables {
    create_redis = false
  }

  assert {
    condition = alltrue([
      length(local.service_env) == 8,
      length([for item in local.service_env : item.name if startswith(item.name, "REDIS_")]) == 0,
      length([for env in google_cloud_run_v2_service.betterlite[0].template[0].containers[0].env : env.name if startswith(env.name, "REDIS_")]) == 0,
    ])
    error_message = "Without a Memorystore instance the service must get no REDIS_HOST, REDIS_PORT or REDIS_SSL."
  }
}

run "redis_transit_encryption_turns_on_redis_ssl" {
  command = plan

  variables {
    redis_transit_encryption = true
  }

  assert {
    condition = alltrue([
      one([for item in local.service_env : item.value if item.name == "REDIS_SSL"]) == "true",
      one([for env in google_cloud_run_v2_service.betterlite[0].template[0].containers[0].env : env.value if env.name == "REDIS_SSL"]) == "true",
    ])
    error_message = "redis_transit_encryption must set REDIS_SSL=true on the service."
  }
}

run "proxy_config_adds_the_bucket_env" {
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
      contains([for item in local.service_env : item.name], "LITELLM_CONFIG_BUCKET_NAME"),
      contains([for item in local.service_env : item.name], "LITELLM_CONFIG_BUCKET_OBJECT_KEY"),
      contains([for item in local.service_env : item.name], "LITELLM_CONFIG_BUCKET_TYPE"),
      contains([for item in local.service_env : item.name], "PROXY_CONFIG_HASH"),
      one([for item in local.service_env : item.value if item.name == "LITELLM_CONFIG_BUCKET_NAME"]) == google_storage_bucket.config[0].name,
      one([for item in local.service_env : item.value if item.name == "LITELLM_CONFIG_BUCKET_OBJECT_KEY"]) == "config.yaml",
      one([for item in local.service_env : item.value if item.name == "LITELLM_CONFIG_BUCKET_TYPE"]) == "gcs",
      one([for item in local.service_env : item.value if item.name == "PROXY_CONFIG_HASH"]) == local.proxy_config_hash,
    ])
    error_message = "A proxy_config must point the service at the config bucket, the config.yaml object and the config hash."
  }
}

run "otel_endpoint_reaches_the_service" {
  command = plan

  variables {
    otel_endpoint = "https://otel.example"
  }

  assert {
    condition = alltrue([
      one([for item in local.service_env : item.value if item.name == "OTEL_ENDPOINT"]) == "https://otel.example",
      one([for env in google_cloud_run_v2_service.betterlite[0].template[0].containers[0].env : env.value if env.name == "OTEL_ENDPOINT"]) == "https://otel.example",
    ])
    error_message = "A non-empty otel_endpoint must reach the container as OTEL_ENDPOINT."
  }
}

run "migration_job_runs_betterlite_db_admin" {
  command = plan

  assert {
    condition = alltrue([
      length(google_cloud_run_v2_job.migrate) == 1,
      google_cloud_run_v2_job.migrate[0].name == "tenant-test-betterlite-migrate",
      google_cloud_run_v2_job.migrate[0].location == "us-central1",
      google_cloud_run_v2_job.migrate[0].deletion_protection == false,
      google_cloud_run_v2_job.migrate[0].template[0].template[0].containers[0].image == var.image,
      join(" ", google_cloud_run_v2_job.migrate[0].template[0].template[0].containers[0].command) == "betterlite-db-admin",
      join(" ", google_cloud_run_v2_job.migrate[0].template[0].template[0].containers[0].args) == "migrate",
      google_cloud_run_v2_job.migrate[0].template[0].template[0].max_retries == 0,
      google_cloud_run_v2_job.migrate[0].template[0].template[0].service_account == google_service_account.runtime.email,
      google_cloud_run_v2_job.migrate[0].template[0].template[0].vpc_access[0].connector == google_vpc_access_connector.this[0].id,
      length(google_cloud_run_v2_job.migrate[0].template[0].template[0].containers[0].env) == 5,
      one([for env in google_cloud_run_v2_job.migrate[0].template[0].template[0].containers[0].env : env.value if env.name == "DATABASE_HOST"]) == google_sql_database_instance.writer.private_ip_address,
      one([for env in google_cloud_run_v2_job.migrate[0].template[0].template[0].containers[0].env : one(env.value_source).secret_key_ref[0].secret if env.name == "DATABASE_PASSWORD"]) == google_secret_manager_secret.db_password.id,
      length(terraform_data.migration) == 1,
      terraform_data.migration[0].triggers_replace["job_id"] == google_cloud_run_v2_job.migrate[0].id,
      terraform_data.migration[0].triggers_replace["job_image"] == var.image,
    ])
    error_message = "The migration job must run betterlite-db-admin migrate with max_retries 0, the database environment and the password secret, and terraform_data.migration must be replaced when the job id or the image changes."
  }
}

run "sizing_region_and_labels_reach_the_runtime" {
  command = plan

  variables {
    cpu             = "4"
    memory          = "8Gi"
    min_instances   = 2
    max_instances   = 5
    concurrency     = 40
    timeout_seconds = 600
    region          = "europe-west1"
    labels          = { team = "platform" }
    lb_domains      = ["api.example.com"]
  }

  assert {
    condition = alltrue([
      google_cloud_run_v2_service.betterlite[0].location == "europe-west1",
      google_cloud_run_v2_service.betterlite[0].labels["team"] == "platform",
      google_cloud_run_v2_service.betterlite[0].template[0].max_instance_request_concurrency == 40,
      google_cloud_run_v2_service.betterlite[0].template[0].timeout == "600s",
      google_cloud_run_v2_service.betterlite[0].template[0].scaling[0].min_instance_count == 2,
      google_cloud_run_v2_service.betterlite[0].template[0].scaling[0].max_instance_count == 5,
      google_cloud_run_v2_service.betterlite[0].template[0].containers[0].resources[0].limits["cpu"] == "4",
      google_cloud_run_v2_service.betterlite[0].template[0].containers[0].resources[0].limits["memory"] == "8Gi",
      google_cloud_run_v2_job.migrate[0].location == "europe-west1",
      google_cloud_run_v2_job.migrate[0].labels["team"] == "platform",
      google_compute_region_network_endpoint_group.betterlite[0].region == "europe-west1",
      google_compute_global_address.lb[0].labels["managed_by"] == "terraform",
      google_compute_global_address.lb[0].labels["team"] == "platform",
      google_compute_global_forwarding_rule.http[0].labels["team"] == "platform",
      google_compute_global_forwarding_rule.https[0].labels["team"] == "platform",
      google_compute_backend_service.betterlite[0].timeout_sec == 600,
    ])
    error_message = "The cpu, memory, instance limits, concurrency, timeout, region and labels must reach every runtime resource they configure, with the timeout on both the service and the backend service."
  }
}

run "runtime_off_builds_no_service_job_neg_or_lb" {
  command = plan

  variables {
    create_runtime = false
    proxy_config   = { model_list = [] }
  }

  assert {
    condition = alltrue([
      length(google_cloud_run_v2_service.betterlite) == 0,
      length(google_cloud_run_v2_job.migrate) == 0,
      length(google_cloud_run_v2_service_iam_member.betterlite_allusers) == 0,
      length(terraform_data.migration) == 0,
      length(google_compute_global_address.lb) == 0,
      length(google_compute_region_network_endpoint_group.betterlite) == 0,
      length(google_compute_backend_service.betterlite) == 0,
      length(google_compute_url_map.betterlite) == 0,
      length(google_compute_url_map.https_redirect) == 0,
      length(google_compute_target_http_proxy.http) == 0,
      length(google_compute_target_https_proxy.https) == 0,
      length(google_compute_global_forwarding_rule.http) == 0,
      length(google_compute_global_forwarding_rule.https) == 0,
      length(google_compute_managed_ssl_certificate.betterlite) == 0,
      output.lb_ip == null,
      output.lb_url == null,
      output.service_url == null,
      output.cloud_run_service_name == null,
      output.migration_job_name == null,
      output.config_bucket == null,
      output.data_bucket == google_storage_bucket.data.name,
      output.redis_host == google_redis_instance.this[0].host,
      output.writer_private_ip == google_sql_database_instance.writer.private_ip_address,
      output.reader_private_ip == google_sql_database_instance.reader.private_ip_address,
      output.runtime_service_account == google_service_account.runtime.email,
      output.network_id == local.network_self_link,
      output.master_key_secret_id == google_secret_manager_secret.master_key.secret_id,
      output.db_password_secret_id == google_secret_manager_secret.db_password.secret_id,
    ])
    error_message = "create_runtime = false must build none of the runtime resources and must keep the store outputs readable, with the runtime outputs null."
  }
}

run "plaintext_lb_is_refused_without_domains_or_opt_in" {
  command = plan

  variables {
    allow_plaintext_lb = false
  }

  expect_failures = [google_compute_backend_service.betterlite]
}

run "domains_alone_allow_the_load_balancer" {
  command = plan

  variables {
    lb_domains         = ["api.example.com"]
    allow_plaintext_lb = false
  }

  assert {
    condition = alltrue([
      length(google_compute_managed_ssl_certificate.betterlite) == 1,
      google_compute_managed_ssl_certificate.betterlite[0].name == "tenant-test-betterlite-cert-1bc412bb",
    ])
    error_message = "lb_domains must satisfy the plaintext guard without allow_plaintext_lb and name the certificate after the domains."
  }
}

run "pool_enabled_with_the_database_passes_the_precondition" {
  command = plan

  variables {
    connection_pool_enabled = true
  }

  assert {
    condition = alltrue([
      local.database_enabled == true,
      contains([for item in local.service_env : item.name], "LITELLM_PGBOUNCER_ENABLED"),
    ])
    error_message = "The module always creates the database the connection pool needs, so enabling the pool must pass the service precondition."
  }
}
