locals {
  # nonsensitive() wraps only the emptiness test, so a supplied secret stays redacted in plans
  license_set     = nonsensitive(var.litellm_license != "")
  ui_password_set = nonsensitive(var.ui_password != "")
}

resource "google_secret_manager_secret" "master_key" {
  secret_id = "${local.name}-master-key"
  labels    = local.labels

  replication {
    auto {}
  }
}

resource "google_secret_manager_secret_version" "master_key" {
  secret      = google_secret_manager_secret.master_key.id
  secret_data = var.litellm_master_key
}

resource "google_secret_manager_secret" "license" {
  count = local.license_set ? 1 : 0

  secret_id = "${local.name}-license"
  labels    = local.labels

  replication {
    auto {}
  }
}

resource "google_secret_manager_secret_version" "license" {
  count = local.license_set ? 1 : 0

  secret      = google_secret_manager_secret.license[0].id
  secret_data = var.litellm_license
}

resource "google_secret_manager_secret" "ui_password" {
  count = local.ui_password_set ? 1 : 0

  secret_id = "${local.name}-ui-password"
  labels    = local.labels

  replication {
    auto {}
  }
}

resource "google_secret_manager_secret_version" "ui_password" {
  count = local.ui_password_set ? 1 : 0

  secret      = google_secret_manager_secret.ui_password[0].id
  secret_data = var.ui_password
}

resource "google_secret_manager_secret_iam_member" "master_key" {
  secret_id = google_secret_manager_secret.master_key.id
  role      = "roles/secretmanager.secretAccessor"
  member    = "serviceAccount:${google_service_account.runtime.email}"
}

resource "google_secret_manager_secret_iam_member" "db_password" {
  secret_id = google_secret_manager_secret.db_password.id
  role      = "roles/secretmanager.secretAccessor"
  member    = "serviceAccount:${google_service_account.runtime.email}"
}

resource "google_secret_manager_secret_iam_member" "license" {
  count = local.license_set ? 1 : 0

  secret_id = google_secret_manager_secret.license[0].id
  role      = "roles/secretmanager.secretAccessor"
  member    = "serviceAccount:${google_service_account.runtime.email}"
}

resource "google_secret_manager_secret_iam_member" "ui_password" {
  count = local.ui_password_set ? 1 : 0

  secret_id = google_secret_manager_secret.ui_password[0].id
  role      = "roles/secretmanager.secretAccessor"
  member    = "serviceAccount:${google_service_account.runtime.email}"
}

# two environment names may point at one secret, and GCP wants one binding per secret
resource "google_secret_manager_secret_iam_member" "extra_secrets" {
  for_each = toset(values(var.extra_secrets))

  secret_id = each.value
  role      = "roles/secretmanager.secretAccessor"
  member    = "serviceAccount:${google_service_account.runtime.email}"
}
