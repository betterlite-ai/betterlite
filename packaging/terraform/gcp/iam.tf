resource "google_service_account" "runtime" {
  # GCP caps an account id at 30 characters, so the shared name is cut to 22 before the -runtime suffix
  account_id   = "${trimsuffix(substr(local.name, 0, 22), "-")}-runtime"
  display_name = "betterlite runtime"
  labels       = local.labels
}

resource "google_project_iam_member" "runtime_cloudsql" {
  project = var.project_id
  role    = "roles/cloudsql.client"
  member  = "serviceAccount:${google_service_account.runtime.email}"
}

resource "google_project_iam_member" "runtime_log_writer" {
  project = var.project_id
  role    = "roles/logging.logWriter"
  member  = "serviceAccount:${google_service_account.runtime.email}"
}
