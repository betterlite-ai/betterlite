resource "random_password" "db_password" {
  length      = 32
  special     = false
  min_lower   = 4
  min_upper   = 4
  min_numeric = 4
}

resource "google_sql_database_instance" "writer" {
  name             = local.name
  region           = var.region
  database_version = var.db_version

  depends_on = [google_service_networking_connection.psa]

  settings {
    edition           = var.db_edition
    tier              = var.db_tier
    availability_type = "REGIONAL"
    disk_size         = 20
    disk_autoresize   = true

    user_labels = local.labels

    backup_configuration {
      enabled                        = true
      point_in_time_recovery_enabled = true
      start_time                     = "07:00"
    }

    ip_configuration {
      ipv4_enabled    = false
      private_network = local.network_self_link
    }

    insights_config {
      query_insights_enabled  = true
      record_application_tags = true
      record_client_address   = true
    }
  }

  deletion_protection = var.cloudsql_deletion_protection

  lifecycle {
    # autoresize only grows the disk, so a plan that reads it back would otherwise replace the instance
    ignore_changes = [settings[0].disk_size]

    precondition {
      condition     = var.create_psa_connection || var.network_id != ""
      error_message = "create_psa_connection must be true unless network_id names an existing VPC that already has private services access."
    }
  }
}

resource "google_sql_database_instance" "reader" {
  name                 = "${local.name}-reader"
  region               = var.region
  database_version     = var.db_version
  master_instance_name = google_sql_database_instance.writer.name

  depends_on = [google_sql_database_instance.writer]

  settings {
    edition           = var.db_edition
    tier              = var.db_tier
    availability_type = "ZONAL"
    disk_autoresize   = true

    user_labels = local.labels

    ip_configuration {
      ipv4_enabled    = false
      private_network = local.network_self_link
    }
  }

  deletion_protection = var.cloudsql_deletion_protection

  lifecycle {
    # the replica grows its own disk independently, with the same replace-on-shrink hazard as the writer
    ignore_changes = [settings[0].disk_size]
  }
}

resource "google_sql_database" "this" {
  name     = var.db_name
  instance = google_sql_database_instance.writer.name

  deletion_policy = "ABANDON"
}

resource "google_sql_user" "app" {
  name     = var.db_username
  instance = google_sql_database_instance.writer.name
  password = random_password.db_password.result

  deletion_policy = "ABANDON"
}

resource "google_secret_manager_secret" "db_password" {
  secret_id = "${local.name}-db-password"
  labels    = local.labels

  replication {
    auto {}
  }
}

resource "google_secret_manager_secret_version" "db_password" {
  secret      = google_secret_manager_secret.db_password.id
  secret_data = random_password.db_password.result
}
