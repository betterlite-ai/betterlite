output "lb_ip" {
  description = "Global anycast IP of the external load balancer, or null when create_runtime is false."
  value       = var.create_runtime ? one(google_compute_global_address.lb[*].address) : null
}

output "lb_url" {
  description = "Proxy URL: https:// with the first lb_domains entry, http:// with the load balancer IP when plaintext is allowed, or null without the runtime."
  value       = var.create_runtime ? (local.tls_enabled ? "https://${var.lb_domains[0]}" : "http://${one(google_compute_global_address.lb[*].address)}") : null
}

output "service_url" {
  description = "Default Cloud Run URL of the betterlite service, or null when create_runtime is false."
  value       = var.create_runtime ? one(google_cloud_run_v2_service.betterlite[*].uri) : null
}

output "cloud_run_service_name" {
  description = "Name of the betterlite Cloud Run service, or null when create_runtime is false."
  value       = var.create_runtime ? one(google_cloud_run_v2_service.betterlite[*].name) : null
}

output "runtime_service_account" {
  description = "Email of the service account the Cloud Run service and the migration job run as."
  value       = google_service_account.runtime.email
}

output "writer_private_ip" {
  description = "Private IP of the Cloud SQL writer."
  value       = google_sql_database_instance.writer.private_ip_address
}

output "reader_private_ip" {
  description = "Private IP of the Cloud SQL read replica."
  value       = google_sql_database_instance.reader.private_ip_address
}

output "redis_host" {
  description = "Memorystore Redis host, or null when create_redis is false."
  value       = local.redis_enabled ? google_redis_instance.this[0].host : null
}

output "data_bucket" {
  description = "GCS bucket betterlite keeps its data in, exported to the service as GCS_BUCKET_NAME."
  value       = google_storage_bucket.data.name
}

output "config_bucket" {
  description = "GCS bucket holding config.yaml, or null when no proxy_config is set."
  value       = local.proxy_config_enabled ? google_storage_bucket.config[0].name : null
}

output "master_key_secret_id" {
  description = "Secret Manager resource id holding LITELLM_MASTER_KEY."
  value       = google_secret_manager_secret.master_key.secret_id
}

output "db_password_secret_id" {
  description = "Secret Manager resource id holding the Cloud SQL application user's password."
  value       = google_secret_manager_secret.db_password.secret_id
}

output "network_id" {
  description = "Resource id of the VPC network the stores and the runtime attach to."
  value       = local.network_self_link
}

output "migration_job_name" {
  description = "Name of the Cloud Run migration job, or null when create_runtime is false."
  value       = var.create_runtime ? one(google_cloud_run_v2_job.migrate[*].name) : null
}
