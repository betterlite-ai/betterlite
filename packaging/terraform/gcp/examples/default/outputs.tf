output "lb_url" {
  description = "Proxy URL through the load balancer: https:// with the first lb_domains entry, http:// with the load balancer IP when plaintext is allowed."
  value       = module.betterlite.lb_url
}

output "lb_ip" {
  description = "Global anycast IP of the load balancer."
  value       = module.betterlite.lb_ip
}

output "service_url" {
  description = "Default Cloud Run URL of the betterlite service, which bypasses the load balancer."
  value       = module.betterlite.service_url
}

output "migration_job_name" {
  description = "Name of the Cloud Run migration job, for a manual re-run with gcloud run jobs execute."
  value       = module.betterlite.migration_job_name
}

output "cloud_run_service_name" {
  description = "Name of the betterlite Cloud Run service."
  value       = module.betterlite.cloud_run_service_name
}
