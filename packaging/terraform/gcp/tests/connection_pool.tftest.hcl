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

variables {
  project_id         = "test-project"
  tenant             = "tenant"
  env                = "test"
  litellm_master_key = "sk-test-master-key"
  image              = "us-central1-docker.pkg.dev/test-project/betterlite/betterlite:test"
  allow_plaintext_lb = true
}

run "pool_variables_stay_out_by_default" {
  command = plan

  assert {
    condition = alltrue([
      length([for item in local.service_env : item.name if startswith(item.name, "LITELLM_PGBOUNCER")]) == 0,
      length([for env in google_cloud_run_v2_service.betterlite[0].template[0].containers[0].env : env.name if startswith(env.name, "LITELLM_PGBOUNCER")]) == 0,
    ])
    error_message = "connection_pool_enabled = false must put none of the LITELLM_PGBOUNCER_* variables in the service."
  }
}

run "pool_enabled_passes_the_three_variables" {
  command = plan

  variables {
    connection_pool_enabled = true
  }

  assert {
    condition = alltrue([
      length([for item in local.service_env : item.name if startswith(item.name, "LITELLM_PGBOUNCER")]) == 3,
      one([for env in google_cloud_run_v2_service.betterlite[0].template[0].containers[0].env : env.value if env.name == "LITELLM_PGBOUNCER_ENABLED"]) == "true",
      one([for env in google_cloud_run_v2_service.betterlite[0].template[0].containers[0].env : env.value if env.name == "LITELLM_PGBOUNCER_MAX_DB_CONNECTIONS"]) == "20",
      one([for env in google_cloud_run_v2_service.betterlite[0].template[0].containers[0].env : env.value if env.name == "LITELLM_PGBOUNCER_MAX_CLIENT_CONN"]) == "1000",
    ])
    error_message = "connection_pool_enabled = true must pass LITELLM_PGBOUNCER_ENABLED and the two pool limits to the service."
  }
}

run "pool_limits_follow_the_variables" {
  command = plan

  variables {
    connection_pool_enabled = true
    pool_max_db_connections = 40
    pool_max_client_conn    = 250
  }

  assert {
    condition = alltrue([
      one([for env in google_cloud_run_v2_service.betterlite[0].template[0].containers[0].env : env.value if env.name == "LITELLM_PGBOUNCER_MAX_DB_CONNECTIONS"]) == "40",
      one([for env in google_cloud_run_v2_service.betterlite[0].template[0].containers[0].env : env.value if env.name == "LITELLM_PGBOUNCER_MAX_CLIENT_CONN"]) == "250",
    ])
    error_message = "The service must carry the caller's pool limits, not the module defaults."
  }
}
