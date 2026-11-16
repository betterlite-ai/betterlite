mock_provider "google" {}
mock_provider "random" {}

variables {
  project_id = "test-project"
  tenant     = "tenant"
  env        = "test"
}

run "default_creates_the_network_and_stores" {
  command = plan

  assert {
    condition = alltrue([
      length(google_compute_network.this) == 1,
      length(google_compute_subnetwork.this) == 1,
      length(google_compute_global_address.psa) == 1,
      length(google_service_networking_connection.psa) == 1,
      length(google_vpc_access_connector.this) == 1,
      length(google_sql_database_instance.writer) == 1,
      length(google_sql_database_instance.reader) == 1,
      length(google_redis_instance.this) == 1,
      length(google_secret_manager_secret.db_password) == 1,
    ])
    error_message = "The default mode must create the network, private services access, the connector, both Cloud SQL instances, Redis and the password secret."
  }

  assert {
    condition = alltrue([
      google_compute_network.this[0].auto_create_subnetworks == false,
      google_compute_network.this[0].routing_mode == "REGIONAL",
      google_compute_subnetwork.this[0].private_ip_google_access == true,
      google_compute_subnetwork.this[0].ip_cidr_range == "10.20.0.0/24",
      google_compute_global_address.psa[0].purpose == "VPC_PEERING",
      google_compute_global_address.psa[0].prefix_length == 16,
      google_vpc_access_connector.this[0].ip_cidr_range == "10.20.1.0/28",
      google_vpc_access_connector.this[0].machine_type == "e2-micro",
      google_vpc_access_connector.this[0].min_instances == 2,
      google_vpc_access_connector.this[0].max_instances == 3,
    ])
    error_message = "The network resources must carry the private-services and connector settings."
  }

  assert {
    condition = alltrue([
      google_sql_database_instance.writer.name == "tenant-test-betterlite",
      google_sql_database_instance.writer.database_version == "POSTGRES_16",
      google_sql_database_instance.writer.settings[0].edition == "ENTERPRISE",
      google_sql_database_instance.writer.settings[0].tier == "db-custom-2-7680",
      google_sql_database_instance.writer.settings[0].availability_type == "REGIONAL",
      google_sql_database_instance.writer.settings[0].disk_size == 20,
      google_sql_database_instance.writer.settings[0].disk_autoresize == true,
      google_sql_database_instance.writer.settings[0].backup_configuration[0].enabled == true,
      google_sql_database_instance.writer.settings[0].backup_configuration[0].point_in_time_recovery_enabled == true,
      google_sql_database_instance.writer.settings[0].backup_configuration[0].start_time == "07:00",
      google_sql_database_instance.writer.settings[0].insights_config[0].query_insights_enabled == true,
      google_sql_database_instance.writer.settings[0].ip_configuration[0].ipv4_enabled == false,
      google_sql_database_instance.writer.deletion_protection == true,
      google_sql_database_instance.reader.name == "tenant-test-betterlite-reader",
      google_sql_database_instance.reader.master_instance_name == "tenant-test-betterlite",
      google_sql_database_instance.reader.settings[0].availability_type == "ZONAL",
      google_sql_database_instance.reader.deletion_protection == true,
    ])
    error_message = "Cloud SQL must keep a REGIONAL writer with backups and private networking and a ZONAL reader that follows the writer."
  }

  assert {
    condition = alltrue([
      google_sql_database.this.name == "betterlite",
      google_sql_user.app.name == "betterlite",
      google_secret_manager_secret.db_password.secret_id == "tenant-test-betterlite-db-password",
      google_redis_instance.this[0].tier == "STANDARD_HA",
      google_redis_instance.this[0].memory_size_gb == 1,
      google_redis_instance.this[0].connect_mode == "PRIVATE_SERVICE_ACCESS",
      google_redis_instance.this[0].redis_version == "REDIS_7_0",
      google_redis_instance.this[0].auth_enabled == true,
      local.labels["managed_by"] == "terraform",
    ])
    error_message = "The database, its user, the password secret, the module label and Memorystore must keep the module defaults."
  }
}

run "caller_labels_merge_with_the_module_label" {
  command = plan

  variables {
    labels = { team = "platform" }
  }

  assert {
    condition     = local.labels["managed_by"] == "terraform" && local.labels["team"] == "platform"
    error_message = "Caller labels must merge with the module's managed_by label."
  }
}

run "runtime_off_keeps_the_stores_and_drops_the_connector" {
  command = plan

  variables {
    create_runtime = false
  }

  assert {
    condition     = length(google_vpc_access_connector.this) == 0
    error_message = "create_runtime = false must not build a VPC connector."
  }

  assert {
    condition = alltrue([
      length(google_compute_network.this) == 1,
      length(google_compute_subnetwork.this) == 1,
      length(google_service_networking_connection.psa) == 1,
      length(google_sql_database_instance.writer) == 1,
      length(google_sql_database_instance.reader) == 1,
      length(google_redis_instance.this) == 1,
    ])
    error_message = "create_runtime = false must keep the network and both data stores for a caller that runs the service elsewhere."
  }
}

run "existing_network_attaches_the_stores" {
  command = plan

  variables {
    network_id            = "projects/host-project/global/networks/shared"
    create_psa_connection = false
  }

  assert {
    condition = alltrue([
      length(google_compute_network.this) == 0,
      length(google_compute_subnetwork.this) == 0,
      length(google_compute_global_address.psa) == 0,
      length(google_service_networking_connection.psa) == 0,
      local.network_self_link == var.network_id,
      local.network_name == "shared",
    ])
    error_message = "A supplied network_id must skip the VPC, the subnet and the private services access resources."
  }

  assert {
    condition = alltrue([
      google_sql_database_instance.writer.settings[0].ip_configuration[0].private_network == var.network_id,
      google_sql_database_instance.reader.settings[0].ip_configuration[0].private_network == var.network_id,
      google_redis_instance.this[0].authorized_network == var.network_id,
    ])
    error_message = "Cloud SQL and Memorystore must attach to the supplied network."
  }
}

run "no_private_services_access_without_a_network_fails" {
  command = plan

  variables {
    create_psa_connection = false
  }

  expect_failures = [google_sql_database_instance.writer]
}

run "redis_off_builds_no_instance" {
  command = plan

  variables {
    create_redis = false
  }

  assert {
    condition     = length(google_redis_instance.this) == 0 && local.redis_enabled == false
    error_message = "create_redis = false must build no Memorystore instance and clear redis_enabled."
  }

  assert {
    condition     = length(google_sql_database_instance.writer) == 1
    error_message = "Turning Redis off must not touch Cloud SQL."
  }
}

run "redis_transit_encryption_defaults_to_disabled" {
  command = plan

  assert {
    condition     = google_redis_instance.this[0].transit_encryption_mode == "DISABLED"
    error_message = "Redis transit encryption must default to DISABLED because betterlite cannot read a server CA yet."
  }
}

run "redis_transit_encryption_opts_in_to_server_authentication" {
  command = plan

  variables {
    redis_transit_encryption = true
  }

  assert {
    condition     = google_redis_instance.this[0].transit_encryption_mode == "SERVER_AUTHENTICATION"
    error_message = "redis_transit_encryption = true must turn on server authentication."
  }
}

run "long_names_are_cut_to_thirty_characters" {
  command = plan

  variables {
    tenant = "abcdefghijklmnopqrst"
    env    = "abcdefgh"
  }

  assert {
    condition     = local.name == "abcdefghijklmnopqrst-abcdefgh"
    error_message = "A name longer than 30 characters must be cut and must not keep the hyphen the cut lands on."
  }

  assert {
    condition     = length(local.name) <= 30 && !endswith(local.name, "-")
    error_message = "Every resource name must stay at or below 30 characters and must not end with a hyphen."
  }
}
