locals {
  # LiteLLM creates the config bucket only for a runtime it creates too
  proxy_config_enabled = var.create_runtime && length(keys(var.proxy_config)) > 0
  proxy_config_hash    = md5(yamlencode(var.proxy_config))
  bucket_location      = var.bucket_location != "" ? var.bucket_location : var.region
}

resource "random_id" "bucket_suffix" {
  byte_length = 4
}

resource "google_storage_bucket" "data" {
  name                        = "${var.project_id}-${local.name}-${random_id.bucket_suffix.hex}"
  location                    = local.bucket_location
  uniform_bucket_level_access = true
  force_destroy               = var.bucket_force_destroy
  public_access_prevention    = "enforced"
  labels                      = local.labels

  versioning {
    enabled = true
  }

  dynamic "lifecycle_rule" {
    for_each = var.data_retention_days > 0 ? [var.data_retention_days] : []

    content {
      condition {
        age = lifecycle_rule.value
      }

      action {
        type = "Delete"
      }
    }
  }
}

resource "google_storage_bucket_iam_member" "data_runtime" {
  bucket = google_storage_bucket.data.name
  role   = "roles/storage.objectAdmin"
  member = "serviceAccount:${google_service_account.runtime.email}"
}

resource "google_storage_bucket" "config" {
  count = local.proxy_config_enabled ? 1 : 0

  name                        = "${var.project_id}-${local.name}-config-${random_id.bucket_suffix.hex}"
  location                    = local.bucket_location
  uniform_bucket_level_access = true
  force_destroy               = var.bucket_force_destroy
  public_access_prevention    = "enforced"
  labels                      = local.labels

  versioning {
    enabled = true
  }
}

resource "google_storage_bucket_object" "config" {
  count = local.proxy_config_enabled ? 1 : 0

  name         = "config.yaml"
  bucket       = google_storage_bucket.config[0].name
  content      = yamlencode(var.proxy_config)
  content_type = "application/yaml"
}

resource "google_storage_bucket_iam_member" "config_runtime" {
  count = local.proxy_config_enabled ? 1 : 0

  bucket = google_storage_bucket.config[0].name
  role   = "roles/storage.objectViewer"
  member = "serviceAccount:${google_service_account.runtime.email}"
}
