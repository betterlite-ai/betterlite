resource "terraform_data" "migration" {
  # GCP needs no separate IAM bootstrap step, so the migration is the only post-store work here. It runs
  # gcloud on the machine that applies, and a non-zero exit fails the apply before the service is created.
  count = var.create_runtime ? 1 : 0

  triggers_replace = {
    job_id    = google_cloud_run_v2_job.migrate[0].id
    job_image = var.image
  }

  provisioner "local-exec" {
    interpreter = ["bash", "-c"]
    environment = {
      JOB     = google_cloud_run_v2_job.migrate[0].name
      REGION  = var.region
      PROJECT = var.project_id
    }
    command = <<-EOT
      set -euo pipefail
      gcloud run jobs execute "$JOB" --region "$REGION" --project "$PROJECT" --wait
    EOT
  }

  depends_on = [
    google_cloud_run_v2_job.migrate,
    google_sql_user.app,
  ]
}
