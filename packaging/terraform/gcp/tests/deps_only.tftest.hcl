mock_provider "google" {
  override_during = plan
}

mock_provider "random" {
  override_during = plan
}

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
      length(google_redis_instance.this) == 1,
      google_compute_network.this[0].name == "tenant-test-betterlite",
      google_compute_subnetwork.this[0].name == "tenant-test-betterlite-us-central1",
      google_compute_global_address.psa[0].name == "tenant-test-betterlite-psa",
      google_sql_database_instance.writer.name == "tenant-test-betterlite",
      google_sql_database_instance.reader.name == "tenant-test-betterlite-reader",
      google_secret_manager_secret.db_password.secret_id == "tenant-test-betterlite-db-password",
      local.network_name == "tenant-test-betterlite",
      local.redis_enabled == true,
    ])
    error_message = "The default mode must create the network, private services access, the connector, both Cloud SQL instances, Redis and the password secret."
  }

  assert {
    condition = alltrue([
      google_compute_network.this[0].auto_create_subnetworks == false,
      google_compute_network.this[0].routing_mode == "REGIONAL",
      google_compute_subnetwork.this[0].private_ip_google_access == true,
      google_compute_subnetwork.this[0].ip_cidr_range == "10.20.0.0/24",
      google_compute_subnetwork.this[0].region == "us-central1",
      google_compute_subnetwork.this[0].network == google_compute_network.this[0].id,
      google_compute_global_address.psa[0].purpose == "VPC_PEERING",
      google_compute_global_address.psa[0].address_type == "INTERNAL",
      google_compute_global_address.psa[0].prefix_length == 16,
      google_compute_global_address.psa[0].network == local.network_self_link,
      google_service_networking_connection.psa[0].network == local.network_self_link,
      google_service_networking_connection.psa[0].service == "servicenetworking.googleapis.com",
      google_service_networking_connection.psa[0].reserved_peering_ranges[0] == google_compute_global_address.psa[0].name,
      google_vpc_access_connector.this[0].region == "us-central1",
      google_vpc_access_connector.this[0].ip_cidr_range == "10.20.1.0/28",
      google_vpc_access_connector.this[0].machine_type == "e2-micro",
      google_vpc_access_connector.this[0].min_instances == 2,
      google_vpc_access_connector.this[0].max_instances == 3,
    ])
    error_message = "The network resources must carry the private-services and connector settings."
  }

  assert {
    condition = alltrue([
      google_sql_database_instance.writer.region == "us-central1",
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
      google_sql_database_instance.writer.settings[0].insights_config[0].record_application_tags == true,
      google_sql_database_instance.writer.settings[0].insights_config[0].record_client_address == true,
      google_sql_database_instance.writer.settings[0].ip_configuration[0].ipv4_enabled == false,
      google_sql_database_instance.writer.settings[0].ip_configuration[0].private_network == local.network_self_link,
      google_sql_database_instance.writer.deletion_protection == true,
      google_sql_database_instance.reader.region == "us-central1",
      google_sql_database_instance.reader.database_version == "POSTGRES_16",
      google_sql_database_instance.reader.master_instance_name == "tenant-test-betterlite",
      google_sql_database_instance.reader.settings[0].availability_type == "ZONAL",
      google_sql_database_instance.reader.settings[0].ip_configuration[0].ipv4_enabled == false,
      google_sql_database_instance.reader.settings[0].ip_configuration[0].private_network == local.network_self_link,
      google_sql_database_instance.reader.deletion_protection == true,
    ])
    error_message = "Cloud SQL must keep a REGIONAL writer with backups and private networking and a ZONAL reader that follows the writer."
  }

  assert {
    condition = alltrue([
      google_sql_database.this.name == "betterlite",
      google_sql_database.this.instance == google_sql_database_instance.writer.name,
      google_sql_database.this.deletion_policy == "ABANDON",
      google_sql_user.app.name == "betterlite",
      google_sql_user.app.instance == google_sql_database_instance.writer.name,
      google_sql_user.app.password == random_password.db_password.result,
      google_sql_user.app.deletion_policy == "ABANDON",
      google_secret_manager_secret_version.db_password.secret == google_secret_manager_secret.db_password.id,
      google_secret_manager_secret_version.db_password.secret_data == random_password.db_password.result,
      google_redis_instance.this[0].name == "tenant-test-betterlite",
      google_redis_instance.this[0].region == "us-central1",
      google_redis_instance.this[0].tier == "STANDARD_HA",
      google_redis_instance.this[0].memory_size_gb == 1,
      google_redis_instance.this[0].connect_mode == "PRIVATE_SERVICE_ACCESS",
      google_redis_instance.this[0].redis_version == "REDIS_7_0",
      google_redis_instance.this[0].authorized_network == local.network_self_link,
      google_redis_instance.this[0].auth_enabled == true,
      local.labels["managed_by"] == "terraform",
    ])
    error_message = "The database, its user and generated password, the password secret, the module label and Memorystore must keep the module defaults."
  }

  assert {
    condition = alltrue([
      random_password.db_password.length == 32,
      random_password.db_password.special == false,
      random_password.db_password.min_lower == 4,
      random_password.db_password.min_upper == 4,
      random_password.db_password.min_numeric == 4,
    ])
    error_message = "The generated password must stay 32 characters of letters and digits with no special characters."
  }
}

run "invalid_tenant_slug_fails" {
  command = plan

  variables {
    tenant = "Tenant_1"
  }

  expect_failures = [var.tenant]
}

run "invalid_env_slug_fails" {
  command = plan

  variables {
    env = "production"
  }

  expect_failures = [var.env]
}

run "unknown_database_edition_fails" {
  command = plan

  variables {
    db_edition = "STANDARD"
  }

  expect_failures = [var.db_edition]
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

  assert {
    condition = alltrue([
      for labels in [
        google_sql_database_instance.writer.settings[0].user_labels,
        google_sql_database_instance.reader.settings[0].user_labels,
        google_redis_instance.this[0].labels,
        google_secret_manager_secret.db_password.labels,
      ] : labels["team"] == "platform" && labels["managed_by"] == "terraform"
    ])
    error_message = "The merged labels must reach both Cloud SQL instances, Memorystore and the password secret."
  }
}

run "caller_variables_reach_every_resource" {
  command = plan

  variables {
    subnet_cidr                  = "10.30.0.0/24"
    vpc_connector_cidr           = "10.30.1.0/28"
    db_edition                   = "ENTERPRISE_PLUS"
    db_tier                      = "db-perf-optimized-N-2"
    db_version                   = "POSTGRES_15"
    db_name                      = "appdb"
    db_username                  = "appuser"
    cloudsql_deletion_protection = false
    redis_tier                   = "BASIC"
    redis_memory_size_gb         = 4
  }

  assert {
    condition = alltrue([
      google_compute_subnetwork.this[0].ip_cidr_range == "10.30.0.0/24",
      google_vpc_access_connector.this[0].ip_cidr_range == "10.30.1.0/28",
      google_sql_database_instance.writer.database_version == "POSTGRES_15",
      google_sql_database_instance.writer.settings[0].edition == "ENTERPRISE_PLUS",
      google_sql_database_instance.writer.settings[0].tier == "db-perf-optimized-N-2",
      google_sql_database_instance.writer.deletion_protection == false,
      google_sql_database_instance.reader.database_version == "POSTGRES_15",
      google_sql_database_instance.reader.settings[0].edition == "ENTERPRISE_PLUS",
      google_sql_database_instance.reader.settings[0].tier == "db-perf-optimized-N-2",
      google_sql_database_instance.reader.deletion_protection == false,
      google_sql_database.this.name == "appdb",
      google_sql_user.app.name == "appuser",
      google_redis_instance.this[0].tier == "BASIC",
      google_redis_instance.this[0].memory_size_gb == 4,
    ])
    error_message = "Every caller-set variable must reach the resources it configures."
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
      google_sql_database_instance.writer.name == "tenant-test-betterlite",
      google_sql_database_instance.reader.name == "tenant-test-betterlite-reader",
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
      google_vpc_access_connector.this[0].network == var.network_id,
      google_sql_database_instance.writer.settings[0].ip_configuration[0].private_network == var.network_id,
      google_sql_database_instance.reader.settings[0].ip_configuration[0].private_network == var.network_id,
      google_redis_instance.this[0].authorized_network == var.network_id,
    ])
    error_message = "The connector, Cloud SQL and Memorystore must attach to the supplied network, project included."
  }
}

run "supplied_network_with_private_services_access_peers_with_it" {
  command = plan

  variables {
    network_id = "projects/host-project/global/networks/shared"
  }

  assert {
    condition = alltrue([
      length(google_compute_network.this) == 0,
      length(google_compute_subnetwork.this) == 0,
      length(google_compute_global_address.psa) == 1,
      length(google_service_networking_connection.psa) == 1,
      google_compute_global_address.psa[0].network == var.network_id,
      google_service_networking_connection.psa[0].network == var.network_id,
    ])
    error_message = "A supplied network_id must still peer that network when private services access is created in it."
  }

  assert {
    condition = alltrue([
      google_vpc_access_connector.this[0].network == var.network_id,
      google_sql_database_instance.writer.settings[0].ip_configuration[0].private_network == var.network_id,
      google_sql_database_instance.reader.settings[0].ip_configuration[0].private_network == var.network_id,
      google_redis_instance.this[0].authorized_network == var.network_id,
    ])
    error_message = "The connector, Cloud SQL and Memorystore must attach to the supplied network while the peering is created."
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
    condition     = google_sql_database_instance.writer.name == "tenant-test-betterlite"
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

run "region_reaches_every_regional_resource" {
  command = plan

  variables {
    region = "europe-west1"
  }

  assert {
    condition = alltrue([
      google_compute_subnetwork.this[0].region == "europe-west1",
      google_compute_subnetwork.this[0].name == "tenant-test-betterlite-europe-west1",
      google_vpc_access_connector.this[0].region == "europe-west1",
      google_sql_database_instance.writer.region == "europe-west1",
      google_sql_database_instance.reader.region == "europe-west1",
      google_redis_instance.this[0].region == "europe-west1",
    ])
    error_message = "A caller's region must reach the subnet, the connector, Cloud SQL and Memorystore."
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

run "connector_name_fits_the_25_character_limit" {
  command = plan

  variables {
    tenant = "acme"
    env    = "stage"
  }

  assert {
    condition     = google_vpc_access_connector.this[0].name == "acme-stage-betterlit-conn" && length(google_vpc_access_connector.this[0].name) <= 25
    error_message = "The connector name must be cut so it stays within 25 characters."
  }
}

run "connector_name_fits_for_the_longest_tenant_and_env" {
  command = plan

  variables {
    tenant = "abcdefghijklmnopqrstu"
    env    = "abcdefghi"
  }

  assert {
    condition     = length(google_vpc_access_connector.this[0].name) <= 25 && google_vpc_access_connector.this[0].name == "abcdefghijklmnopqrst-conn"
    error_message = "The longest tenant and env must still give a connector name within 25 characters."
  }
}

run "connector_name_drops_a_hyphen_at_the_cut" {
  command = plan

  variables {
    tenant = "abcdefghijklmnopqrs"
    env    = "dev"
  }

  assert {
    condition     = google_vpc_access_connector.this[0].name == "abcdefghijklmnopqrs-conn"
    error_message = "A hyphen the 20-character cut lands on must not be kept in the connector name."
  }
}
